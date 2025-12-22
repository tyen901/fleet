use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Md5Digest(#[serde(with = "hex_serde")] pub [u8; 16]);

impl Md5Digest {
    pub fn md5_bytes(bytes: &[u8]) -> Self {
        let d = md5::compute(bytes);
        Self(d.0)
    }

    pub fn parse_hex(s: &str) -> Result<Self> {
        let s = s.trim();
        if s.len() != 32 {
            anyhow::bail!("md5 hex must be 32 chars, got {}", s.len());
        }
        let mut out = [0u8; 16];
        hex::decode_to_slice(s, &mut out).map_err(|_| anyhow::anyhow!("invalid hex"))?;
        Ok(Self(out))
    }

    pub fn to_hex_upper(&self) -> String {
        hex::encode_upper(self.0)
    }

    pub fn as_bytes(&self) -> &[u8; 16] {
        &self.0
    }
}

impl fmt::Debug for Md5Digest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Md5Digest({})", self.to_hex_upper())
    }
}

impl fmt::Display for Md5Digest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_hex_upper())
    }
}

mod hex_serde {
    use super::Md5Digest;
    use serde::Deserialize as _;
    use serde::{Deserializer, Serializer};

    pub fn serialize<S>(bytes: &[u8; 16], ser: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let d = Md5Digest(*bytes);
        ser.serialize_str(&d.to_hex_upper())
    }

    pub fn deserialize<'de, D>(de: D) -> Result<[u8; 16], D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(de)?;
        let d = Md5Digest::parse_hex(&s).map_err(serde::de::Error::custom)?;
        Ok(*d.as_bytes())
    }
}
