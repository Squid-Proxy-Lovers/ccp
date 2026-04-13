// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;
use crate::identity::{
    ACCESS_URI_PREFIX, SESSION_URI_PREFIX, parse_access_identity_uri, parse_session_identity_uri,
};
use rcgen::KeyPair;
use rustls::pki_types::pem::PemObject;
use x509_parser::{extensions::GeneralName, parse_x509_certificate};

#[test]
fn extract_auth_request_accepts_redeem_json_post() {
    let request = "\
POST /auth/redeem HTTP/1.1\r\n\
Host: localhost\r\n\
Content-Type: application/json\r\n\
Content-Length: 50\r\n\
\r\n\
{\"token\":\"test-token\",\"csr_pem\":\"csr\"}";
    assert_eq!(
        extract_auth_request(request).expect("JSON auth request should parse"),
        Some(AuthRedeemRequest {
            token: "test-token".to_string(),
            csr_pem: "csr".to_string(),
        })
    );
}

#[test]
fn extract_auth_request_rejects_invalid_payload() {
    let request = "\
POST /auth/redeem HTTP/1.1\r\n\
Host: localhost\r\n\
Content-Type: application/json\r\n\
Content-Length: 2\r\n\
\r\n\
{}";
    assert!(extract_auth_request(request).is_err());
}

#[test]
fn enrollment_tokens_remain_redeemable_until_expiry() {
    let _env_guard = crate::init::test_env_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let temp_dir = std::env::temp_dir().join(format!("ccp-auth-token-{}", Uuid::new_v4()));
    fs::create_dir_all(&temp_dir).expect("temp dir should be created");
    unsafe {
        std::env::set_var(crate::init::SERVER_DATA_DIR_ENV, &temp_dir);
    }
    crate::init::init_sqlite(&crate::init::db_path()).expect("sqlite should init");
    tokio::runtime::Runtime::new()
        .expect("tokio runtime should initialize")
        .block_on(crate::init::initialize_cpp_server("auth-token-session"))
        .expect("server bootstrap should succeed");

    let issued = crate::init::issue_enrollment_token("auth-token-session", "read", Some(3600))
        .expect("read token should issue");

    let first = consume_auth_token(&issued.token).expect("first redeem should succeed");
    let second = consume_auth_token(&issued.token).expect("second redeem should succeed");

    assert_eq!(first.session_id, second.session_id);
    assert_eq!(first.access_level, "read");
    assert_eq!(second.access_level, "read");
    assert_eq!(first.metadata, second.metadata);

    unsafe {
        std::env::remove_var(crate::init::SERVER_DATA_DIR_ENV);
    }
    fs::remove_dir_all(&temp_dir).expect("temp dir should be removed");
}

#[test]
fn client_certificate_embeds_session_identity_uri() {
    let _env_guard = crate::init::test_env_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let temp_dir = std::env::temp_dir().join(format!("ccp-auth-cert-{}", Uuid::new_v4()));
    fs::create_dir_all(&temp_dir).expect("temp dir should be created");
    unsafe {
        std::env::set_var(crate::init::SERVER_DATA_DIR_ENV, &temp_dir);
    }
    crate::init::init_sqlite(&crate::init::db_path()).expect("sqlite should init");
    tokio::runtime::Runtime::new()
        .expect("tokio runtime should initialize")
        .block_on(crate::init::initialize_cpp_server("auth-cert-session"))
        .expect("server bootstrap should succeed");

    let key = KeyPair::generate().expect("key should generate");
    let csr_params = CertificateParams::new(Vec::<String>::new()).expect("csr params should build");
    let csr = csr_params
        .serialize_request(&key)
        .expect("csr should serialize")
        .pem()
        .expect("csr pem should encode");
    let (_, cert_pem, _, _, _) = issue_client_certificate(1, "read_write", &csr)
        .expect("client certificate issuance should succeed");
    let der = rustls::pki_types::CertificateDer::pem_slice_iter(cert_pem.as_bytes())
        .next()
        .expect("certificate PEM should parse")
        .expect("certificate PEM should decode");
    let (_, cert) = parse_x509_certificate(der.as_ref()).expect("certificate DER should parse");
    let san = cert
        .subject_alternative_name()
        .expect("SAN extension lookup should work")
        .expect("certificate should contain SAN");
    let uri = san
        .value
        .general_names
        .iter()
        .find_map(|name| match name {
            GeneralName::URI(uri) if uri.starts_with(SESSION_URI_PREFIX) => {
                Some((*uri).to_string())
            }
            _ => None,
        })
        .expect("certificate should contain CCP session identity URI");
    assert_eq!(
        parse_session_identity_uri(&uri).expect("session URI should parse"),
        1
    );
    let access_uri = san
        .value
        .general_names
        .iter()
        .find_map(|name| match name {
            GeneralName::URI(uri) if uri.starts_with(ACCESS_URI_PREFIX) => Some((*uri).to_string()),
            _ => None,
        })
        .expect("certificate should contain CCP access identity URI");
    assert_eq!(
        parse_access_identity_uri(&access_uri).expect("access URI should parse"),
        "read_write"
    );

    unsafe {
        std::env::remove_var(crate::init::SERVER_DATA_DIR_ENV);
    }
    fs::remove_dir_all(&temp_dir).expect("temp dir should be removed");
}
