use rustc_hash::FxHashMap;
use sha2::{Digest as _, Sha256};
use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;

use crate::{Analysis, AnalysisOptions};

pub(crate) type Fingerprint = [u8; 32];

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum CacheDependencySource {
    Relative {
        base: PathBuf,
        path: String,
    },
    Stdlib {
        name: String,
    },
    Package {
        importer: Option<PathBuf>,
        parts: Vec<String>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum CacheDependencyKind {
    Data,
    Module,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CacheDependency {
    pub source: CacheDependencySource,
    pub kind: CacheDependencyKind,
    pub fingerprint: Fingerprint,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct ModuleCacheSlot {
    pub identity: PathBuf,
    pub run_hir_passes: bool,
    pub run_thir_passes: bool,
}

impl ModuleCacheSlot {
    pub fn new(identity: PathBuf, options: AnalysisOptions) -> Self {
        Self {
            identity,
            run_hir_passes: options.run_hir_passes,
            run_thir_passes: options.run_thir_passes,
        }
    }
}

#[derive(Clone)]
pub(crate) struct CachedAnalysis {
    pub source: Fingerprint,
    pub context: Fingerprint,
    pub fingerprint: Fingerprint,
    pub dependencies: Vec<CacheDependency>,
    pub analysis: Rc<Analysis>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AnalysisCacheStats {
    pub module_hits: usize,
    pub module_misses: usize,
}

/// Explicit, process-local cache for completed imported-module analyses.
///
/// Entries are addressed by stable module identity and validated against source,
/// package-manifest, standard-library, compiler, and transitive dependency
/// fingerprints before reuse. The cache has no ambient global state: callers own
/// it and choose the lifetime shared by CLI work, an LSP session, or web rebuilds.
#[derive(Default)]
pub struct AnalysisCache {
    entries: RefCell<FxHashMap<ModuleCacheSlot, CachedAnalysis>>,
    hits: Cell<usize>,
    misses: Cell<usize>,
}

impl AnalysisCache {
    pub fn stats(&self) -> AnalysisCacheStats {
        AnalysisCacheStats {
            module_hits: self.hits.get(),
            module_misses: self.misses.get(),
        }
    }

    pub fn clear(&self) {
        self.entries.borrow_mut().clear();
        self.hits.set(0);
        self.misses.set(0);
    }

    pub(crate) fn get(&self, slot: &ModuleCacheSlot) -> Option<CachedAnalysis> {
        self.entries.borrow().get(slot).cloned()
    }

    pub(crate) fn insert(&self, slot: ModuleCacheSlot, entry: CachedAnalysis) {
        self.entries.borrow_mut().insert(slot, entry);
    }

    pub(crate) fn record_hit(&self) {
        self.hits.set(self.hits.get().saturating_add(1));
    }

    pub(crate) fn record_miss(&self) {
        self.misses.set(self.misses.get().saturating_add(1));
    }
}

pub(crate) fn fingerprint_parts<'a>(parts: impl IntoIterator<Item = &'a [u8]>) -> Fingerprint {
    let mut digest = Sha256::new();
    for part in parts {
        digest.update((part.len() as u64).to_le_bytes());
        digest.update(part);
    }
    digest.finalize().into()
}

pub(crate) fn fingerprint_text(text: &str) -> Fingerprint {
    fingerprint_parts([text.as_bytes()])
}

pub(crate) fn module_fingerprint(
    slot: &ModuleCacheSlot,
    source: Fingerprint,
    context: Fingerprint,
    dependencies: &[CacheDependency],
) -> Fingerprint {
    let mut digest = Sha256::new();
    update_part(&mut digest, slot.identity.to_string_lossy().as_bytes());
    update_part(
        &mut digest,
        &[slot.run_hir_passes as u8, slot.run_thir_passes as u8],
    );
    update_part(&mut digest, &source);
    update_part(&mut digest, &context);
    for dependency in dependencies {
        match &dependency.source {
            CacheDependencySource::Relative { base, path } => {
                update_part(&mut digest, b"relative");
                update_part(&mut digest, base.to_string_lossy().as_bytes());
                update_part(&mut digest, path.as_bytes());
            }
            CacheDependencySource::Stdlib { name } => {
                update_part(&mut digest, b"stdlib");
                update_part(&mut digest, name.as_bytes());
            }
            CacheDependencySource::Package { importer, parts } => {
                update_part(&mut digest, b"package");
                if let Some(importer) = importer {
                    update_part(&mut digest, importer.to_string_lossy().as_bytes());
                } else {
                    update_part(&mut digest, b"");
                }
                for part in parts {
                    update_part(&mut digest, part.as_bytes());
                }
            }
        }
        update_part(
            &mut digest,
            &[match dependency.kind {
                CacheDependencyKind::Data => 0,
                CacheDependencyKind::Module => 1,
            }],
        );
        update_part(&mut digest, &dependency.fingerprint);
    }
    digest.finalize().into()
}

fn update_part(digest: &mut Sha256, part: &[u8]) {
    digest.update((part.len() as u64).to_le_bytes());
    digest.update(part);
}
