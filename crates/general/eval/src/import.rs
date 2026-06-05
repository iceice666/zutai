use std::path::PathBuf;

use rustc_hash::FxHashMap;

use crate::value::EvalValue;

/// Per-session cache for resolved imports.
///
/// Later import resolution should canonicalize paths before using them as
/// keys, so repeated imports share one evaluated value.
#[derive(Debug, Default)]
pub struct ImportCache {
    entries: FxHashMap<PathBuf, ImportEntry>,
}

impl ImportCache {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get(&self, path: &PathBuf) -> Option<&ImportEntry> {
        self.entries.get(path)
    }

    pub fn insert(&mut self, path: PathBuf, entry: ImportEntry) -> Option<ImportEntry> {
        self.entries.insert(path, entry)
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }
}

/// Cached result for one import.
#[derive(Debug, Clone)]
pub enum ImportEntry {
    Immediate(EvalValue),
    General(EvalValue),
}
