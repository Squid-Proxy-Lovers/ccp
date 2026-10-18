// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

#![expect(
    clippy::indexing_slicing,
    clippy::unreachable,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "test and benchmark support code: a panic here is a failed test, not a production crash"
)]

pub mod harness;
