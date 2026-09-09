//! Credential references declared in `metadata.fabro.credentials`.

use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, de};

/// Where one provider secret comes from.
///
/// A provider lists these in order; the first that resolves wins.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CredentialRef {
    /// A token or OAuth entry in the Fabro vault.
    Vault(String),
    /// A process environment variable.
    Env(String),
    /// The AWS default credential chain. Resolves without a secret; the
    /// Bedrock adapter signs each request.
    AwsSigv4,
}

impl std::fmt::Display for CredentialRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Vault(name) => write!(f, "vault:{name}"),
            Self::Env(name) => write!(f, "env:{name}"),
            Self::AwsSigv4 => f.write_str("aws_sigv4"),
        }
    }
}

impl FromStr for CredentialRef {
    type Err = CredentialRefParseError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if value == "aws_sigv4" {
            return Ok(Self::AwsSigv4);
        }
        if let Some(name) = value.strip_prefix("vault:") {
            return if name.is_empty() {
                Err(CredentialRefParseError::EmptyVault)
            } else {
                Ok(Self::Vault(name.to_string()))
            };
        }
        if let Some(name) = value.strip_prefix("env:") {
            return if name.is_empty() {
                Err(CredentialRefParseError::EmptyEnv)
            } else {
                Ok(Self::Env(name.to_string()))
            };
        }
        Err(CredentialRefParseError::Invalid)
    }
}

impl Serialize for CredentialRef {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for CredentialRef {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = String::deserialize(deserializer)?;
        value.parse().map_err(de::Error::custom)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum CredentialRefParseError {
    #[error("credential reference must be `vault:<name>`, `env:<NAME>`, or `aws_sigv4`")]
    Invalid,
    #[error("credential reference is missing a name after `vault:`")]
    EmptyVault,
    #[error("credential reference is missing a name after `env:`")]
    EmptyEnv,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_each_form() {
        assert_eq!(
            "vault:OPENAI_CODEX".parse::<CredentialRef>().unwrap(),
            CredentialRef::Vault("OPENAI_CODEX".into())
        );
        assert_eq!(
            "env:KIMI_API_KEY".parse::<CredentialRef>().unwrap(),
            CredentialRef::Env("KIMI_API_KEY".into())
        );
        assert_eq!(
            "aws_sigv4".parse::<CredentialRef>().unwrap(),
            CredentialRef::AwsSigv4
        );
    }

    #[test]
    fn rejects_literal_secrets_without_echoing_them() {
        let err = "sk-ant-1234".parse::<CredentialRef>().unwrap_err();
        assert_eq!(err, CredentialRefParseError::Invalid);
        assert!(!err.to_string().contains("sk-ant"));
        assert_eq!(
            "vault:".parse::<CredentialRef>().unwrap_err(),
            CredentialRefParseError::EmptyVault
        );
        assert_eq!(
            "env:".parse::<CredentialRef>().unwrap_err(),
            CredentialRefParseError::EmptyEnv
        );
    }

    #[test]
    fn round_trips_through_serde_strings() {
        let value: Vec<CredentialRef> =
            serde_json::from_str(r#"["env:A", "vault:b", "aws_sigv4"]"#).unwrap();
        assert_eq!(
            serde_json::to_string(&value).unwrap(),
            r#"["env:A","vault:b","aws_sigv4"]"#
        );
        assert!(serde_json::from_str::<Vec<CredentialRef>>(r#"["sk-literal"]"#).is_err());
    }
}
