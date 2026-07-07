//! Embedded standard-library sources and module metadata for Zutai general mode.
//!
//! This crate owns the canonical `.zt` source text for `import stdlib.<name>`.
//! Compiler layers consume the registry instead of keeping their own parallel
//! module lists.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StdlibVisibility {
    Ambient,
    Explicit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StdlibModule {
    pub name: &'static str,
    pub source: &'static str,
    pub visibility: StdlibVisibility,
}

pub const STREAM_MODULE_SRC: &str = include_str!("modules/stream.zt");
pub const PRELUDE_MODULE_SRC: &str = include_str!("modules/prelude.zt");
pub const OPTIONAL_MODULE_SRC: &str = include_str!("modules/optional.zt");
pub const RESULT_MODULE_SRC: &str = include_str!("modules/result.zt");
pub const NUM_MODULE_SRC: &str = include_str!("modules/num.zt");
pub const TEXT_MODULE_SRC: &str = include_str!("modules/text.zt");
pub const CMP_MODULE_SRC: &str = include_str!("modules/cmp.zt");
pub const CONFIG_MODULE_SRC: &str = include_str!("modules/config.zt");
pub const REFLECT_MODULE_SRC: &str = include_str!("modules/reflect.zt");
pub const LIST_MODULE_SRC: &str = include_str!("modules/list.zt");
pub const DATA_MODULE_SRC: &str = include_str!("modules/data.zt");
pub const VALIDATE_MODULE_SRC: &str = include_str!("modules/validate.zt");
pub const FS_MODULE_SRC: &str = include_str!("modules/fs.zt");

pub const MODULES: &[StdlibModule] = &[
    StdlibModule {
        name: "stream",
        source: STREAM_MODULE_SRC,
        visibility: StdlibVisibility::Ambient,
    },
    StdlibModule {
        name: "prelude",
        source: PRELUDE_MODULE_SRC,
        visibility: StdlibVisibility::Ambient,
    },
    StdlibModule {
        name: "optional",
        source: OPTIONAL_MODULE_SRC,
        visibility: StdlibVisibility::Explicit,
    },
    StdlibModule {
        name: "result",
        source: RESULT_MODULE_SRC,
        visibility: StdlibVisibility::Explicit,
    },
    StdlibModule {
        name: "num",
        source: NUM_MODULE_SRC,
        visibility: StdlibVisibility::Explicit,
    },
    StdlibModule {
        name: "text",
        source: TEXT_MODULE_SRC,
        visibility: StdlibVisibility::Explicit,
    },
    StdlibModule {
        name: "cmp",
        source: CMP_MODULE_SRC,
        visibility: StdlibVisibility::Explicit,
    },
    StdlibModule {
        name: "config",
        source: CONFIG_MODULE_SRC,
        visibility: StdlibVisibility::Explicit,
    },
    StdlibModule {
        name: "reflect",
        source: REFLECT_MODULE_SRC,
        visibility: StdlibVisibility::Explicit,
    },
    StdlibModule {
        name: "list",
        source: LIST_MODULE_SRC,
        visibility: StdlibVisibility::Explicit,
    },
    StdlibModule {
        name: "data",
        source: DATA_MODULE_SRC,
        visibility: StdlibVisibility::Explicit,
    },
    StdlibModule {
        name: "validate",
        source: VALIDATE_MODULE_SRC,
        visibility: StdlibVisibility::Explicit,
    },
    StdlibModule {
        name: "fs",
        source: FS_MODULE_SRC,
        visibility: StdlibVisibility::Explicit,
    },
];

pub fn modules() -> &'static [StdlibModule] {
    MODULES
}

pub fn module(name: &str) -> Option<&'static StdlibModule> {
    MODULES.iter().find(|module| module.name == name)
}

pub fn source(name: &str) -> Option<&'static str> {
    module(name).map(|module| module.source)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL_MODULES: &[&str] = &[
        "stream", "prelude", "optional", "result", "num", "text", "cmp", "config", "reflect",
        "list", "data", "validate", "fs",
    ];

    #[test]
    fn registry_contains_current_modules() {
        let names: Vec<_> = modules().iter().map(|module| module.name).collect();
        assert_eq!(names, ALL_MODULES);
        for name in ALL_MODULES {
            assert!(source(name).is_some(), "missing stdlib source for {name}");
        }
    }

    #[test]
    fn ambient_visibility_is_limited_to_stream_and_prelude() {
        for module in modules() {
            let expected = match module.name {
                "stream" | "prelude" => StdlibVisibility::Ambient,
                _ => StdlibVisibility::Explicit,
            };
            assert_eq!(
                module.visibility, expected,
                "unexpected visibility for {}",
                module.name
            );
        }
    }

    #[test]
    fn unknown_module_lookup_returns_none() {
        assert!(module("nope").is_none());
        assert!(source("nope").is_none());
    }
}
