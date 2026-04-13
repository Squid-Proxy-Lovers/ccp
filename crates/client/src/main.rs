// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    client::run_cli().await
}
