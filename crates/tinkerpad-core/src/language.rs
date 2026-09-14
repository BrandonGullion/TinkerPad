//! Identifiers for languages.
//!
//! This is the only plugin concept in `tinkerpad-core`: the rest of the app
//! refers to a language by id, and only `tinkerpad-plugin` knows what a
//! plugin contains.

use serde::{Deserialize, Serialize};

/// A language plugin's id, such as `csharp` or `typescript`: lowercase letters,
/// digits, and hyphens, validated when a plugin loads.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct LanguageId(String);

impl LanguageId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for LanguageId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl AsRef<str> for LanguageId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl std::borrow::Borrow<str> for LanguageId {
    fn borrow(&self) -> &str {
        &self.0
    }
}

impl PartialEq<str> for LanguageId {
    fn eq(&self, other: &str) -> bool {
        self.0 == other
    }
}

impl PartialEq<&str> for LanguageId {
    fn eq(&self, other: &&str) -> bool {
        self.0 == *other
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serialises_as_a_plain_string() {
        let id = LanguageId::new("csharp");
        assert_eq!(serde_json::to_string(&id).unwrap(), r#""csharp""#);
        assert_eq!(
            serde_json::from_str::<LanguageId>(r#""rust""#).unwrap(),
            "rust"
        );
    }

    #[test]
    fn looks_up_by_str() {
        let mut map = std::collections::HashMap::new();
        map.insert(LanguageId::new("go"), 1);
        assert_eq!(map.get("go"), Some(&1));
    }
}
