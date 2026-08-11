use std::fmt;
use std::str::FromStr;

use hex::FromHexError;
use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use sha2::{Digest, Sha256};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct BlobHash([u8; 32]);

impl BlobHash {
    pub fn new(content: &[u8]) -> Self {
        let hash = Sha256::digest(content);
        let mut bytes = [0_u8; 32];
        bytes.copy_from_slice(&hash);
        Self(bytes)
    }
}

impl fmt::Display for BlobHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&hex::encode(self.0))
    }
}

impl FromStr for BlobHash {
    type Err = FromHexError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut bytes = [0_u8; 32];
        hex::decode_to_slice(s, &mut bytes)?;
        Ok(Self(bytes))
    }
}

impl Serialize for BlobHash {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for BlobHash {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        value.parse().map_err(D::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use crate::BlobHash;

    #[test]
    fn same_content_produces_same_blob_hash() {
        assert_eq!(BlobHash::new(b"hello"), BlobHash::new(b"hello"));
    }

    #[test]
    fn display_is_lowercase_sha256_hex() {
        assert_eq!(
            BlobHash::new(b"hello").to_string(),
            "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
        );
    }

    #[test]
    fn different_content_produces_different_blob_hashes() {
        assert_ne!(BlobHash::new(b"hello"), BlobHash::new(b"world"));
    }

    #[test]
    fn display_and_parse_round_trip() {
        let blob_hash = BlobHash::new(b"hello");
        let parsed: BlobHash = blob_hash.to_string().parse().unwrap();
        assert_eq!(parsed, blob_hash);
    }

    #[test]
    fn serde_round_trip() {
        let blob_hash = BlobHash::new(b"hello");
        let value = serde_json::to_value(blob_hash).unwrap();
        let parsed: BlobHash = serde_json::from_value(value).unwrap();
        assert_eq!(parsed, blob_hash);
    }

    #[test]
    fn parse_rejects_non_hex_blob_hashes() {
        let parsed = "not-a-blob-hash".parse::<BlobHash>();
        assert!(parsed.is_err());
    }
}
