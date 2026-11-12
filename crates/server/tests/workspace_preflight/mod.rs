// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn software_metadata_uses_both_tls_surfaces_without_opening_or_writing_database() {
    let f = Fixture::new().await;
    let (receipt, key) = f.enroll().await;
    let private = f.private_client(receipt["certificate_pem"].as_str().unwrap(), &key);
    let public = f.public_client();
    let db = f.db();
    db.execute_batch("BEGIN EXCLUSIVE").unwrap();
    let before = std::fs::read(f.directory.join("ccp.sqlite3")).unwrap();
    let expected = json!({"instance_id":f.trust.instance_id,"package_version":env!("CARGO_PKG_VERSION"),"native_wire":1,"workspace_api":1});
    for (http, origin) in [(&public, f.url("")), (&private, f.mtls(""))] {
        let result = json_reply(
            http.get(format!("{origin}/api/v1/software-info"))
                .send()
                .await
                .unwrap(),
            200,
        )
        .await;
        assert_eq!(result["data"], expected);
        json_reply(
            http.post(format!("{origin}/api/v1/software-info"))
                .json(&json!({}))
                .send()
                .await
                .unwrap(),
            405,
        )
        .await;
        json_reply(
            http.get(format!("{origin}/api/v1/software-info?extra=1"))
                .send()
                .await
                .unwrap(),
            400,
        )
        .await;
    }
    assert_eq!(
        std::fs::read(f.directory.join("ccp.sqlite3")).unwrap(),
        before
    );
    assert!(
        public
            .get(f.mtls("/api/v1/software-info"))
            .send()
            .await
            .is_err()
    );
    db.execute_batch("ROLLBACK").unwrap();
    drop(db);
    f.finish().await;
}
