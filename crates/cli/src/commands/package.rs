use std::error::Error;
use std::path::PathBuf;

use clap::Subcommand;
use zutai_package::acquire::{AcquireOptions, Operation};

#[derive(Clone, Debug, Subcommand)]
pub(crate) enum PackageCommand {
    /// Resolve manifests, rewrite the lock deterministically, and fill the cache
    Sync {
        /// Package root or any path inside the root package
        #[arg(default_value = ".")]
        path: PathBuf,
        /// Refuse all network access and use only cached Git objects/snapshots
        #[arg(long)]
        offline: bool,
        /// Local transport override for deterministic package fixtures
        #[arg(long, value_names = ["URL", "PATH"], num_args = 2, hide = true)]
        transport_override: Option<Vec<String>>,
    },
    /// Fill the cache from an unchanged valid lockfile
    Fetch {
        /// Package root or any path inside the root package
        #[arg(default_value = ".")]
        path: PathBuf,
        /// Refuse all network access and use only cached Git objects/snapshots
        #[arg(long)]
        offline: bool,
        /// Local transport override for deterministic package fixtures
        #[arg(long, value_names = ["URL", "PATH"], num_args = 2, hide = true)]
        transport_override: Option<Vec<String>>,
    },
    /// Re-resolve all or selected root dependency aliases and rewrite the lock
    Update {
        /// Root dependency aliases to update; omit to update every root dependency
        aliases: Vec<String>,
        /// Package root or any path inside the root package
        #[arg(long, default_value = ".")]
        path: PathBuf,
        /// Refuse all network access and use only cached Git objects/snapshots
        #[arg(long)]
        offline: bool,
        /// Local transport override for deterministic package fixtures
        #[arg(long, value_names = ["URL", "PATH"], num_args = 2, hide = true)]
        transport_override: Option<Vec<String>>,
    },
}

impl PackageCommand {
    pub(crate) fn run(self) -> Result<(), Box<dyn Error>> {
        let (path, offline, operation, transport_override) = match &self {
            Self::Sync {
                path,
                offline,
                transport_override,
            } => (path, *offline, Operation::Sync, transport_override),
            Self::Fetch {
                path,
                offline,
                transport_override,
            } => (path, *offline, Operation::Fetch, transport_override),
            Self::Update {
                aliases,
                path,
                offline,
                transport_override,
            } => (
                path,
                *offline,
                Operation::Update(aliases),
                transport_override,
            ),
        };
        let transport_overrides = transport_override
            .as_ref()
            .map(|values| [(values[0].clone(), PathBuf::from(&values[1]))])
            .unwrap_or_default();
        let root = package_root(path)?;
        let lock = zutai_package::acquire::run(AcquireOptions {
            root: &root,
            cache_dir: None,
            offline,
            operation,
            transport_overrides: &transport_overrides,
        })?;
        println!(
            "Prepared {} package nodes in {}",
            lock.packages.len(),
            root.join(zutai_package::LOCK_NAME).display()
        );
        Ok(())
    }
}

fn package_root(path: &std::path::Path) -> Result<PathBuf, Box<dyn Error>> {
    let canonical = std::fs::canonicalize(path)?;
    let mut current = if canonical.is_dir() {
        canonical
    } else {
        canonical
            .parent()
            .ok_or("package path must have a parent directory")?
            .to_path_buf()
    };
    loop {
        if current.join(zutai_package::MANIFEST_NAME).is_file() {
            return Ok(current);
        }
        if !current.pop() {
            return Err(format!(
                "no {} found at or above {}",
                zutai_package::MANIFEST_NAME,
                path.display()
            )
            .into());
        }
    }
}
