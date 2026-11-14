//! Runs tests/difftest.sh (ferrooxide vs. Icarus Verilog) when iverilog is installed.

#[test]
fn difftest_against_icarus() {
    if std::process::Command::new("iverilog").arg("-V").output().is_err() {
        eprintln!("iverilog not found, skipping");
        return;
    }
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let st = std::process::Command::new("cargo")
        .args(["build", "--release", "-q"])
        .current_dir(&root)
        .status()
        .unwrap();
    assert!(st.success());
    let out = std::process::Command::new(root.join("tests/difftest.sh")).current_dir(&root).output().unwrap();
    print!("{}", String::from_utf8_lossy(&out.stdout));
    assert!(out.status.success(), "difftest failed");
}

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
    let mut child = std::process::Command::new(root.join("target/release/ferrooxide"))
        .args(["-q", "-i", "tests/cdsp/counter.v"])
        .current_dir(&root)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(b"watch tb.x\nrun 30ns\nrestart\ntime\nrun 30ns\nquit\n").unwrap();
    let out = String::from_utf8(child.wait_with_output().unwrap().stdout).unwrap();
    let lines: Vec<&str> = out.lines().collect();
    assert!(lines.iter().filter(|l| **l == "22000: tb.x = 9").count() == 2, "{out}");
    assert!(lines.iter().any(|l| l.starts_with("restarted: loaded 1 files")), "{out}");
    assert!(lines.contains(&"time 0"), "{out}");
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

/// --surfer: a stand-in viewer (tests/cdsp/fake_surfer.py) connects over WCP and CDSP; the
/// initial signals are added after a "no waveform" retry, $finish gets a marker, and
/// ferrooxide exits with the viewer.
#[test]
fn surfer_pairing() {
    if std::process::Command::new("python3").arg("-V").output().is_err() {
        eprintln!("python3 not found, skipping");
        return;
    }
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let st = std::process::Command::new("cargo").args(["build", "--release", "-q"]).current_dir(&root).status().unwrap();
    assert!(st.success());
    let log = std::env::temp_dir().join(format!("fx_fake_surfer_{}.log", std::process::id()));
    let out = std::process::Command::new(root.join("target/release/ferrooxide"))
        .args(["-q", "--surfer", "--trace", r"u\.q$", "--trace", "x$", "tests/cdsp/counter.v"])
        .env("FX_SURFER", "tests/cdsp/fake_surfer.py")
        .env("FAKE_SURFER_LOG", &log)
        .current_dir(&root)
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let got = std::fs::read_to_string(&log).unwrap();
    let _ = std::fs::remove_file(&log);
    assert_eq!(
        got,
        "wcp add_items ['tb.u.q', 'tb.x'] (no waveform yet)\n\
         wcp add_items ['tb.u.q', 'tb.x']\n\
         cdsp simulation_finished\n\
         wcp add_markers [{'move_focus': False, 'name': '$finish', 'time': 1022000000}]\n"
    );
}

/// `-i --surfer`: console commands move the simulation; the viewer gets simulation_paused and
/// the cursor follows (breakpoint stop, run to a time); quit ends the session.
#[test]
fn surfer_console() {
    use std::io::Write;
    if std::process::Command::new("python3").arg("-V").output().is_err() {
        eprintln!("python3 not found, skipping");
        return;
    }
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let st = std::process::Command::new("cargo").args(["build", "--release", "-q"]).current_dir(&root).status().unwrap();
    assert!(st.success());
    let log = std::env::temp_dir().join(format!("fx_fake_surfer_console_{}.log", std::process::id()));
    let _ = std::fs::remove_file(&log);
    let mut child = std::process::Command::new(root.join("target/release/ferrooxide"))
        .args(["-q", "-i", "--surfer", "tests/cdsp/counter.v"])
        .env("FX_SURFER", "tests/cdsp/fake_surfer.py")
        .env("FAKE_SURFER_MODE", "watch")
        .env("FAKE_SURFER_LOG", &log)
        .current_dir(&root)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let read = || std::fs::read_to_string(&log).unwrap_or_default();
    let wait = |what: &str| {
        for _ in 0..100 {
            if read().contains(what) {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        panic!("no '{what}' from the viewer: {}", read());
    };
    wait("ready");
    child.stdin.take().unwrap().write_all(b"break x 4'd9\nrun\nprint x d\nrun 100ns\nquit\n").unwrap();
    let out = String::from_utf8(child.wait_with_output().unwrap().stdout).unwrap();
    assert!(out.contains("break: tb.x = 9 at time 22000") && out.contains("stopped at time 100000"), "{out}");
    wait("closed");
    let got = read();
    let _ = std::fs::remove_file(&log);
    // (the two connections are read in no fixed order: compare each on its own)
    let only = |p: &str| got.lines().filter(|l| l.starts_with(p)).collect::<Vec<_>>().join("\n");
    assert_eq!(only("cdsp"), "cdsp simulation_paused 0.000000022000000\ncdsp simulation_paused 0.000000100000000", "{got}");
    assert_eq!(only("wcp"), "wcp set_cursor 22000000\nwcp set_cursor 100000000", "{got}");
    assert!(got.starts_with("ready\n") && got.ends_with("closed\n"), "{got}");
}

/// Features the reference simulator lacks: each tests/sv/X.v must print tests/sv/X.expected,
/// in all three modes (default, --no-jit, --no-fuse).
#[test]
fn sv_expected_outputs() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let st = std::process::Command::new("cargo").args(["build", "--release", "-q"]).current_dir(&root).status().unwrap();
    assert!(st.success());
    let mut files: Vec<_> = std::fs::read_dir(root.join("tests/sv"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "v"))
        .collect();
    files.sort();
    assert!(!files.is_empty());
    for f in files {
        let expected = std::fs::read_to_string(f.with_extension("expected")).unwrap();
        for mode in [None, Some("--no-jit"), Some("--no-fuse")] {
            let mut cmd = std::process::Command::new(root.join("target/release/ferrooxide"));
            // (a relative path: it appears in the $finish message)
            cmd.arg("-q").args(mode).arg(std::path::Path::new("tests/sv").join(f.file_name().unwrap())).current_dir(&root);
            let out = cmd.output().unwrap();
            assert_eq!(String::from_utf8_lossy(&out.stdout), expected, "{} {:?}", f.display(), mode);
        }
    }
}
