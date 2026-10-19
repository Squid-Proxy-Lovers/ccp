// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::parse_content_length;

#[test]
fn content_length_header_is_case_insensitive() {
    let headers = b"POST /auth HTTP/1.1\r\nHost: localhost\r\nContent-LENGTH:  42 \r\n\r\n";
    assert_eq!(parse_content_length(headers), Some(42));
}

#[test]
fn content_length_ignores_other_and_malformed_headers() {
    assert_eq!(parse_content_length(b"Host: localhost\r\n\r\n"), None);
    assert_eq!(parse_content_length(b"Content-Length: many\r\n\r\n"), None);
    assert_eq!(parse_content_length(b"X-\xc4\xb0: 1\r\n\r\n"), None);
    assert_eq!(parse_content_length(b"\xff\r\n"), None);
}
