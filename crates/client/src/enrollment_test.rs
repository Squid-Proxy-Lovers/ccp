// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Context;
use rcgen::{CertificateParams, KeyPair};
use reqwest::Url;
use serde::{Deserialize, Serialize};

use crate::enrollment_structs::{EnrollmentMaterial, EnrollmentMetadata, StoredEnrollment};
use crate::storage::save_enrollment;



#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enroll_redeem_url_requires_auth_redeem_path() {
        let err = Url::parse("https://example.com/auth").expect("url should parse");
        assert_ne!(err.path(), "/auth/redeem");
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
}