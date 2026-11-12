// SPDX-License-Identifier: AGPL-3.0-or-later
use std::{fs, process::Command};
use uuid::Uuid;

#[test]
fn client_home_override_and_home_resolution() {
    let root = std::env::temp_dir().join(format!("ccp-home-{}", Uuid::new_v4()));
    let home = root.join("home");
    let enrollment = home.join(".ccp-client/enrollments/broken");
    fs::create_dir_all(&enrollment).unwrap();
    fs::write(enrollment.join("metadata.json"), "invalid JSON").unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_client"));
    command.arg("sessions").env_remove("CCP_CLIENT_HOME");
    #[cfg(windows)]
    command.env_remove("HOME").env("USERPROFILE", &home);
    #[cfg(not(windows))]
    command.env("HOME", &home);
    let output = command.output().unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("metadata.json"));
    let output = command
        .env("CCP_CLIENT_HOME", root.join("override"))
        .output()
        .unwrap();
    assert!(output.status.success());
    #[cfg(windows)]
    {
        let output = command
            .env_remove("CCP_CLIENT_HOME")
            .env("HOME", root.join("other-home"))
            .output()
            .unwrap();
        assert!(output.status.success());
    }
    fs::remove_dir_all(root).unwrap();
}
