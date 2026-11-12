// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;
use ccp_transport::{
    ByteStream, CLIENT_PATH, ENROLL_PATH, SUBPROTOCOL,
    websocket::{
        self, WebSocketStream,
        tungstenite::{client::IntoClientRequest, protocol::Role},
    },
    websocket_config,
};
mod gates;
mod lifecycle;
type Carrier = ByteStream<TlsStream<TcpStream>>;
type Inner = tokio_rustls::client::TlsStream<Carrier>;

async fn upgrade(f: &Fixture, path: &str) -> WebSocketStream<TlsStream<TcpStream>> {
    let wire = Wire::connect(f, None).await;
    let mut request = format!("wss://{}{path}", f.startup.https_address)
        .into_client_request()
        .unwrap();
    request
        .headers_mut()
        .insert("sec-websocket-protocol", SUBPROTOCOL.parse().unwrap());
    let (socket, response) = tokio::time::timeout(
        Duration::from_secs(3),
        websocket::client_async_with_config(request, wire.stream, Some(websocket_config())),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(response.status(), 101);
    assert_eq!(response.headers()["sec-websocket-protocol"], SUBPROTOCOL);
    assert!(!response.headers().contains_key("sec-websocket-extensions"));
    socket
}
async fn connect(f: &Fixture, identity: Option<(&str, &KeyPair)>) -> Wire<Inner> {
    let path = if identity.is_some() {
        CLIENT_PATH
    } else {
        ENROLL_PATH
    };
    let socket = upgrade(f, path).await;
    let tls = TlsConnector::from(Arc::new(tls_config(
        Some(&f.trust.ca_certificate_pem),
        identity,
    )))
    .connect(
        ServerName::try_from("localhost").unwrap(),
        ByteStream::new(socket),
    )
    .await
    .unwrap();
    Wire {
        stream: tls,
        buffer: Vec::new(),
    }
}
async fn send<S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin>(
    wire: &mut Wire<S>,
    path: &str,
    body: Option<&Value>,
    extra: &str,
) -> (u16, Value) {
    let data = body
        .map(|b| serde_json::to_string(b).unwrap())
        .unwrap_or_default();
    let method = if body.is_some() { "POST" } else { "GET" };
    wire.send(&format!("{method} {path} HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: {}\r\n{extra}\r\n{data}",data.len())).await;
    wire.response().await
}
async fn enroll(f: &Fixture, workspace: Uuid, key: &str) -> (Value, KeyPair) {
    let token = f.initial(key, workspace).await;
    let key = KeyPair::generate().unwrap();
    let body = json!({"invitation":token["secret"],"request_id":Uuid::new_v4().to_string(),"csr_pem":csr(&key)});
    let mut wire = connect(f, None).await;
    let (code, result) = send(&mut wire, "/api/v1/enroll", Some(&body), "").await;
    assert_eq!(code, 200);
    (result["data"].clone(), key)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn enrollment_and_actual_mtls_identity_share_the_browser_listener() {
    let f = Fixture::with_tunnels(true).await;
    assert_eq!(
        f.public_client()
            .get(f.url("/admin"))
            .send()
            .await
            .unwrap()
            .status(),
        200
    );
    let (receipt, key) = enroll(&f, f.setup.workspace_id, f.setup.owner_key.secret()).await;
    let mut inner = connect(
        &f,
        Some((receipt["certificate_pem"].as_str().unwrap(), &key)),
    )
    .await;
    let (code, list) = send(&mut inner, "/api/v1/workspaces", None, "").await;
    assert_eq!(code, 200);
    assert_eq!(
        list["data"]["workspaces"][0]["id"],
        f.setup.workspace_id.to_string()
    );
    let status = Uuid::new_v4().to_string();
    let body = json!({"workspace_id":f.setup.workspace_id.to_string(),"status_id":status,"operation_id":Uuid::new_v4().to_string(),"expected_revision":0,"change":{"action":"set","agent":null,"message":"secret carrier status"}});
    let (code, _) = send(
        &mut inner,
        "/api/v1/work/status/mutate",
        Some(&body),
        "X-CCP-Client: spoof\r\n",
    )
    .await;
    assert_eq!(code, 200);
    let (code, current) = send(
        &mut inner,
        "/api/v1/work/status/get",
        Some(&json!({"workspace_id":f.setup.workspace_id.to_string(),"status_id":status})),
        "",
    )
    .await;
    assert_eq!(code, 200);
    assert_eq!(current["data"]["client_id"], receipt["client_id"]);
    assert_eq!(current["data"]["message"], "secret carrier status");
    let (code, not_found) = send(&mut inner, CLIENT_PATH, None, "").await;
    assert_eq!(code, 404);
    assert_eq!(not_found["error"]["code"], "not_found");
    drop(inner);
    f.finish().await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bootstrap_cannot_reach_admin_even_with_valid_key_or_browser_cookie() {
    let f = Fixture::with_tunnels(true).await;
    let login = f
        .public_client()
        .post(f.url("/api/v1/admin/session/login"))
        .header("origin", format!("https://{}", f.startup.https_address))
        .header("x-ccp-csrf", "login")
        .json(&json!({"key":f.setup.owner_key.secret()}))
        .send()
        .await
        .unwrap();
    assert_eq!(login.status(), 200);
    let cookie = login.headers()["set-cookie"]
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_string();
    let mut inner = connect(&f, None).await;
    for path in [
        "/admin",
        "/admin/app.js",
        "/api/v1/admin/session",
        "/api/v1/admin/workspaces",
        "/api/v1/admin/invitations/link",
        CLIENT_PATH,
    ] {
        let (code, result) = send(
            &mut inner,
            path,
            None,
            &format!(
                "Authorization: Bearer {}\r\nCookie: {cookie}\r\n",
                f.setup.owner_key.secret()
            ),
        )
        .await;
        assert_eq!(code, 404);
        assert_eq!(result["error"]["code"], "not_found");
    }
    for path in [
        "/api/v1/admin/session/login",
        "/api/v1/admin/invitations/initial",
    ] {
        let (code, result) = send(
            &mut inner,
            path,
            Some(&json!({"key":f.setup.owner_key.secret()})),
            "",
        )
        .await;
        assert_eq!(code, 404);
        assert_eq!(result["error"]["code"], "not_found");
    }
    for path in ["/api/v1/health", "/api/v1/software-info"] {
        let (code, result) = send(&mut inner, path, None, "").await;
        assert_eq!(code, 200);
        assert_eq!(result["data"]["instance_id"], f.trust.instance_id);
    }
    drop(inner);
    f.finish().await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_other_workspace_and_current_revocation_are_checked_on_reused_inner_connection() {
    let f = Fixture::with_tunnels(true).await;
    let second = f.second();
    let (first, key) = enroll(&f, f.setup.workspace_id, f.setup.owner_key.secret()).await;
    let (other, other_key) = enroll(&f, second.workspace_id, second.owner_key.secret()).await;
    let mut wire = connect(&f, Some((first["certificate_pem"].as_str().unwrap(), &key))).await;
    let mut authorized = connect(
        &f,
        Some((other["certificate_pem"].as_str().unwrap(), &other_key)),
    )
    .await;
    let target = json!({"workspace_id":second.workspace_id.to_string(),"after":null,"limit":1});
    let (code, _) = send(
        &mut authorized,
        "/api/v1/work/status/list",
        Some(&target),
        "",
    )
    .await;
    assert_eq!(code, 200);
    let (code, refusal) = send(
        &mut wire,
        "/api/v1/work/status/list",
        Some(&target),
        &format!("X-CCP-Client: {}\r\n", other["client_id"].as_str().unwrap()),
    )
    .await;
    assert_eq!(code, 403);
    assert_eq!(refusal["error"]["code"], "forbidden");
    let (code, _) = send(&mut wire, "/api/v1/workspaces", None, "").await;
    assert_eq!(code, 200);
    f.db()
        .execute(
            "UPDATE ccp_credentials SET revoked_at=?1 WHERE id=?2",
            rusqlite::params![now(), first["credential_id"].as_str().unwrap()],
        )
        .unwrap();
    let (code, result) = send(&mut wire, "/api/v1/workspaces", None, "").await;
    assert_eq!(code, 403);
    assert_eq!(result["error"]["code"], "forbidden");
    drop(wire);
    drop(authorized);
    f.finish().await;
}
