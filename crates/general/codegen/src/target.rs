use std::error::Error;
use std::fmt;
use std::str::FromStr;

const SUPPORTED_TARGETS: &str = "x86_64-unknown-linux-gnu, aarch64-unknown-linux-gnu, x86_64-apple-darwin, aarch64-apple-darwin";

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum NativeArch {
    X86_64,
    Aarch64,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum NativeOs {
    Linux,
    Macos,
}

/// A validated native compilation target.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct NativeTarget {
    arch: NativeArch,
    os: NativeOs,
}

impl NativeTarget {
    pub const X86_64_LINUX: Self = Self::new(NativeArch::X86_64, NativeOs::Linux);
    pub const AARCH64_LINUX: Self = Self::new(NativeArch::Aarch64, NativeOs::Linux);
    pub const X86_64_MACOS: Self = Self::new(NativeArch::X86_64, NativeOs::Macos);
    pub const AARCH64_MACOS: Self = Self::new(NativeArch::Aarch64, NativeOs::Macos);
    pub const SUPPORTED: [Self; 4] = [
        Self::X86_64_LINUX,
        Self::AARCH64_LINUX,
        Self::X86_64_MACOS,
        Self::AARCH64_MACOS,
    ];

    const fn new(arch: NativeArch, os: NativeOs) -> Self {
        Self { arch, os }
    }

    /// Resolve the build host without falling back to an unrelated target.
    pub fn host() -> Result<Self, NativeTargetError> {
        Self::from_arch_os(std::env::consts::ARCH, std::env::consts::OS)
    }

    /// Resolve a Rust architecture/OS pair into the supported native target set.
    pub fn from_arch_os(arch: &str, os: &str) -> Result<Self, NativeTargetError> {
        match (arch, os) {
            ("x86_64", "linux") => Ok(Self::X86_64_LINUX),
            ("aarch64", "linux") => Ok(Self::AARCH64_LINUX),
            ("x86_64", "macos") => Ok(Self::X86_64_MACOS),
            ("aarch64", "macos") => Ok(Self::AARCH64_MACOS),
            _ => Err(NativeTargetError::UnsupportedHost {
                arch: arch.to_owned(),
                os: os.to_owned(),
            }),
        }
    }

    pub const fn arch(self) -> NativeArch {
        self.arch
    }

    pub const fn os(self) -> NativeOs {
        self.os
    }

    pub const fn triple(self) -> &'static str {
        match (self.arch, self.os) {
            (NativeArch::X86_64, NativeOs::Linux) => "x86_64-unknown-linux-gnu",
            (NativeArch::Aarch64, NativeOs::Linux) => "aarch64-unknown-linux-gnu",
            (NativeArch::X86_64, NativeOs::Macos) => "x86_64-apple-darwin",
            (NativeArch::Aarch64, NativeOs::Macos) => "aarch64-apple-darwin",
        }
    }

    pub const fn data_layout(self) -> &'static str {
        match (self.arch, self.os) {
            (NativeArch::X86_64, NativeOs::Linux) => {
                "e-m:e-p270:32:32-p271:32:32-p272:64:64-i64:64-i128:128-f80:128-n8:16:32:64-S128"
            }
            (NativeArch::Aarch64, NativeOs::Linux) => {
                "e-m:e-p270:32:32-p271:32:32-p272:64:64-i8:8:32-i16:16:32-i64:64-i128:128-n32:64-S128-Fn32"
            }
            (NativeArch::X86_64, NativeOs::Macos) => {
                "e-m:o-p270:32:32-p271:32:32-p272:64:64-i64:64-i128:128-f80:128-n8:16:32:64-S128"
            }
            (NativeArch::Aarch64, NativeOs::Macos) => {
                "e-m:o-p270:32:32-p271:32:32-p272:64:64-i64:64-i128:128-n32:64-S128-Fn32"
            }
        }
    }

    pub const fn shared_library_extension(self) -> &'static str {
        match self.os {
            NativeOs::Linux => ".so",
            NativeOs::Macos => ".dylib",
        }
    }
}

impl fmt::Display for NativeTarget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.triple())
    }
}

impl FromStr for NativeTarget {
    type Err = NativeTargetError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "x86_64-unknown-linux-gnu" => Ok(Self::X86_64_LINUX),
            "aarch64-unknown-linux-gnu" => Ok(Self::AARCH64_LINUX),
            "x86_64-apple-darwin" => Ok(Self::X86_64_MACOS),
            "aarch64-apple-darwin" => Ok(Self::AARCH64_MACOS),
            _ => Err(NativeTargetError::UnsupportedTriple(value.to_owned())),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum NativeTargetError {
    UnsupportedTriple(String),
    UnsupportedHost { arch: String, os: String },
}

impl fmt::Display for NativeTargetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedTriple(triple) => write!(
                f,
                "unsupported native target `{triple}`; supported targets: {SUPPORTED_TARGETS}"
            ),
            Self::UnsupportedHost { arch, os } => write!(
                f,
                "unsupported native host `{arch}-{os}`; pass --target with one of: {SUPPORTED_TARGETS}"
            ),
        }
    }
}

impl Error for NativeTargetError {}
