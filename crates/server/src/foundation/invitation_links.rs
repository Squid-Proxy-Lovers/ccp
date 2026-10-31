// SPDX-License-Identifier: AGPL-3.0-or-later
use super::DataRole;
use anyhow::{Context, bail};
use uuid::Uuid;

pub const DEFAULT_LINK_LIFETIME_SECONDS: i64 = 30 * 60;
const LONG_LINK_SECONDS: i64 = 7 * 24 * 60 * 60;
#[derive(Clone, Copy, Debug, Default)]
pub enum LinkLifetime {
    #[default]
    Short,
    At {
        expires_at: i64,
        confirmed: bool,
    },
    Never {
        confirmed: bool,
    },
}
impl LinkLifetime {
    pub fn long_lived(self, now: i64) -> bool {
        match self {
            Self::Never { .. } => true,
            Self::At { expires_at, .. } => expires_at.saturating_sub(now) > LONG_LINK_SECONDS,
            Self::Short => false,
        }
    }
    pub fn expires_at(self, now: i64) -> anyhow::Result<Option<i64>> {
        if now < 0 {
            bail!("invalid server time");
        }
        match self {
            Self::Short => Ok(Some(
                now.checked_add(DEFAULT_LINK_LIFETIME_SECONDS)
                    .context("link expiry overflow")?,
            )),
            Self::At {
                expires_at,
                confirmed,
            } if expires_at > now => {
                if self.long_lived(now) && !confirmed {
                    bail!("long-lived links require explicit confirmation");
                }
                Ok(Some(expires_at))
            }
            Self::Never { confirmed: true } => Ok(None),
            Self::Never { confirmed: false } => {
                bail!("no-expiry links require explicit confirmation")
            }
            Self::At { .. } => bail!("link expiry must be in the future"),
        }
    }
}
#[derive(Clone, Copy, Debug, Default)]
pub enum LinkAdmission {
    #[default]
    Individual,
    InitialGroup {
        max_clients: Option<u32>,
    },
}
#[derive(Clone, Copy, Debug)]
pub struct LinkInvitationRequest {
    pub workspace_id: Uuid,
    pub role: DataRole,
    pub admission: LinkAdmission,
    pub lifetime: LinkLifetime,
}
