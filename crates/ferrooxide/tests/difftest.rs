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
