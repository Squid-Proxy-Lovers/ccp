// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;
use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use uuid::Uuid;

pub(super) const MAX_AGE: u64 = 8 * 60 * 60;
const TOTAL: usize = 1024;
const PER_KEY: usize = 32;
const PER_WORKSPACE: usize = 128;
const INSTANCE_RESERVE: usize = 64;
struct Record {
    key_id: Uuid,
    // A quota partition only; current scope always comes from the live key.
    workspace: Option<Uuid>,
    origin: String,
    deadline: Instant,
    expires_at: i64,
}

pub(in crate::workspace_http) struct Sessions {
    secret: [u8; 32],
    records: Mutex<HashMap<[u8; 32], Record>>,
}
pub(super) struct Resolved {
    pub key_id: Uuid,
    pub expires_at: i64,
    pub csrf: String,
}
pub(super) struct Issued {
    pub token: String,
    pub session: Resolved,
}

impl Sessions {
    pub(in crate::workspace_http) fn new() -> Result<Self, Error> {
        Ok(Self {
            secret: random()?,
            records: Mutex::new(HashMap::new()),
        })
    }
    pub(super) fn issue(
        &self,
        key_id: Uuid,
        workspace: Option<Uuid>,
        origin: &str,
        previous: Option<&str>,
        now: i64,
    ) -> Result<Issued, Error> {
        let token = hex(&random()?);
        let digest = hash(&token);
        let expires_at = now
            .checked_add(i64::try_from(MAX_AGE).map_err(|_invalid| Error::Unavailable)?)
            .filter(|_| now >= 0)
            .ok_or(Error::Unavailable)?;
        let at = Instant::now();
        let deadline = at
            .checked_add(Duration::from_secs(MAX_AGE))
            .ok_or(Error::Unavailable)?;
        let mut records = self
            .records
            .lock()
            .map_err(|_poisoned| Error::Unavailable)?;
        records.retain(|_, r| r.deadline > at && r.expires_at > now);
        let old = previous
            .map(hash)
            .filter(|h| records.get(h).is_some_and(|r| r.origin == origin));
        let count = records
            .iter()
            .filter(|(h, r)| r.key_id == key_id && Some(**h) != old)
            .count();
        let size = records.len().saturating_sub(usize::from(old.is_some()));
        let scoped = records
            .iter()
            .filter(|(h, r)| r.workspace == workspace && Some(**h) != old)
            .count();
        if size >= TOTAL
            || count >= PER_KEY
            || records.contains_key(&digest)
            || (workspace.is_some()
                && (size >= TOTAL - INSTANCE_RESERVE || scoped >= PER_WORKSPACE))
        {
            return Err(Error::Unavailable);
        }
        let csrf = self.csrf(&token)?;
        if let Some(old) = old {
            records.remove(&old);
        }
        records.insert(
            digest,
            Record {
                key_id,
                workspace,
                origin: origin.to_owned(),
                deadline,
                expires_at,
            },
        );
        Ok(Issued {
            token,
            session: Resolved {
                key_id,
                expires_at,
                csrf,
            },
        })
    }
    pub(super) fn resolve(
        &self,
        token: &str,
        origin: &str,
        proof: Option<&str>,
    ) -> Result<Resolved, Error> {
        valid(token)?;
        let now = crate::instance_tls::now().map_err(|_clock| Error::Unavailable)?;
        if let Some(proof) = proof {
            self.mac(token)?
                .verify_slice(&decode(proof)?)
                .map_err(|_mismatch| Error::Forbidden)?;
        }
        let mut records = self
            .records
            .lock()
            .map_err(|_poisoned| Error::Unavailable)?;
        let at = Instant::now();
        records.retain(|_, r| r.deadline > at && r.expires_at > now);
        let record = records
            .get(&hash(token))
            .filter(|r| r.origin == origin)
            .ok_or(Error::Unauthorized)?;
        Ok(Resolved {
            key_id: record.key_id,
            expires_at: record.expires_at,
            csrf: self.csrf(token)?,
        })
    }
    pub(super) fn remove(&self, token: &str) -> Result<(), Error> {
        self.records
            .lock()
            .map_err(|_poisoned| Error::Unavailable)?
            .remove(&hash(token));
        Ok(())
    }
    fn mac(&self, token: &str) -> Result<Hmac<Sha256>, Error> {
        let mut mac =
            Hmac::<Sha256>::new_from_slice(&self.secret).map_err(|_invalid| Error::Unavailable)?;
        mac.update(b"ccp-browser-csrf-v1\0");
        mac.update(token.as_bytes());
        Ok(mac)
    }
    fn csrf(&self, token: &str) -> Result<String, Error> {
        Ok(hex(&self.mac(token)?.finalize().into_bytes()))
    }
}
fn hash(token: &str) -> [u8; 32] {
    Sha256::digest(token.as_bytes()).into()
}
fn random() -> Result<[u8; 32], Error> {
    let mut bytes = [0; 32];
    getrandom::getrandom(&mut bytes).map_err(|_entropy| Error::Unavailable)?;
    Ok(bytes)
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
fn valid(token: &str) -> Result<(), Error> {
    if token.len() != 64
        || !token
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(Error::Unauthorized);
    }
    Ok(())
}
fn decode(proof: &str) -> Result<Vec<u8>, Error> {
    valid(proof).map_err(|_invalid| Error::Forbidden)?;
    proof
        .as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| {
            let pair = std::str::from_utf8(pair).map_err(|_encoding| Error::Forbidden)?;
            u8::from_str_radix(pair, 16).map_err(|_hex| Error::Forbidden)
        })
        .collect()
}

#[cfg(test)]
mod tests;
