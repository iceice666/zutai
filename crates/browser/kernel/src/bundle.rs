use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Portable input consumed by the browser-side semantic pipeline.
///
/// Paths always use `/`, are relative to the source root, and are validated by
/// `zutai_semantic::analyze_sources` before a bundle is emitted or loaded.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WebBundleV3 {
    pub format_version: u32,
    pub entry: String,
    pub sources: BTreeMap<String, String>,
    #[serde(default)]
    pub stdlib_compiler_compatibility: String,
    #[serde(default)]
    pub stdlib_sources: BTreeMap<String, String>,
    #[serde(default)]
    pub packages: zutai_semantic::PortablePackageGraph,
}

impl WebBundleV3 {
    pub const FORMAT_VERSION: u32 = 3;

    pub fn new(
        entry: String,
        sources: BTreeMap<String, String>,
        stdlib_compiler_compatibility: String,
        stdlib_sources: BTreeMap<String, String>,
        packages: zutai_semantic::PortablePackageGraph,
    ) -> Self {
        Self {
            format_version: Self::FORMAT_VERSION,
            entry,
            sources,
            stdlib_compiler_compatibility,
            stdlib_sources,
            packages,
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
        let packages = zutai_semantic::PortablePackageGraph {
            root_package: Some("app".to_owned()),
            packages: BTreeMap::from([(
                "app".to_owned(),
                zutai_semantic::PortablePackage {
                    source: zutai_semantic::PortablePackageSource::Path,
                    name: "app".to_owned(),
                    dependencies: BTreeMap::from([("math".to_owned(), "math".to_owned())]),
                    modules: BTreeMap::new(),
                    sources: BTreeMap::new(),
                },
            )]),
        };
        let bundle = WebBundleV3::new(
            "main.zt".to_owned(),
            sources,
            "0.1.0".to_owned(),
            BTreeMap::from([
                ("prelude".to_owned(), "1".to_owned()),
                ("stream".to_owned(), "1".to_owned()),
            ]),
            packages,
        );
        let json = serde_json::to_string(&bundle).unwrap();
        assert_eq!(serde_json::from_str::<WebBundleV3>(&json).unwrap(), bundle);
    }

    #[test]
    fn future_bundle_versions_are_rejected() {
        let bundle = WebBundleV3 {
            format_version: 1,
            entry: "main.zt".to_owned(),
            sources: BTreeMap::new(),
            stdlib_compiler_compatibility: String::new(),
            stdlib_sources: BTreeMap::new(),
            packages: zutai_semantic::PortablePackageGraph::default(),
        };
        assert!(bundle.validate_version().is_err());
    }

    #[test]
    fn version_one_json_reaches_precise_version_rejection() {
        let json = r#"{"format_version":1,"entry":"main.zt","sources":{"main.zt":"1\n"}}"#;
        let bundle: WebBundleV3 = serde_json::from_str(json).unwrap();
        let error = bundle.validate_version().unwrap_err();
        assert_eq!(error.found, 1);
        assert_eq!(error.supported, 3);
    }
}
