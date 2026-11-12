// SPDX-License-Identifier: AGPL-3.0-or-later
#[cfg(test)]
mod tests {
    use rusqlite::Connection;
    use serde_json::Value;
    use sha2::{Digest, Sha256};
    use std::{
        fs,
        path::PathBuf,
        process::{Command, Output},
    };

    struct Fixture(PathBuf);
    impl Drop for Fixture {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).unwrap();
        }
    }
    fn run(f: &Fixture, hash: &str) -> Output {
        Command::new(env!("CARGO_BIN_EXE_server"))
            .args(["legacy-convert", "--data-dir"])
            .arg(f.0.join("source"))
            .arg("--destination")
            .arg(f.0.join("converted"))
            .args([
                "--expect-source-sha256",
                hash,
                "--operator",
                "new operator",
                "--confirm-offline",
            ])
            .output()
            .unwrap()
    }
    #[test]
    fn explicit_cli_converts_once_and_never_emits_source_payload_or_old_credentials() {
        let f = Fixture(
            std::env::temp_dir().join(format!("ccp-native-convert-cli-{}", uuid::Uuid::new_v4())),
        );
        fs::create_dir_all(f.0.join("source")).unwrap();
        let source = f.0.join("source/ccp.sqlite3");
        let db = Connection::open(&source).unwrap();
        db.pragma_update(None, "journal_mode", "WAL").unwrap();
        db.execute_batch(include_str!("../src/init-db.sql"))
            .unwrap();
        db.execute_batch("INSERT INTO schema_version(version) VALUES(2); INSERT INTO sessions(id,name,owner) VALUES(1,'native','PRIVATE_OWNER'); INSERT INTO message_packs(session_id,name,context) VALUES(1,'entry','PRIVATE_SOURCE_BODY'); INSERT INTO auth_tokens(session_id,token_nonce,token_value,token_hash,token_prefix,access_level) VALUES(1,'private','PRIVATE_NATIVE_TOKEN','hash','prefix','admin');").unwrap();
        drop(db);
        let bytes = fs::read(&source).unwrap();
        let hash = format!("{:x}", Sha256::digest(&bytes));
        let output = run(&f, &hash);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let value: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(value["receipt"]["conversion_performed"], true);
        assert_eq!(value["receipt"]["source_sha256"], hash);
        assert_eq!(value["receipt"]["entry_count"], 1);
        assert!(
            value["instance_admin_key"]["secret"]
                .as_str()
                .unwrap()
                .starts_with("ccp_admin_")
        );
        assert_eq!(value["workspaces"][0]["name"], "native");
        assert!(!String::from_utf8_lossy(&output.stdout).contains("PRIVATE_"));
        assert_eq!(fs::read(&source).unwrap(), bytes);
        assert_eq!(fs::read_dir(f.0.join("source")).unwrap().count(), 1);
        let target = f.0.join("converted/ccp.sqlite3");
        let original = fs::read(&target).unwrap();
        let refused = run(&f, &hash);
        assert!(!refused.status.success());
        assert!(refused.stdout.is_empty());
        assert_eq!(fs::read(&target).unwrap(), original);
        assert_eq!(fs::read(&source).unwrap(), bytes);
    }
}
