// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;

pub(super) fn validate(
    profile: &WorkspaceProfile,
    value: &Delivery,
    workspace: Option<&str>,
) -> anyhow::Result<()> {
    metadata(value, workspace)?;
    let context = &value.context;
    let credential = profile
        .credential
        .as_ref()
        .context("guidance context requires an enrolled profile")?;
    binding(
        context,
        &profile.trust.instance_id,
        &credential.receipt.client_id,
        &credential.receipt.credential_id,
        workspace,
    )
}

fn metadata(value: &Delivery, workspace: Option<&str>) -> anyhow::Result<()> {
    super::validate(&value.guidance, workspace)?;
    check(value.format == 1)
}

fn binding(
    context: &DeliveryContext,
    instance: &str,
    client: &str,
    credential: &str,
    workspace: Option<&str>,
) -> anyhow::Result<()> {
    for id in [
        &context.instance_id,
        &context.client_id,
        &context.credential_id,
    ] {
        identity(id)?;
    }
    check(
        context.instance_id == instance
            && context.client_id == client
            && context.credential_id == credential,
    )?;
    match (workspace, context.workspace_grant_id.as_deref()) {
        (Some(_), Some(grant)) => identity(grant),
        (None, None) => Ok(()),
        _ => check(false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};
    const INSTANCE: &str = "11111111-1111-4111-8111-111111111111";
    const CLIENT: &str = "22222222-2222-4222-8222-222222222222";
    const CREDENTIAL: &str = "33333333-3333-4333-8333-333333333333";
    fn context() -> Value {
        json!({"instance_id":INSTANCE,"client_id":CLIENT,"credential_id":CREDENTIAL,"workspace_grant_id":"44444444-4444-4444-8444-444444444444"})
    }
    #[test]
    fn delivery_binding_must_match_verified_profile_and_grant_presence() {
        let good = context();
        let parsed: DeliveryContext = serde_json::from_value(good.clone()).unwrap();
        binding(&parsed, INSTANCE, CLIENT, CREDENTIAL, Some(INSTANCE)).unwrap();
        assert!(binding(&parsed, INSTANCE, CLIENT, CREDENTIAL, None).is_err());
        for field in [
            "instance_id",
            "client_id",
            "credential_id",
            "workspace_grant_id",
        ] {
            for bad in [json!("PRIVATE_GUIDANCE"), json!(Uuid::nil().to_string())] {
                let mut value = good.clone();
                value[field] = bad;
                let typed = serde_json::from_value::<DeliveryContext>(value).unwrap();
                let error = binding(&typed, INSTANCE, CLIENT, CREDENTIAL, Some(INSTANCE))
                    .unwrap_err()
                    .to_string();
                assert!(!error.contains("PRIVATE_GUIDANCE"));
            }
        }
        for field in ["instance_id", "client_id", "credential_id"] {
            let mut value = good.clone();
            value[field] = json!("55555555-5555-4555-8555-555555555555");
            assert!(
                binding(
                    &serde_json::from_value(value).unwrap(),
                    INSTANCE,
                    CLIENT,
                    CREDENTIAL,
                    Some(INSTANCE)
                )
                .is_err()
            );
        }
        let mut value = good;
        value["workspace_grant_id"] = Value::Null;
        let typed: DeliveryContext = serde_json::from_value(value).unwrap();
        binding(&typed, INSTANCE, CLIENT, CREDENTIAL, None).unwrap();
        assert!(binding(&typed, INSTANCE, CLIENT, CREDENTIAL, Some(INSTANCE)).is_err());
    }
    #[test]
    fn delivery_metadata_requires_supported_format_before_use() {
        let value = json!({"format":1,"context":context(),"guidance":{"precedence":"global_first","global":{"scope":{"kind":"global"},"revision":0,"text":"","publisher":null},"workspace":null}});
        let mut typed: Delivery = serde_json::from_value(value).unwrap();
        metadata(&typed, None).unwrap();
        typed.format = 2;
        assert!(metadata(&typed, None).is_err());
        typed.format = 1;
        typed.guidance.global.text = "PRIVATE_GUIDANCE".into();
        assert!(
            !metadata(&typed, None)
                .unwrap_err()
                .to_string()
                .contains("PRIVATE_GUIDANCE")
        );
    }
}
