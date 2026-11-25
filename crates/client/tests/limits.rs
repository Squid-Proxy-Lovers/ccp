// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

//! `--limit` is validated by the argument parser, before any profile is opened.

use std::process::{Command, Output};

const WORKSPACE: &str = "11111111-1111-4111-8111-111111111111";
const NODE: &str = "22222222-2222-4222-8222-222222222222";

fn missing_profile() -> String {
    std::env::temp_dir()
        .join(format!("ccp-limit-test-missing-{}", std::process::id()))
        .join("profile")
        .display()
        .to_string()
}

/// Every listing command, without its `--limit`, and the largest accepted page.
fn listings() -> Vec<(Vec<String>, u32)> {
    let profile = missing_profile();
    let content = |command: &[&str]| {
        let mut args = vec!["workspace", "content", "--profile", &profile];
        args.extend_from_slice(command);
        args.extend_from_slice(&["--workspace-id", WORKSPACE]);
        args.into_iter().map(str::to_owned).collect::<Vec<_>>()
    };
    vec![
        (content(&["shelves"]), 32),
        (content(&["archived-shelves"]), 32),
        (content(&["books", "--shelf-id", NODE]), 32),
        (content(&["entries", "--book-id", NODE]), 32),
        (content(&["history", "--entry-id", NODE]), 32),
        (content(&["native-history", "--entry-id", NODE]), 32),
        (
            [
                "workspace",
                "status",
                "--profile",
                &profile,
                "list",
                "--workspace-id",
                WORKSPACE,
            ]
            .map(str::to_owned)
            .to_vec(),
            32,
        ),
        (
            ["workspace", "workspaces", "--profile", &profile]
                .map(str::to_owned)
                .to_vec(),
            500,
        ),
    ]
}

fn run(args: &[String], limit: Option<&str>) -> std::io::Result<Output> {
    let mut command = Command::new(env!("CARGO_BIN_EXE_client"));
    command.args(args);
    if let Some(limit) = limit {
        // `=` keeps values such as `-1` from being parsed as flags.
        command.arg(format!("--limit={limit}"));
    }
    command.output()
}

#[test]
fn out_of_range_limits_are_usage_errors_naming_the_bounds() {
    for (args, max) in listings() {
        let over = (max + 1).to_string();
        for bad in ["0", over.as_str(), "-1", "65536", "4294967296", "ten", ""] {
            let output = run(&args, Some(bad)).unwrap();
            assert_eq!(output.status.code(), Some(2), "{args:?} --limit {bad:?}");
            let stderr = String::from_utf8(output.stderr).unwrap();
            assert!(
                stderr.contains(&format!("--limit must be between 1 and {max}")),
                "{args:?} --limit {bad:?}: {stderr}"
            );
            assert!(!stderr.contains("mismatched"), "{stderr}");
            assert!(output.stdout.is_empty());
        }
    }
}

#[test]
fn in_range_and_default_limits_pass_the_parser() {
    for (args, max) in listings() {
        let max = max.to_string();
        for limit in [None, Some("1"), Some(max.as_str())] {
            // Parsing succeeds, so the command proceeds to open the missing
            // profile and fails at runtime (exit 1) rather than with usage (2).
            let output = run(&args, limit).unwrap();
            assert_eq!(output.status.code(), Some(1), "{args:?} --limit {limit:?}");
            let stderr = String::from_utf8(output.stderr).unwrap();
            assert!(!stderr.contains("--limit"), "{args:?}: {stderr}");
        }
    }
}
