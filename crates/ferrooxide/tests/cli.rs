//! Runs the command-line tool: lint, warnings, interactive mode, MCP and CDSP sessions, and
//! the own suite (tests/suite.sh).

#[test]
fn lint_sensitivity_lists() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let st = std::process::Command::new("cargo").args(["build", "--release", "-q"]).current_dir(&root).status().unwrap();
    assert!(st.success());
    let out = std::process::Command::new(root.join("target/release/ferrooxide"))
        .args(["--lint", "tests/lint/sens.v"])
        .current_dir(&root)
        .output()
        .unwrap();
    let expected = std::fs::read_to_string(root.join("tests/lint/sens.expected")).unwrap();
    assert_eq!(String::from_utf8_lossy(&out.stdout), expected);
    assert_eq!(out.status.code(), Some(1));
}

/// Lint: names that are SystemVerilog keywords, once per name and module.
#[test]
fn lint_sv_keywords() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let st = std::process::Command::new("cargo").args(["build", "--release", "-q"]).current_dir(&root).status().unwrap();
    assert!(st.success());
    let out = std::process::Command::new(root.join("target/release/ferrooxide"))
        .args(["--lint", "-g2005", "tests/lint/keywords.v"])
        .current_dir(&root)
        .output()
        .unwrap();
    let expected = std::fs::read_to_string(root.join("tests/lint/keywords.expected")).unwrap();
    assert_eq!(String::from_utf8_lossy(&out.stdout), expected);
    assert_eq!(out.status.code(), Some(1));
}

/// Elaboration warnings (tests/warnings/*.v, options in a `// flags:` line): stderr without the
/// summary line, then stdout, equal to `<name>.expected`.
#[test]
fn elab_warnings() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let st = std::process::Command::new("cargo").args(["build", "--release", "-q"]).current_dir(&root).status().unwrap();
    assert!(st.success());
    let dir = root.join("tests/warnings");
    let mut n = 0;
    for e in std::fs::read_dir(&dir).unwrap() {
        let p = e.unwrap().path();
        if p.extension().is_none_or(|x| x != "v") {
            continue;
        }
        let name = p.file_name().unwrap().to_str().unwrap().to_string();
        let src = std::fs::read_to_string(&p).unwrap();
        let flags: Vec<&str> = src.lines().find_map(|l| l.strip_prefix("// flags:")).map(|f| f.split_whitespace().collect()).unwrap_or_default();
        let out = std::process::Command::new(root.join("target/release/ferrooxide"))
            .args(&flags)
            .arg(&name)
            .current_dir(&dir)
            .output()
            .unwrap();
        let err = String::from_utf8_lossy(&out.stderr);
        let mut got: String = err.lines().filter(|l| !l.starts_with("ferrooxide:")).map(|l| format!("{l}\n")).collect();
        got.push_str(&String::from_utf8_lossy(&out.stdout));
        let expected = std::fs::read_to_string(p.with_extension("expected")).unwrap();
        assert_eq!(got, expected, "{name}");
        n += 1;
    }
    assert!(n > 0);
}

/// Interactive mode: a command script on stdin (run, step, print, force, release, set, list).
#[test]
fn interactive_commands() {
    use std::io::Write;
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let st = std::process::Command::new("cargo").args(["build", "--release", "-q"]).current_dir(&root).status().unwrap();
    assert!(st.success());
    let script = std::fs::read(root.join("tests/interactive/counter.cmd")).unwrap();
    let mut child = std::process::Command::new(root.join("target/release/ferrooxide"))
        .args(["-i", "tests/interactive/counter.v"])
        .current_dir(&root)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(&script).unwrap();
    let out = child.wait_with_output().unwrap();
    let expected = std::fs::read_to_string(root.join("tests/interactive/counter.expected")).unwrap();
    assert_eq!(String::from_utf8_lossy(&out.stdout), expected);
}

/// `restart` in the interactive mode: the design starts over at time 0, watches are kept.
#[test]
fn interactive_restart() {
    use std::io::Write;
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let st = std::process::Command::new("cargo").args(["build", "--release", "-q"]).current_dir(&root).status().unwrap();
    assert!(st.success());
    let vcd = std::env::temp_dir().join(format!("fx_restart_{}.vcd", std::process::id()));
    let mut child = std::process::Command::new(root.join("target/release/ferrooxide"))
        .args(["-q", "-i", "--vcd", vcd.to_str().unwrap(), "tests/cdsp/counter.v"])
        .current_dir(&root)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(b"watch tb.x\nrun 50ns\nrestart\ntime\nrun 30ns\nquit\n").unwrap();
    let out = String::from_utf8(child.wait_with_output().unwrap().stdout).unwrap();
    let lines: Vec<&str> = out.lines().collect();
    assert!(lines.iter().filter(|l| **l == "22000: tb.x = 9").count() == 2, "{out}");
    assert!(lines.iter().any(|l| l.starts_with("restarted: loaded 1 files")), "{out}");
    assert!(lines.contains(&"time 0"), "{out}");
    // the VCD holds the second run only (written anew from time 0)
    let v = std::fs::read_to_string(&vcd).unwrap();
    let _ = std::fs::remove_file(&vcd);
    assert_eq!(v.matches("$enddefinitions").count(), 1, "{v}");
    assert_eq!(v.lines().filter(|l| l.starts_with('#')).last(), Some("#30000"), "{v}");
}

/// MCP server: a scripted JSON-RPC session (initialize, tools/list, load, watch, break, run,
/// force, values, lint, errors).
#[test]
fn mcp_session() {
    use std::io::Write;
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let st = std::process::Command::new("cargo").args(["build", "--release", "-q"]).current_dir(&root).status().unwrap();
    assert!(st.success());
    let script = std::fs::read(root.join("tests/mcp/session.jsonl")).unwrap();
    let mut child = std::process::Command::new(root.join("target/release/ferrooxide"))
        .arg("--mcp")
        .current_dir(&root)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(&script).unwrap();
    let out = String::from_utf8(child.wait_with_output().unwrap().stdout).unwrap();
    let lines: Vec<&str> = out.lines().collect();
    // one answer per request (the notification gets none)
    assert_eq!(lines.len(), 14, "{out}");
    let by_id = |id: u32| lines.iter().find(|l| l.contains(&format!("\"id\":{id},"))).copied().unwrap_or("");
    assert!(by_id(1).contains("\"serverInfo\""));
    assert!(by_id(2).contains("\"load_design\"") && by_id(2).contains("\"add_breakpoint\""));
    assert!(by_id(3).contains("\"isError\":true"));
    assert!(by_id(7).contains("break: top.q = 03 at time 35"));
    assert!(by_id(9).contains("top.rst = 1'b1") && by_id(9).contains("no signal 'nope'"));
    assert!(by_id(12).contains("$finish called at 1000"));
    assert!(by_id(13).contains("incomplete sensitivity list"));
    assert!(by_id(14).contains("-32601"));
}

/// Debug server (CDSP): greeting, hierarchy, references, run to a time, history query with
/// x/z and a multi-word value, run to $finish, errors.
#[test]
fn cdsp_session() {
    use std::io::{BufRead, Read, Write};
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let st = std::process::Command::new("cargo").args(["build", "--release", "-q"]).current_dir(&root).status().unwrap();
    assert!(st.success());
    let mut child = std::process::Command::new(root.join("target/release/ferrooxide"))
        .args(["-q", "--cdsp", "0", "tests/cdsp/counter.v"])
        .current_dir(&root)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let mut err = std::io::BufReader::new(child.stderr.take().unwrap());
    let mut line = String::new();
    err.read_line(&mut line).unwrap();
    let addr = line.split_whitespace().find(|w| w.starts_with("127.0.0.1:")).unwrap().trim_end_matches(';').to_string();
    let mut s = std::net::TcpStream::connect(&addr).unwrap();
    let mut buf = Vec::new();
    let mut ask = |m: &str, answers: usize| -> String {
        s.write_all(m.as_bytes()).unwrap();
        s.write_all(b"\0").unwrap();
        let mut got = Vec::new();
        while got.len() < answers {
            if let Some(i) = buf.iter().position(|&c| c == 0) {
                got.push(String::from_utf8(buf[..i].to_vec()).unwrap());
                buf.drain(..=i);
                continue;
            }
            let mut chunk = [0u8; 4096];
            let n = s.read(&mut chunk).unwrap();
            assert!(n > 0, "connection closed");
            buf.extend_from_slice(&chunk[..n]);
        }
        got.join("\n")
    };
    assert!(ask(r#"{"type":"greeting","version":0}"#, 1).contains(r#""item_values_encoding":["base64(u32)"]"#));
    assert!(ask(r#"{"type":"command","command":"list_scopes","scope":null}"#, 1).contains(r#""tb u":{"#));
    let items = ask(r#"{"type":"command","command":"list_items","scope":"tb u"}"#, 1);
    assert!(items.contains(r#""tb u q":{"#) && items.contains(r#""width":40"#) && !items.contains(r#""tb x""#));
    ask(r#"{"type":"command","command":"reference_items","reference":"r","items":[["tb x"],["tb u q"]]}"#, 1);
    let run = ask(r#"{"type":"command","command":"run_simulation","until_time":"0.000000030000000","until_diagnostics":[],"sample_item_values":true}"#, 2);
    assert!(run.contains(r#""event":"simulation_paused""#) && run.contains(r#""time":"0.000000030000000""#), "{run}");
    let q = ask(r#"{"type":"command","command":"query_interval","interval":["0.000000020000000","0.000000030000000"],"collapse":true,"items":"r","item_values_encoding":"base64(u32)","diagnostics":false}"#, 1);
    // x = 4'bx01z (x -> 1, z -> 0) and q = 2 * 40'h1_0000_0001 at 20 ns; x = 9 at 22 ns; q
    // incremented at 25 ns
    assert!(q.contains(r#"{"item_values":"CgAAAAIAAAACAAAA","time":"0.000000020000000"}"#), "{q}");
    assert!(q.contains(r#"{"item_values":"CQAAAAIAAAACAAAA","time":"0.000000022000000"}"#), "{q}");
    assert!(q.contains(r#"{"item_values":"CQAAAAMAAAADAAAA","time":"0.000000025000000"}"#), "{q}");
    assert_eq!(q.matches(r#""time""#).count(), 3, "{q}");
    // a real is a 64-bit item holding the IEEE double (1.5)
    ask(r#"{"type":"command","command":"reference_items","reference":"re","items":[["tb r"]]}"#, 1);
    let q = ask(r#"{"type":"command","command":"query_interval","interval":["0.000000000000000","0.000000000000000"],"collapse":true,"items":"re","item_values_encoding":"base64(u32)","diagnostics":false}"#, 1);
    assert!(q.contains(r#""item_values":"AAAAAAAA+D8=""#), "{q}");
    let fin = ask(r#"{"type":"command","command":"run_simulation","until_time":null,"until_diagnostics":[],"sample_item_values":true}"#, 2);
    assert!(fin.contains(r#""event":"simulation_finished""#) && fin.contains(r#""time":"0.000001022000000""#), "{fin}");
    assert!(ask(r#"{"type":"command","command":"run_simulation","until_time":null,"until_diagnostics":[],"sample_item_values":true}"#, 1).contains(r#""error":"invalid_status""#));
    assert!(ask(r#"{"type":"command","command":"nope"}"#, 1).contains(r#""error":"invalid_command""#));
    child.kill().unwrap();
    child.wait().unwrap();
}

/// `-i --cdsp`: console commands move the simulation and the client gets simulation_paused
/// (breakpoint stop, run to a time); quit ends the session.
#[test]
fn cdsp_console() {
    use std::io::{BufRead, Read, Write};
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let st = std::process::Command::new("cargo").args(["build", "--release", "-q"]).current_dir(&root).status().unwrap();
    assert!(st.success());
    let mut child = std::process::Command::new(root.join("target/release/ferrooxide"))
        .args(["-q", "-i", "--cdsp", "0", "tests/cdsp/counter.v"])
        .current_dir(&root)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let mut err = std::io::BufReader::new(child.stderr.take().unwrap());
    let mut line = String::new();
    err.read_line(&mut line).unwrap();
    let addr = line.split_whitespace().find(|w| w.starts_with("127.0.0.1:")).unwrap().to_string();
    let mut s = std::net::TcpStream::connect(&addr).unwrap();
    let mut buf = Vec::new();
    // the next `n` messages from the server
    let mut next = |s: &mut std::net::TcpStream, n: usize| -> String {
        let mut got = Vec::new();
        while got.len() < n {
            if let Some(i) = buf.iter().position(|&c| c == 0) {
                got.push(String::from_utf8(buf[..i].to_vec()).unwrap());
                buf.drain(..=i);
                continue;
            }
            let mut chunk = [0u8; 4096];
            let k = s.read(&mut chunk).unwrap();
            assert!(k > 0, "connection closed");
            buf.extend_from_slice(&chunk[..k]);
        }
        got.join("\n")
    };
    s.write_all(b"{\"type\":\"greeting\",\"version\":0}\0").unwrap();
    assert!(next(&mut s, 1).contains(r#""type":"greeting""#));
    let mut stdin = child.stdin.take().unwrap();
    stdin.write_all(b"break x 4'd9\nrun\n").unwrap();
    let ev = next(&mut s, 1);
    assert!(ev.contains(r#""event":"simulation_paused""#) && ev.contains(r#""time":"0.000000022000000""#), "{ev}");
    stdin.write_all(b"print x d\nrun 100ns\n").unwrap();
    let ev = next(&mut s, 1);
    assert!(ev.contains(r#""event":"simulation_paused""#) && ev.contains(r#""time":"0.000000100000000""#), "{ev}");
    stdin.write_all(b"quit\n").unwrap();
    drop(stdin);
    let out = String::from_utf8(child.wait_with_output().unwrap().stdout).unwrap();
    assert!(out.contains("break: tb.x = 9 at time 22000") && out.contains("stopped at time 100000"), "{out}");
}

/// Our own suite (tests/suite.sh): self-checking tests and tests with expected outputs, in
/// all three modes; needs no other simulator.
#[test]
fn own_suite() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let st = std::process::Command::new("cargo").args(["build", "--release", "-q"]).current_dir(&root).status().unwrap();
    assert!(st.success());
    let out = std::process::Command::new(root.join("tests/suite.sh")).current_dir(&root).output().unwrap();
    print!("{}", String::from_utf8_lossy(&out.stdout));
    assert!(out.status.success(), "suite failed");
}
