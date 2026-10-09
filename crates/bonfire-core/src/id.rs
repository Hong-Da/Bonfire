//! 对象 ID：BLAKE3 哈希，文本形式为 `b3:<64 位十六进制>`。

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

pub const PREFIX: &str = "b3:";

#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ObjectId([u8; 32]);

impl ObjectId {
    pub fn of(bytes: &[u8]) -> Self {
        Self(*blake3::hash(bytes).as_bytes())
    }

    pub fn from_hash(h: blake3::Hash) -> Self {
        Self(*h.as_bytes())
    }

    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// 64 位十六进制，不带前缀。
    pub fn to_hex(&self) -> String {
        hex::encode(self.0)
    }

    /// 前 n 位十六进制，用于显示。
    pub fn short(&self, n: usize) -> String {
        let mut h = self.to_hex();
        h.truncate(n.min(64));
        h
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("无效的对象 ID：{0}")]
pub struct ParseIdError(pub String);

impl FromStr for ObjectId {
    type Err = ParseIdError;

    /// 接受 `b3:<hex>` 或纯 64 位十六进制。
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let h = s.strip_prefix(PREFIX).unwrap_or(s);
        let mut out = [0u8; 32];
        hex::decode_to_slice(h, &mut out).map_err(|_| ParseIdError(s.to_string()))?;
        Ok(Self(out))
    }
}

impl fmt::Display for ObjectId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{PREFIX}{}", self.to_hex())
    }
}

impl fmt::Debug for ObjectId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{PREFIX}{}", self.short(12))
    }
}

impl Serialize for ObjectId {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for ObjectId {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_parse_roundtrip() {
        let id = ObjectId::of(b"hello");
        let s = id.to_string();
        assert!(s.starts_with("b3:"));
        assert_eq!(s.len(), 3 + 64);
        assert_eq!(s.parse::<ObjectId>().unwrap(), id);
        assert_eq!(id.to_hex().parse::<ObjectId>().unwrap(), id);
    }

    #[test]
    fn known_vector() {
        // BLAKE3 空输入的标准测试向量。
        assert_eq!(
            ObjectId::of(b"").to_hex(),
            "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262"
        );
    }

    #[test]
    fn rejects_bad_input() {
        assert!("b3:xyz".parse::<ObjectId>().is_err());
        assert!("b3:abcd".parse::<ObjectId>().is_err());
    }

    #[test]
    fn serde_as_string() {
        let id = ObjectId::of(b"x");
        let json = serde_json::to_string(&id).unwrap();
        assert_eq!(json, format!("\"{id}\""));
        assert_eq!(serde_json::from_str::<ObjectId>(&json).unwrap(), id);
    }
}
