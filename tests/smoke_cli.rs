use std::process::Command;

#[test]
fn help_exits_cleanly() {
    let output = Command::new(env!("CARGO_BIN_EXE_venturi"))
        .arg("--help")
        .output()
        .expect("failed to run --help");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Linux audio mixer for PipeWire"));
}
