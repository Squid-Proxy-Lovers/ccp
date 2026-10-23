// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Bounded WebSocket byte carrier for an independently authenticated inner TLS session.
//! This module supplies no authentication, routing or TLS trust policy.
mod stream;
pub use stream::ByteStream;
pub use tokio_tungstenite as websocket;
use websocket::tungstenite::protocol::WebSocketConfig;

pub const ENROLL_PATH: &str = "/ccp/tunnel/v1/enroll";
pub const CLIENT_PATH: &str = "/ccp/tunnel/v1/client";
pub const SUBPROTOCOL: &str = "ccp-tls-v1";
pub const FRAME_LIMIT: usize = 64 * 1024;
const WRITE_CHUNK: usize = 16 * 1024;

/// Binary messages are independently bounded; larger TLS/HTTP bodies stream across them.
/// No compression is enabled. Handshake/connection deadlines belong to the caller.
pub fn websocket_config() -> WebSocketConfig {
    WebSocketConfig::default()
        .read_buffer_size(8192)
        .write_buffer_size(WRITE_CHUNK)
        .max_write_buffer_size(128 * 1024)
        .max_frame_size(Some(FRAME_LIMIT))
        .max_message_size(Some(FRAME_LIMIT))
        .accept_unmasked_frames(false)
}
