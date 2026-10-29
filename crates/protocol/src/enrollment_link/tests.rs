// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later
use super::*;
use proptest::prelude::*;
use uuid::Uuid;

fn target() -> LinkTarget {
    LinkTarget {
        https_origin: "https://ccp.example:8443/".into(),
        mtls_origin: "https://ccp.example:8444/".into(),
        instance_id: Uuid::from_u128(1),
        workspace_id: Uuid::from_u128(2),
        ca_sha256: [0; 32],
        workspace_name: "Team context".into(),
    }
}
fn token() -> String {
    format!("ccp_join_{}", "a".repeat(64))
}
fn uri() -> String {
    EnrollmentLink::new(target(), &token()).unwrap().to_uri()
}

#[test]
fn canonical_roundtrip_keeps_public_target_and_exact_secret_in_fragment_only() {
    let link = EnrollmentLink::parse(&uri()).unwrap();
    assert_eq!(link.target(), &target());
    assert_eq!(link.invitation(), token());
    let uri = link.to_uri();
    assert_eq!(uri.split('#').next(), Some("https://ccp.example:8443/join"));
    assert!(!uri.split('#').next().unwrap().contains(&token()));
    assert!(!uri.contains(['\'', '"', ' ', '\n', '\r']));
    assert_eq!(EnrollmentLink::parse(&uri).unwrap().to_uri(), uri);
}
#[test]
fn port_short_form_and_different_mtls_origin_are_both_supported() {
    let original = uri();
    let short = original.replace("m=https%3A%2F%2Fccp.example%3A8444%2F", "m=8444");
    assert_ne!(short, original);
    assert_eq!(EnrollmentLink::parse(&short).unwrap().target(), &target());
    let mut expected = target();
    expected.mtls_origin = "https://elsewhere.example:9444/".into();
    let link = EnrollmentLink::new(expected.clone(), &token()).unwrap();
    assert_eq!(
        EnrollmentLink::parse(&link.to_uri()).unwrap().target(),
        &expected
    );
}
#[test]
fn malformed_or_ambiguous_links_have_only_fixed_non_secret_errors() {
    let good = uri();
    let invalid = [
        good.replace("https://ccp.example:8443/", "http://ccp.example:8443/"),
        good.replace("/join#", "/join?x=1#"),
        good.replace("/join#", "/./join#"),
        good.replace("https://ccp.example:8443/", "https:////ccp.example:8443/"),
        good.replace("https://ccp.example:8443/", "https://@ccp.example:8443/"),
        good.replace(
            "https://ccp.example:8443/",
            "https://user:pass@ccp.example:8443/",
        ),
        good.replace("v=1", "v=2"),
        format!("{good}&x=1"),
        format!("{good}&v=1"),
        format!("{good}&%76=1"),
        format!("{good}&"),
        format!("{good}#extra"),
        good.replace("m=https%3A%2F%2Fccp.example%3A8444%2F", "m=0"),
        good.replace("m=https%3A%2F%2Fccp.example%3A8444%2F", "m=08444"),
        good.replace(
            "m=https%3A%2F%2Fccp.example%3A8444%2F",
            "m=https%3A%2F%2Fuser%40other.example",
        ),
        good.replace("n=Team+context", "n=%"),
        good.replace("n=Team+context", "n=%GG"),
        good.replace("n=Team+context", "n=%C0%AF"),
        good.replace("n=Team+context", "n=a%00b"),
        good.replace("n=Team+context", "n=%20name"),
        good.replace("n=Team+context", &format!("n={}", "x".repeat(257))),
        good.replace("sha256%3A", "sha1%3A"),
        good.replace(
            "w=00000000-0000-0000-0000-000000000002",
            "w=00000000-0000-0000-0000-000000000000",
        ),
    ];
    for value in invalid {
        let error = EnrollmentLink::parse(&value).err().unwrap();
        assert!(!error.to_string().contains(&token()));
        assert!(!format!("{error:?}").contains(&token()));
    }
    assert_eq!(
        EnrollmentLink::parse(&"x".repeat(MAX_LINK_BYTES + 1)).err(),
        Some(LinkError::Size)
    );
}
#[test]
fn name_bounds_are_bytes_and_origins_cannot_hide_routes_or_shell_metacharacters() {
    for name in ["é".repeat(128), "🦑".repeat(64)] {
        let mut t = target();
        t.workspace_name = name.clone();
        let link = EnrollmentLink::new(t, &token()).unwrap();
        assert_eq!(
            EnrollmentLink::parse(&link.to_uri())
                .unwrap()
                .target()
                .workspace_name,
            name
        );
    }
    for name in ["", " ", "name\nline", "a\u{7f}b"] {
        let mut t = target();
        t.workspace_name = name.into();
        assert_eq!(
            EnrollmentLink::new(t, &token()).err(),
            Some(LinkError::WorkspaceName)
        );
    }
    for origin in [
        "https://a/../",
        "https:///a",
        "https://a?",
        "https://a#",
        "https://a/%2e",
        "https://a'.example",
        "https://a$().example",
        "https://0.0.0.0",
        "https://[::]",
        "https://a:0",
    ] {
        let mut t = target();
        t.https_origin = origin.into();
        assert_eq!(
            EnrollmentLink::new(t, &token()).err(),
            Some(LinkError::Origin)
        );
    }
}
fn targets() -> impl Strategy<Value = LinkTarget> {
    (
        any::<[u8; 32]>(),
        1u128..=u128::MAX,
        1u128..=u128::MAX,
        prop_oneof![Just(443u16), 1u16..=u16::MAX],
        0u8..3,
        any::<bool>(),
        any::<bool>(),
        prop::collection::vec(
            prop_oneof![
                Just('a'),
                Just(' '),
                Just('é'),
                Just('名'),
                Just('🦑'),
                Just('%'),
                Just('&'),
                Just('\''),
                Just('/'),
                Just('+'),
                Just('='),
                Just('#'),
                Just('~')
            ],
            0..35,
        ),
    )
        .prop_map(
            |(pin, instance, workspace, port, mode, same_host, uppercase, name)| {
                let host = match mode {
                    0 => format!("node-{port}.example"),
                    1 => "127.0.0.1".into(),
                    _ => "[::1]".into(),
                };
                let host = if uppercase {
                    host.to_ascii_uppercase()
                } else {
                    host
                };
                let mtls_host = if same_host {
                    host.clone()
                } else {
                    match mode {
                        0 => "[::1]".into(),
                        1 => "clients.example".into(),
                        _ => "127.0.0.2".into(),
                    }
                };
                let mtls_port = if port == u16::MAX { port - 1 } else { port + 1 };
                LinkTarget {
                    https_origin: format!("https://{host}:{port}/"),
                    mtls_origin: format!("https://{mtls_host}:{mtls_port}/"),
                    instance_id: Uuid::from_u128(instance),
                    workspace_id: Uuid::from_u128(workspace),
                    ca_sha256: pin,
                    workspace_name: format!("Team{}end", name.into_iter().collect::<String>()),
                }
            },
        )
}
proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]
    #[test]
    fn valid_targets_roundtrip_and_canonicalization_is_idempotent(t in targets(), bytes in any::<[u8;32]>()) {
        let token=format!("ccp_join_{}",bytes.iter().map(|b|format!("{b:02x}")).collect::<String>());
        let link=EnrollmentLink::new(t,&token).unwrap();
        let encoded=link.to_uri();
        let parsed=EnrollmentLink::parse(&encoded).unwrap();
        prop_assert_eq!(parsed.target(),link.target());
        prop_assert_eq!(parsed.invitation(),token.as_str());
        prop_assert_eq!(parsed.to_uri(),encoded.clone());
        prop_assert!(!encoded.split('#').next().unwrap().contains(&token));
        prop_assert!(!encoded.contains(['\'','"',' ','\r','\n']));
        let https=url::Url::parse(&link.target().https_origin).unwrap();
        let mtls=url::Url::parse(&link.target().mtls_origin).unwrap();
        if https.host_str()==mtls.host_str() {
            let short=short_port_uri(&link);
            let parsed_short=EnrollmentLink::parse(&short).unwrap();
            prop_assert_eq!(parsed_short.target(),link.target());
            prop_assert_eq!(parsed_short.invitation(),token.as_str());
        }
    }
    #[test]
    fn every_generated_unknown_or_duplicate_field_is_refused(t in targets(), name in "[a-z]{2,18}", bytes in any::<[u8;32]>()) {
        let secret=format!("ccp_join_{}",bytes.iter().map(|b|format!("{b:02x}")).collect::<String>());
        let good=EnrollmentLink::new(t,&secret).unwrap().to_uri();
        let error=EnrollmentLink::parse(&format!("{good}&extra_{name}=value")).err().unwrap();
        prop_assert_eq!(error,LinkError::Fields);
        prop_assert!(!error.to_string().contains(&secret));
        for field in ["v","i","m","ca","t","w","n"] {
            prop_assert_eq!(EnrollmentLink::parse(&format!("{good}&{field}=ignored")).err(),Some(LinkError::Fields));
        }
    }
    #[test]
    fn arbitrary_input_is_rejected_privately_or_reencodes_to_a_valid_same_target(value in ".{0,1200}") {
        match EnrollmentLink::parse(&value) {
            Ok(link)=>{
                let rebuilt=EnrollmentLink::parse(&link.to_uri()).unwrap();
                prop_assert_eq!(link.target(),rebuilt.target());
                prop_assert_eq!(link.invitation(),rebuilt.invitation());
            }
            Err(error)=>{
                prop_assert!(!error.to_string().contains("ccp_join_"));
                let debug = format!("{error:?}");
                prop_assert!(!debug.contains("ccp_join_"));
            }
        }
    }
    #[test]
    fn mutations_of_valid_links_refuse_privately_or_reencode_without_semantic_drift(
        t in targets(), index in any::<u16>(), action in 0u8..4, byte in 0u8..=127,
    ) {
        let mut candidate=EnrollmentLink::new(t,&token()).unwrap().to_uri();
        let at=usize::from(index)%(candidate.len()+1);
        match action {
            0 if at<candidate.len()=>{candidate.remove(at);}
            1=>candidate.insert(at,char::from(byte)),
            2 if at<candidate.len()=>candidate.replace_range(at..at+1,&char::from(byte).to_string()),
            _=>candidate.insert_str(at,&format!("%{byte:02X}")),
        }
        match EnrollmentLink::parse(&candidate) {
            Ok(parsed)=>{
                let encoded=parsed.to_uri();
                let rebuilt=EnrollmentLink::parse(&encoded).unwrap();
                prop_assert_eq!(parsed.target(),rebuilt.target());
                prop_assert_eq!(parsed.invitation(),rebuilt.invitation());
                prop_assert_eq!(rebuilt.to_uri(),encoded);
            }
            Err(error)=>{
                prop_assert!(!error.to_string().contains(&candidate));
                prop_assert!(!error.to_string().contains("ccp_join_"));
            }
        }
    }

}

fn short_port_uri(link: &EnrollmentLink) -> String {
    let mtls = url::Url::parse(&link.target().mtls_origin).unwrap();
    let port = mtls.port_or_known_default().unwrap().to_string();
    let uri = link.to_uri();
    let fragment = uri.split_once('#').unwrap().1;
    let mut encoded = url::form_urlencoded::Serializer::new(String::new());
    for (key, value) in url::form_urlencoded::parse(fragment.as_bytes()) {
        encoded.append_pair(&key, if key == "m" { &port } else { &value });
    }
    format!("{}join#{}", link.target().https_origin, encoded.finish())
}

#[test]
fn shared_browser_and_rust_vectors_agree_on_acceptance_and_public_targets() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("../../testdata/enrollment_links.json")).unwrap();
    let invitation = format!(
        "ccp_join_{}",
        fixture["invitation_suffix"].as_str().unwrap()
    );
    for case in fixture["cases"].as_array().unwrap() {
        let uri = case["uri"]
            .as_str()
            .unwrap()
            .replace("{invitation}", &invitation);
        let parsed = EnrollmentLink::parse(&uri);
        assert_eq!(
            parsed.is_ok(),
            case["valid"].as_bool().unwrap(),
            "{}",
            case["name"]
        );
        if let Ok(link) = parsed {
            assert_eq!(
                link.target().https_origin,
                case["https_origin"].as_str().unwrap()
            );
            assert_eq!(
                link.target().mtls_origin,
                case["mtls_origin"].as_str().unwrap()
            );
            assert_eq!(
                link.target().workspace_name,
                case["workspace_name"].as_str().unwrap()
            );
            assert_eq!(link.invitation(), invitation);
            assert_eq!(
                link.target().instance_id.to_string(),
                case["i"].as_str().unwrap()
            );
            assert_eq!(
                link.target().workspace_id.to_string(),
                case["w"].as_str().unwrap()
            );
            let expected: [u8; 32] = serde_json::from_value(case["ca_bytes"].clone()).unwrap();
            assert_eq!(link.target().ca_sha256, expected);
            assert_eq!(
                link.to_uri(),
                case["canonical"]
                    .as_str()
                    .unwrap()
                    .replace("{invitation}", &invitation)
            );
        } else {
            let error = parsed.err().unwrap();
            assert_eq!(
                error_kind(error),
                case["error"].as_str().unwrap(),
                "{}",
                case["name"]
            );
            assert!(!error.to_string().contains(&invitation));
            assert!(!format!("{error:?}").contains(&invitation));
        }
    }
}
fn error_kind(error: LinkError) -> &'static str {
    match error {
        LinkError::Size => "size",
        LinkError::Syntax => "syntax",
        LinkError::Version => "version",
        LinkError::Origin => "origin",
        LinkError::Fields => "fields",
        LinkError::Identity => "identity",
        LinkError::Pin => "pin",
        LinkError::Invitation => "invitation",
        LinkError::WorkspaceName => "name",
    }
}

#[test]
fn unpadded_sha256_last_symbol_has_exactly_two_zero_pad_bits() {
    let alphabet = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let original = format!("ca=sha256%3A{}", "A".repeat(43));
    for (index, byte) in alphabet.iter().enumerate() {
        let encoded = format!("ca=sha256%3A{}{}", "A".repeat(42), char::from(*byte));
        let input = uri().replace(&original, &encoded);
        assert_eq!(EnrollmentLink::parse(&input).is_ok(), index % 4 == 0);
    }
}
