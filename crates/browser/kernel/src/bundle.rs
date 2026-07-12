use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Portable input consumed by the browser-side semantic pipeline.
///
/// Paths always use `/`, are relative to the source root, and are validated by
/// `zutai_semantic::analyze_sources` before a bundle is emitted or loaded.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WebBundleV1 {
    pub format_version: u32,
    pub entry: String,
    pub sources: BTreeMap<String, String>,
}

impl WebBundleV1 {
    pub const FORMAT_VERSION: u32 = 1;

    pub fn new(entry: String, sources: BTreeMap<String, String>) -> Self {
        Self {
            format_version: Self::FORMAT_VERSION,
            entry,
            sources,
        }
    }

    pub fn validate_version(&self) -> Result<(), BundleVersionError> {
        if self.format_version == Self::FORMAT_VERSION {
            Ok(())
        } else {
            Err(BundleVersionError {
                found: self.format_version,
                supported: Self::FORMAT_VERSION,
            })
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("unsupported Zutai web bundle version {found}; this kernel supports {supported}")]
pub struct BundleVersionError {
    pub found: u32,
    pub supported: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundle_schema_round_trips_deterministically() {
        let mut sources = BTreeMap::new();
        sources.insert("main.zt".to_owned(), "1\n".to_owned());
        let bundle = WebBundleV1::new("main.zt".to_owned(), sources);
        let json = serde_json::to_string(&bundle).unwrap();
        assert_eq!(
            json,
            r#"{"format_version":1,"entry":"main.zt","sources":{"main.zt":"1\n"}}"#
        );
        assert_eq!(serde_json::from_str::<WebBundleV1>(&json).unwrap(), bundle);
    }

    #[test]
    fn future_bundle_versions_are_rejected() {
        let bundle = WebBundleV1 {
            format_version: 2,
            entry: "main.zt".to_owned(),
            sources: BTreeMap::new(),
        };
        assert!(bundle.validate_version().is_err());
    }
}
