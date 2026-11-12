// SPDX-License-Identifier: AGPL-3.0-or-later
use super::*;

const PREVIEW: &str = "/api/v1/admin/workspaces/deletion/preview";
const START: &str = "/api/v1/admin/workspaces/deletion/start";
const CHECK: &str = "/api/v1/admin/workspaces/deletion/status";
const ARCHIVE: &str = "/api/v1/admin/workspaces/archive";

async fn bearer(f: &Fixture, key: &str, path: &str, body: &Value, status: u16) -> Value {
    json_reply(
        f.public_client()
            .post(f.url(path))
            .bearer_auth(key)
            .json(body)
            .send()
            .await
            .unwrap(),
        status,
    )
    .await
}
fn intent(f: &Fixture, revision: &Value) -> Value {
    json!({"workspace_id":f.setup.workspace_id.to_string(),"operation_id":Uuid::new_v4().to_string(),"expected_revision":revision,"confirm_workspace_id":f.setup.workspace_id.to_string()})
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn deletion_owner_cookie_confirmation_csrf_scoped_key_retirement_and_global_receipt() {
    let f = Fixture::new().await;
    let second = f.second();
    let (cookie, session) = login(&f, f.setup.owner_key.secret(), None).await;
    let csrf = session["csrf_token"].as_str().unwrap();
    post(&f, &cookie, csrf, PREVIEW, &target(&f), 403).await;
    post(&f, &cookie, csrf, ARCHIVE, &target(&f), 200).await;
    let preview = post(&f, &cookie, csrf, PREVIEW, &target(&f), 200).await;
    assert_eq!(preview["data"]["name"], "first");
    let body = intent(&f, &preview["data"]["lifecycle_revision"]);
    let mut wrong = body.clone();
    wrong["confirm_workspace_id"] = json!(second.workspace_id.to_string());
    code(
        &post(&f, &cookie, csrf, START, &wrong, 400).await,
        "invalid_request",
    );
    for proof in [None, Some("wrong")] {
        let mut request = f
            .public_client()
            .post(f.url(START))
            .header("cookie", &cookie)
            .json(&body);
        if let Some(proof) = proof {
            request = request
                .header("origin", origin(&f))
                .header("x-ccp-csrf", proof);
        }
        json_reply(request.send().await.unwrap(), 403).await;
    }
    assert_eq!(
        f.db()
            .query_row("SELECT count(*) FROM ccp_workspace_deletions", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    let receipt = post(&f, &cookie, csrf, START, &body, 200).await;
    assert!(receipt["data"]["completed_at"].is_null());
    assert_eq!(receipt["data"]["operation_id"], body["operation_id"]);
    assert_eq!(
        receipt["data"]["lifecycle_revision"].as_u64().unwrap(),
        preview["data"]["lifecycle_revision"].as_u64().unwrap() + 1
    );
    let check = json!({"workspace_id":body["workspace_id"],"operation_id":body["operation_id"]});
    // Normal authentication rejects the scoped key immediately, even before row retirement.
    post(&f, &cookie, csrf, CHECK, &check, 401).await;
    bearer(&f, f.setup.owner_key.secret(), START, &body, 401).await;
    let result = bearer(&f, f.setup.instance_key.secret(), CHECK, &check, 200).await;
    assert_eq!(result["data"]["started_at"], receipt["data"]["started_at"]);
    assert!(!result.to_string().contains("first"));
    bearer(&f, f.setup.instance_key.secret(), START, &body, 409).await;
    let other = json!({"workspace_id":second.workspace_id.to_string()});
    bearer(&f, second.owner_key.secret(), PREVIEW, &other, 403).await;
    assert_eq!(
        f.db()
            .query_row(
                "SELECT state FROM ccp_workspaces WHERE id=?1",
                [second.workspace_id.to_string()],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
        "active"
    );
    f.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn deletion_https_strict_admin_only_boundary_and_current_global_authority() {
    let f = Fixture::new().await;
    let (identity, key) = f.enroll().await;
    let admin = {
        let mut db = f.db();
        let mut s = AccessStore::new(&mut db).unwrap();
        let actor = s
            .authenticate_admin_key(f.setup.owner_key.secret(), now())
            .unwrap();
        s.appoint_workspace_admin_with_key(
            actor,
            f.setup.workspace_id,
            &server::foundation::AdminAppointment {
                principal: InitialOwner::NewPrincipal("deletion admin".into()),
                key_expires_at: None,
            },
            now(),
        )
        .unwrap()
        .secret()
        .to_owned()
    };
    bearer(&f, f.setup.instance_key.secret(), ARCHIVE, &target(&f), 200).await;
    let preview = bearer(&f, f.setup.instance_key.secret(), PREVIEW, &target(&f), 200).await;
    let body = intent(&f, &preview["data"]["lifecycle_revision"]);
    bearer(&f, &admin, START, &body, 403).await;
    let mtls = f.private_client(identity["certificate_pem"].as_str().unwrap(), &key);
    for path in [PREVIEW, START, CHECK] {
        json_reply(
            mtls.post(f.mtls(path)).json(&body).send().await.unwrap(),
            404,
        )
        .await;
        json_reply(
            f.public_client()
                .post(f.url(path))
                .json(&body)
                .send()
                .await
                .unwrap(),
            401,
        )
        .await;
        json_reply(
            f.public_client()
                .get(f.url(path))
                .bearer_auth(f.setup.instance_key.secret())
                .send()
                .await
                .unwrap(),
            405,
        )
        .await;
    }
    let mut extra = body.clone();
    extra["actor_id"] = json!(f.setup.instance_key.principal_id.to_string());
    bearer(&f, f.setup.instance_key.secret(), START, &extra, 400).await;
    let mut missing = body.clone();
    missing
        .as_object_mut()
        .unwrap()
        .remove("confirm_workspace_id");
    bearer(&f, f.setup.instance_key.secret(), START, &missing, 400).await;
    let mut invalid = body.clone();
    invalid["operation_id"] = json!(Uuid::nil().to_string());
    bearer(&f, f.setup.instance_key.secret(), START, &invalid, 400).await;
    json_reply(
        f.public_client()
            .post(f.url(&format!("{START}?workspace_id=other")))
            .bearer_auth(f.setup.instance_key.secret())
            .json(&body)
            .send()
            .await
            .unwrap(),
        400,
    )
    .await;
    let receipt = bearer(&f, f.setup.instance_key.secret(), START, &body, 200).await;
    assert_eq!(
        bearer(&f, f.setup.instance_key.secret(), START, &body, 200).await,
        receipt
    );
    json_reply(
        mtls.get(f.mtls("/api/v1/workspaces")).send().await.unwrap(),
        200,
    )
    .await;
    let check =
        json!({"workspace_id":body["workspace_id"],"operation_id":Uuid::new_v4().to_string()});
    bearer(&f, f.setup.instance_key.secret(), CHECK, &check, 404).await;
    let global = f.setup.instance_key.principal_id;
    f.db()
        .execute(
            "DELETE FROM ccp_server_admins WHERE admin_id=?1",
            [global.to_string()],
        )
        .unwrap();
    bearer(&f, f.setup.instance_key.secret(), CHECK, &check, 401).await;
    f.finish().await;
}

async fn restart(f: Fixture) -> Fixture {
    let Fixture {
        directory,
        setup,
        trust,
        stop,
        task,
        ..
    } = f;
    stop.send(()).unwrap();
    task.await.unwrap().unwrap();
    let server = WorkspaceServer::bind(Config {
        data_dir: directory.clone(),
        https_listen: "127.0.0.1:0".parse().unwrap(),
        mtls_listen: "127.0.0.1:0".parse().unwrap(),
        activate_workspace: false,
    })
    .await
    .unwrap();
    let startup = server.startup().unwrap();
    let (stop, signal) = oneshot::channel();
    let task =
        tokio::spawn(server.run_until(async { signal.await.context("test restart stop dropped") }));
    Fixture {
        directory,
        setup,
        trust,
        startup,
        stop,
        task,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn deletion_real_worker_resumes_after_restart_and_removes_payload_without_touching_other_workspace()
 {
    let f = Fixture::new().await;
    let second = f.second();
    for index in 0..17 {
        json_reply(f.public_client().post(f.url("/api/v1/content/shelves/create")).bearer_auth(f.setup.owner_key.secret()).json(&json!({"workspace_id":f.setup.workspace_id.to_string(),"name":format!("PRIVATE_NAME{index}"),"description":"PRIVATE_BODY"})).send().await.unwrap(),200).await;
    }
    bearer(&f, f.setup.instance_key.secret(), ARCHIVE, &target(&f), 200).await;
    let p = bearer(&f, f.setup.instance_key.secret(), PREVIEW, &target(&f), 200).await;
    let body = intent(&f, &p["data"]["lifecycle_revision"]);
    let ws = f.setup.workspace_id.to_string();
    f.db().execute_batch("CREATE TRIGGER deletion_worker_payload_fault BEFORE DELETE ON ccp_content_shelves BEGIN SELECT RAISE(ABORT,'PRIVATE_BODY'); END;").unwrap();
    let started = bearer(&f, f.setup.instance_key.secret(), START, &body, 200).await;
    tokio::time::timeout(Duration::from_secs(6),async {
        loop {
            if f.db().query_row("SELECT count(*) FROM ccp_admin_keys WHERE workspace_id=?1 AND revoked_at IS NULL",[&ws],|r|r.get::<_,usize>(0)).unwrap()==0 {break;}
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }).await.unwrap();
    let f = restart(f).await;
    let check = json!({"workspace_id":ws,"operation_id":body["operation_id"]});
    let pending = bearer(&f, f.setup.instance_key.secret(), CHECK, &check, 200).await;
    assert_eq!(pending["data"]["started_at"], started["data"]["started_at"]);
    assert!(pending["data"]["completed_at"].is_null());
    assert_eq!(
        f.db()
            .query_row("SELECT count(*) FROM ccp_content_shelves", [], |r| r
                .get::<_, usize>(0))
            .unwrap(),
        17
    );
    f.db()
        .execute_batch("DROP TRIGGER deletion_worker_payload_fault")
        .unwrap();
    tokio::time::timeout(Duration::from_secs(25), async {
        loop {
            let receipt = bearer(&f, f.setup.instance_key.secret(), CHECK, &check, 200).await;
            if !receipt["data"]["completed_at"].is_null() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        f.db()
            .query_row("SELECT count(*) FROM ccp_content_shelves", [], |r| r
                .get::<_, usize>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        f.db()
            .query_row(
                "SELECT count(*) FROM ccp_access_audit WHERE workspace_id=?1",
                [&ws],
                |r| r.get::<_, usize>(0)
            )
            .unwrap(),
        0
    );
    let second_body = json!({"workspace_id":second.workspace_id.to_string()});
    bearer(
        &f,
        second.owner_key.secret(),
        "/api/v1/admin/workspaces/inspect",
        &second_body,
        200,
    )
    .await;
    f.finish().await;
}
