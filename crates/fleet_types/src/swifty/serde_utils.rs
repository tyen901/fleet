use relative_path::RelativePathBuf;
use serde::{Deserialize, Deserializer};
use std::fmt;

pub fn deserialize_relpath<'de, D>(deserializer: D) -> Result<RelativePathBuf, D::Error>
where
    D: Deserializer<'de>,
{
    let s = String::deserialize(deserializer)?;
    let normalized = s.replace('\\', "/");
    Ok(RelativePathBuf::from(normalized))
}

pub fn deserialize_u16_string_or_number<'de, D>(deserializer: D) -> Result<u16, D::Error>
where
    D: Deserializer<'de>,
{
    struct V;

    impl<'de> serde::de::Visitor<'de> for V {
        type Value = u16;

        fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "u16 or string containing a u16")
        }

        fn visit_u64<E: serde::de::Error>(self, v: u64) -> Result<u16, E> {
            u16::try_from(v).map_err(|_| E::custom(format!("port out of range for u16: {v}")))
        }

        fn visit_i64<E: serde::de::Error>(self, v: i64) -> Result<u16, E> {
            if v < 0 {
                return Err(E::custom(format!("port out of range for u16: {v}")));
            }
            u16::try_from(v as u64)
                .map_err(|_| E::custom(format!("port out of range for u16: {v}")))
        }

        fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<u16, E> {
            v.parse::<u16>()
                .map_err(|e| E::custom(format!("invalid port string {v:?}: {e}")))
        }

        fn visit_string<E: serde::de::Error>(self, v: String) -> Result<u16, E> {
            self.visit_str(&v)
        }
    }

    deserializer.deserialize_any(V)
}
