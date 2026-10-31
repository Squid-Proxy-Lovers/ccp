// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;

#[test]
fn enroll_redeem_url_requires_auth_redeem_path() {
    for value in [
        "https://example.com/auth",
        "https://example.com/auth/redeem/",
        "not a URL",
    ] {
        assert!(parse_redeem_url(value).is_err(), "accepted {value}");
    }
    assert_eq!(
        parse_redeem_url("https://example.com/auth/redeem")
            .unwrap()
            .path(),
        "/auth/redeem"
    );
    assert!(parse_redeem_url("http://127.0.0.1/auth/redeem").is_ok());
}

#[test]
fn auth_redeem_response_deserializes() {
    let response = r#"{
        "session": {
            "session_name": "test-session",
            "session_id": 7,
            "description": "desc",
            "owner": "dudcom",
            "labels": ["rust", "agents"],
            "visibility": "private",
            "purpose": "testing"
        },
        "access": "read_write",
        "client_common_name": "client-123",
        "mtls_endpoint": "https://localhost:1338",
        "client_cert_expires_at": 123456,
        "cert_warning_window_seconds": 1209600,
        "ca_cert_pem": "CA-LINE\n",
        "client_cert_pem": "CERT-LINE\n"
    }"#;

    let parsed: AuthRedeemResponse =
        serde_json::from_str(response).expect("response should deserialize");

    assert_eq!(parsed.session.session_name, "test-session");
    assert_eq!(parsed.access, "read_write");
    assert_eq!(parsed.client_common_name, "client-123");
    assert_eq!(parsed.client_cert_expires_at, 123456);
    assert_eq!(parsed._cert_warning_window_seconds, 1209600);
}
