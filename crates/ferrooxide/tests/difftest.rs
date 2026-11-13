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
