use std::process::Command;

#[test]
fn help_exposes_phase_1a_commands() {
    let output = Command::new(env!("CARGO_BIN_EXE_devmap"))
        .arg("--help")
        .output()
        .expect("run devmap --help");

    assert!(output.status.success());

    let stdout = String::from_utf8(output.stdout).expect("help is UTF-8");
    assert!(stdout.contains("init"), "missing init command: {stdout}");
    assert!(
        stdout.contains("common-ground"),
        "missing common-ground command: {stdout}"
    );
    assert!(stdout.contains("status"), "missing status command: {stdout}");
}

