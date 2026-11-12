// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later
use super::*;
const OVERVIEW: &str = "/api/v1/admin/workspaces/overview";
const INSPECT: &str = "/api/v1/admin/workspaces/inspect";
const DAY: i64 = 86_400;

fn window(f: &Fixture, workspace: Uuid, days: i64) -> Value {
    let _ = f;
    json!({"workspace_id":workspace.to_string(),"days":days})
}
async fn write(f: &Fixture, client: &Client, path: &str, body: &Value) -> Value {
    json_reply(
        client
            .post(f.mtls(&format!("/api/v1/content/{path}")))
            .json(body)
            .send()
            .await
            .unwrap(),
        200,
    )
    .await["data"]
        .clone()
}
fn change(f: &Fixture, change: Value) -> Value {
    json!({"workspace_id":f.setup.workspace_id.to_string(),"operation_id":Uuid::new_v4().to_string(),"agent":"agent","change":change})
}
fn backdate(f: &Fixture, entry: &Value, revision: i64, action: &str, client: &Value, at: i64) {
    f.db().execute("INSERT INTO ccp_content_versions(entry_id,workspace_id,revision,action,actor_kind,actor_id,client_id,recorded_at,content_json) VALUES (?1,?2,?3,?4,'credential',?5,?5,?6,'{}')",
        rusqlite::params![entry.as_str().unwrap(),f.setup.workspace_id.to_string(),revision,action,client.as_str().unwrap(),at]).unwrap();
}
fn counts(day: &Value) -> [i64; 5] {
    ["create", "append", "revise", "archive", "restore"].map(|key| day[key].as_i64().unwrap())
}

/// Client and admin content writes in today's window; returns the client ID.
async fn seed(f: &Fixture, cookie: &str, csrf: &str) -> Value {
    let (receipt, key) = f.enroll().await;
    let client = f.private_client(receipt["certificate_pem"].as_str().unwrap(), &key);
    let workspace = f.setup.workspace_id.to_string();
    let shelf = write(
        f,
        &client,
        "shelves/create",
        &json!({"workspace_id":workspace,"name":"secret shelf","description":""}),
    )
    .await;
    let book = write(f,&client,"books/create",&json!({"workspace_id":workspace,"shelf_id":shelf["id"],"name":"secret book","description":""})).await;
    let body = json!({"text":"private body","description":"","labels":[]});
    let kept = write(
        f,
        &client,
        "entries/mutate",
        &change(
            f,
            json!({"action":"create","book_id":book["id"],"name":"secret entry","content":body}),
        ),
    )
    .await;
    let gone = write(
        f,
        &client,
        "entries/mutate",
        &change(
            f,
            json!({"action":"create","book_id":book["id"],"name":"archived entry","content":body}),
        ),
    )
    .await;
    write(f,&client,"entries/mutate",&change(f,json!({"action":"append","entry_id":kept["entry_id"],"expected_revision":1,"text":" more"}))).await;
    write(
        f,
        &client,
        "entries/mutate",
        &change(
            f,
            json!({"action":"archive","entry_id":gone["entry_id"],"expected_revision":1}),
        ),
    )
    .await;
    let revise = change(
        f,
        json!({"action":"revise","entry_id":kept["entry_id"],"expected_revision":2,"content":body}),
    );
    post(
        f,
        cookie,
        csrf,
        "/api/v1/content/entries/mutate",
        &revise,
        200,
    )
    .await;
    let today = now() - now().rem_euclid(DAY);
    let id = receipt["client_id"].clone();
    backdate(
        f,
        &kept["entry_id"],
        90,
        "restore",
        &id,
        today - 2 * DAY + 60,
    );
    backdate(f, &kept["entry_id"], 91, "revise", &id, today - 6 * DAY - 1);
    backdate(f, &kept["entry_id"], 92, "revise", &id, today + DAY);
    id
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn browser_overview_counts_real_writes_by_utc_day_action_and_writer() {
    let f = Fixture::new().await;
    let (cookie, session) = login(&f, f.setup.owner_key.secret(), None).await;
    let csrf = session["csrf_token"].as_str().unwrap();
    let empty = post(
        &f,
        &cookie,
        csrf,
        OVERVIEW,
        &window(&f, f.setup.workspace_id, 3),
        200,
    )
    .await;
    assert_eq!(empty["data"]["activity"].as_array().unwrap().len(), 3);
    for day in empty["data"]["activity"].as_array().unwrap() {
        assert_eq!(counts(day), [0; 5]);
    }
    assert_eq!(empty["data"]["writers"], json!([]));
    assert_eq!(
        empty["data"]["totals"],
        json!({"shelves":0,"books":0,"entries":0,"archived_entries":0})
    );
    let client = seed(&f, &cookie, csrf).await;
    post(&f,&cookie,csrf,"/api/v1/admin/invitations/individual",&json!({"workspace_id":f.setup.workspace_id.to_string(),"role":"read","expires_at":now()+3600}),200).await;
    f.db()
        .execute(
            "UPDATE ccp_credentials SET expires_at=?1 WHERE client_id=?2",
            rusqlite::params![now() + 3600, client.as_str().unwrap()],
        )
        .unwrap();
    let before = now();
    let value = post(
        &f,
        &cookie,
        csrf,
        OVERVIEW,
        &window(&f, f.setup.workspace_id, 7),
        200,
    )
    .await;
    let data = &value["data"];
    assert_eq!(data.as_object().unwrap().len(), 7);
    assert_eq!(data["workspace_id"], f.setup.workspace_id.to_string());
    assert_eq!(data["days"], 7);
    let generated = data["generated_at"].as_i64().unwrap();
    assert!((before..=now()).contains(&generated));
    let today = generated - generated.rem_euclid(DAY);
    let activity = data["activity"].as_array().unwrap();
    assert_eq!(activity.len(), 7);
    for (index, day) in activity.iter().enumerate() {
        let offset = 6 - i64::try_from(index).unwrap();
        assert_eq!(day["day"], today - offset * DAY);
        let expected = match offset {
            0 => [2, 1, 1, 1, 0],
            2 => [0, 0, 0, 0, 1],
            _ => [0; 5],
        };
        assert_eq!(counts(day), expected, "day offset {offset}");
    }
    assert_eq!(
        data["totals"],
        json!({"shelves":1,"books":1,"entries":1,"archived_entries":1})
    );
    let (name, last): (String, i64) = f.db().query_row("SELECT c.name, MAX(v.recorded_at) FROM ccp_clients c JOIN ccp_content_versions v ON v.client_id=c.id WHERE c.id=?1 AND v.revision<90", [client.as_str().unwrap()], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
    let writers = data["writers"].as_array().unwrap();
    assert_eq!(writers.len(), 2);
    assert_eq!(
        writers[0],
        json!({"client_id":client,"label":name,"writes":5,"last_write_at":last})
    );
    assert_eq!(writers[1]["client_id"], Value::Null);
    assert_eq!(writers[1]["label"], "Admin");
    assert_eq!(writers[1]["writes"], 1);
    assert_eq!(
        data["access"],
        json!({"active_clients":1,"credentials_expiring_within_7d":1,"pending_invitations":1})
    );
    for private in [
        "private body",
        "secret shelf",
        "secret book",
        "secret entry",
        "archived entry",
    ] {
        assert!(!value.to_string().contains(private));
    }
    f.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn browser_overview_access_counts_exclude_closed_and_distant_records() {
    let f = Fixture::new().await;
    let (receipt, _) = f.enroll().await;
    let (cookie, session) = login(&f, f.setup.owner_key.secret(), None).await;
    let csrf = session["csrf_token"].as_str().unwrap();
    let client = receipt["client_id"].as_str().unwrap().to_string();
    let issue = json!({"workspace_id":f.setup.workspace_id.to_string(),"role":"read","expires_at":now()+3600});
    let revoked = post(
        &f,
        &cookie,
        csrf,
        "/api/v1/admin/invitations/individual",
        &issue,
        200,
    )
    .await;
    post(&f,&cookie,csrf,"/api/v1/admin/invitations/revoke",&json!({"workspace_id":f.setup.workspace_id.to_string(),"invitation_id":revoked["data"]["invitation_id"]}),200).await;
    f.db()
        .execute(
            "UPDATE ccp_credentials SET expires_at=?1 WHERE client_id=?2",
            rusqlite::params![now() + 8 * DAY, client],
        )
        .unwrap();
    let access = |value: Value| value["data"]["access"].clone();
    let body = window(&f, f.setup.workspace_id, 1);
    assert_eq!(
        access(post(&f, &cookie, csrf, OVERVIEW, &body, 200).await),
        json!({"active_clients":1,"credentials_expiring_within_7d":0,"pending_invitations":0})
    );
    f.db()
        .execute("UPDATE ccp_clients SET disabled=1 WHERE id=?1", [client])
        .unwrap();
    let value = post(&f, &cookie, csrf, OVERVIEW, &body, 200).await;
    assert_eq!(value["data"]["activity"].as_array().unwrap().len(), 1);
    assert_eq!(
        access(value),
        json!({"active_clients":0,"credentials_expiring_within_7d":0,"pending_invitations":0})
    );
    f.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn browser_overview_reuses_inspect_authority_for_instance_scoped_and_foreign_keys() {
    let f = Fixture::new().await;
    let second = f.second();
    let first = f.setup.workspace_id;
    let (global, session) = login(&f, f.setup.instance_key.secret(), None).await;
    let proof = session["csrf_token"].as_str().unwrap();
    for workspace in [first, second.workspace_id] {
        let value = post(&f, &global, proof, OVERVIEW, &window(&f, workspace, 2), 200).await;
        assert_eq!(value["data"]["workspace_id"], workspace.to_string());
    }
    let (foreign, session) = login(&f, second.owner_key.secret(), None).await;
    let proof = session["csrf_token"].as_str().unwrap();
    let missing = Uuid::new_v4();
    for workspace in [first, missing] {
        let inspect = post(
            &f,
            &foreign,
            proof,
            INSPECT,
            &json!({"workspace_id":workspace.to_string()}),
            403,
        )
        .await;
        let overview = post(
            &f,
            &foreign,
            proof,
            OVERVIEW,
            &window(&f, workspace, 2),
            403,
        )
        .await;
        code(&overview, "forbidden");
        assert_eq!(overview, inspect);
    }
    post(
        &f,
        &foreign,
        proof,
        OVERVIEW,
        &window(&f, second.workspace_id, 2),
        200,
    )
    .await;
    f.db()
        .execute(
            "UPDATE ccp_workspaces SET state='purged' WHERE id=?1",
            [second.workspace_id.to_string()],
        )
        .unwrap();
    let purged = window(&f, second.workspace_id, 2);
    let inspect = post(
        &f,
        &foreign,
        proof,
        INSPECT,
        &json!({"workspace_id":second.workspace_id.to_string()}),
        401,
    )
    .await;
    code(&inspect, "unauthorized");
    let (global, session) = login(&f, f.setup.instance_key.secret(), None).await;
    let proof = session["csrf_token"].as_str().unwrap();
    let inspect = post(
        &f,
        &global,
        proof,
        INSPECT,
        &json!({"workspace_id":second.workspace_id.to_string()}),
        403,
    )
    .await;
    assert_eq!(
        post(&f, &global, proof, OVERVIEW, &purged, 403).await,
        inspect
    );
    f.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn browser_overview_rejects_bad_windows_and_keeps_session_csrf_rules() {
    let f = Fixture::new().await;
    let (cookie, session) = login(&f, f.setup.owner_key.secret(), None).await;
    let csrf = session["csrf_token"].as_str().unwrap();
    let workspace = f.setup.workspace_id.to_string();
    for body in [
        json!({"workspace_id":workspace,"days":0}),
        json!({"workspace_id":workspace,"days":91}),
        json!({"workspace_id":workspace,"days":-1}),
        json!({"workspace_id":workspace,"days":"7"}),
        json!({"workspace_id":workspace}),
        json!({"workspace_id":"not-a-uuid","days":7}),
        json!({"workspace_id":workspace,"days":7,"client_id":"PRIVATE"}),
    ] {
        code(
            &post(&f, &cookie, csrf, OVERVIEW, &body, 400).await,
            "invalid_request",
        );
    }
    let edge = post(
        &f,
        &cookie,
        csrf,
        OVERVIEW,
        &window(&f, f.setup.workspace_id, 90),
        200,
    )
    .await;
    assert_eq!(edge["data"]["activity"].as_array().unwrap().len(), 90);
    let body = window(&f, f.setup.workspace_id, 7);
    code(
        &post(&f, &cookie, "wrong", OVERVIEW, &body, 403).await,
        "forbidden",
    );
    let public = f.public_client();
    let unsigned = public
        .post(f.url(OVERVIEW))
        .header("cookie", &cookie)
        .header("x-ccp-csrf", csrf)
        .json(&body);
    code(
        &json_reply(unsigned.send().await.unwrap(), 403).await,
        "forbidden",
    );
    let anonymous = public
        .post(f.url(OVERVIEW))
        .header("origin", origin(&f))
        .json(&body);
    code(
        &json_reply(anonymous.send().await.unwrap(), 401).await,
        "unauthorized",
    );
    let get = public.get(f.url(OVERVIEW)).header("cookie", &cookie);
    code(
        &json_reply(get.send().await.unwrap(), 405).await,
        "method_not_allowed",
    );
    let mixed = public
        .post(f.url(OVERVIEW))
        .header("cookie", &cookie)
        .header("origin", origin(&f))
        .header("x-ccp-csrf", csrf)
        .bearer_auth(f.setup.owner_key.secret())
        .json(&body);
    code(
        &json_reply(mixed.send().await.unwrap(), 400).await,
        "invalid_request",
    );
    let bearer = public
        .post(f.url(OVERVIEW))
        .bearer_auth(f.setup.owner_key.secret())
        .json(&body);
    assert_eq!(
        json_reply(bearer.send().await.unwrap(), 200).await["data"]["days"],
        7
    );
    f.finish().await;
}
