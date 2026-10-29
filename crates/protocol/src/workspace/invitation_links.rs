// SPDX-License-Identifier: AGPL-3.0-or-later
//! Explicit link issuance, separate from the existing finite invitation APIs.
use super::Role;
use serde::{Deserialize, Serialize};
#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Lifetime {
    Short {},
    At {
        expires_at: i64,
        #[serde(default)]
        confirm_long_lived: bool,
    },
    Never {
        confirm_no_expiry: bool,
    },
}
#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Admission {
    Individual {},
    InitialGroup { max_clients: Option<u32> },
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Issue {
    pub workspace_id: String,
    pub role: Role,
    #[serde(default)]
    pub admission: Admission,
    #[serde(default)]
    pub lifetime: Lifetime,
    /// Public HTTPS origin when an admin uses a local forward or rewritten Host.
    pub https_origin: Option<String>,
    /// Override the configured direct mTLS origin when the external port/host differs.
    pub mtls_origin: Option<String>,
}
/// Returned once. Contains a bearer in the fragment; never implicitly Debug/Clone.
#[derive(Serialize, Deserialize)]
pub struct Issued {
    pub invitation_id: String,
    pub enrollment_link: String,
    pub workspace_id: String,
    pub workspace_name: String,
    pub expires_at: Option<i64>,
    pub max_uses: Option<u64>,
}

impl Default for Lifetime {
    fn default() -> Self {
        Self::Short {}
    }
}
impl Default for Admission {
    fn default() -> Self {
        Self::Individual {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn empty_tagged_variants_reject_extra_fields_and_defaults_remain_finite() {
        let base = json!({"workspace_id":"workspace","role":"read"});
        let parsed: Issue = serde_json::from_value(base.clone()).unwrap();
        assert!(matches!(parsed.admission, Admission::Individual {}));
        assert!(matches!(parsed.lifetime, Lifetime::Short {}));
        for extra in [
            json!({"lifetime":{"kind":"short","confirm_no_expiry":true}}),
            json!({"admission":{"kind":"individual","max_clients":5}}),
            json!({"lifetime":{"kind":"never"}}),
            json!({"lifetime":{"kind":"at","expires_at":1234,"unexpected":true}}),
            json!({"extra":true}),
        ] {
            let mut value = base.clone();
            value
                .as_object_mut()
                .unwrap()
                .extend(extra.as_object().unwrap().clone());
            assert!(serde_json::from_value::<Issue>(value).is_err());
        }
    }
}
