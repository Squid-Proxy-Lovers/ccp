// SPDX-License-Identifier: AGPL-3.0-or-later
use std::process::Command;

#[test]
fn workspace_help_omits_unrelated_native_examples() {
    for command in [
        vec!["workspace", "trust", "--help"],
        vec!["workspace", "status", "mutate", "--help"],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_client"))
            .args(command)
            .output()
            .unwrap();
        assert!(output.status.success());
        let text = String::from_utf8(output.stdout).unwrap();
        assert!(!text.contains("Input formats:"));
        assert!(text.contains("--profile"));
    }
}
#[test]
fn workspace_errors_keep_usage_and_exit_two_without_native_examples() {
    let output = Command::new(env!("CARGO_BIN_EXE_client"))
        .args(["workspace", "not-a-command"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    let text = String::from_utf8(output.stderr).unwrap();
    assert!(text.contains("unrecognized subcommand"));
    assert!(!text.contains("Input formats:"));
}
#[test]
fn root_help_keeps_native_examples_and_success_exit() {
    let output = Command::new(env!("CARGO_BIN_EXE_client"))
        .arg("--help")
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(
        String::from_utf8(output.stdout)
            .unwrap()
            .contains("Input formats:")
    );
}
