// SPDX-License-Identifier: AGPL-3.0-or-later
use serde::{Deserialize, Deserializer, Serializer, de::Error};

pub(super) mod integer {
    use super::*;
    pub fn serialize<S: Serializer>(value: &i64, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&value.to_string())
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<i64, D::Error> {
        let text = String::deserialize(deserializer)?;
        let value: i64 = text
            .parse()
            .map_err(|_error| D::Error::custom("invalid native integer"))?;
        if value.to_string() != text {
            return Err(D::Error::custom("noncanonical native integer"));
        }
        Ok(value)
    }
}
pub(super) mod real {
    use super::*;
    pub fn serialize<S: Serializer>(value: &u64, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&format!("{value:016x}"))
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<u64, D::Error> {
        let text = String::deserialize(deserializer)?;
        let value = u64::from_str_radix(&text, 16)
            .map_err(|_error| D::Error::custom("invalid native real bits"))?;
        if format!("{value:016x}") != text {
            return Err(D::Error::custom("noncanonical native real bits"));
        }
        Ok(value)
    }
}
