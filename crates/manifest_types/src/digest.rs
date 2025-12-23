use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;

#[derive(thiserror::Error, Debug)]
pub enum DigestError {
    #[error("invalid hex digest: {0}")]
    InvalidHex(String),
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Md5Digest {
    inner: [u8; 16],
}

impl Md5Digest {
    pub fn from_bytes(inner: [u8; 16]) -> Self {
        Self { inner }
    }

    pub fn as_bytes(&self) -> &[u8; 16] {
        &self.inner
    }

    pub fn to_hex_upper(&self) -> String {
        hex::encode_upper(self.inner)
    }

    pub fn parse_hex(s: &str) -> Result<Self, DigestError> {
        let mut buf = [0u8; 16];
        hex::decode_to_slice(s, &mut buf).map_err(|_| DigestError::InvalidHex(s.to_string()))?;
        Ok(Self { inner: buf })
    }
}

impl fmt::Debug for Md5Digest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("Md5Digest")
            .field(&self.to_hex_upper())
            .finish()
    }
}

impl Serialize for Md5Digest {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_hex_upper())
    }
}

impl<'de> Deserialize<'de> for Md5Digest {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        Self::parse_hex(&s).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_hex_accepts_valid_md5() {
        let d = Md5Digest::parse_hex("0123456789ABCDEF0123456789ABCDEF").unwrap();
        assert_eq!(d.to_hex_upper(), "0123456789ABCDEF0123456789ABCDEF");
    }

    #[test]
    fn parse_hex_rejects_invalid() {
        let err = Md5Digest::parse_hex("not-hex").unwrap_err();
        match err {
            DigestError::InvalidHex(s) => assert_eq!(s, "not-hex"),
        }
    }
}
