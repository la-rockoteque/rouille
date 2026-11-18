//! Remembers which builds of a test assembly already passed.
//!
//! The fingerprint covers every file in the output directory, the filter and
//! the environment. When any of them changes, the assembly runs again. Files a
//! test reads from outside its output directory are not covered: use
//! `--no-cache` when such a file changes.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use serde::{Deserialize, Serialize};
use walkdir::WalkDir;

use crate::report::Counts;
use crate::runner::Target;

/// Variables that change between shells but do not change test behavior.
const VOLATILE_ENV: &[&str] = &[
    "PWD", "OLDPWD", "SHLVL", "_", "TERM_SESSION_ID", "ITERM_SESSION_ID", "WINDOWID",
    "COLUMNS", "LINES", "SECURITYSESSIONID", "SSH_AUTH_SOCK", "TMUX_PANE",
];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entry {
    pub fingerprint: String,
    pub passed: bool,
    pub counts: Counts,
    pub duration_ms: u64,
}

#[derive(Default, Serialize, Deserialize)]
pub struct Cache {
    /// Source fingerprint of the last successful build.
    #[serde(default)]
    pub build: Option<String>,
    entries: HashMap<String, Entry>,
    #[serde(skip)]
    path: PathBuf,
}

impl Cache {
    pub fn load(state_dir: &Path) -> Cache {
        let path = state_dir.join("cache.json");
        let mut cache: Cache = std::fs::read(&path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default();
        cache.path = path;
        cache
    }

    pub fn get(&self, target: &Target) -> Option<&Entry> {
        self.entries.get(&key(target))
    }

    pub fn put(&mut self, target: &Target, entry: Entry) {
        self.entries.insert(key(target), entry);
    }

    pub fn save(&self) -> std::io::Result<()> {
        let json = serde_json::to_vec_pretty(self).map_err(std::io::Error::other)?;
        std::fs::write(&self.path, json)
    }
}

fn key(target: &Target) -> String {
    target.dll.to_string_lossy().to_string()
}

pub fn fingerprint(target: &Target, filter: Option<&str>) -> String {
    let mut h = blake3::Hasher::new();
    h.update(env!("CARGO_PKG_VERSION").as_bytes());
    h.update(target.kind.name().as_bytes());
    h.update(filter.unwrap_or("").as_bytes());
    h.update(&[0]);

    let env: BTreeMap<_, _> = std::env::vars_os()
        .filter(|(k, _)| !VOLATILE_ENV.contains(&k.to_string_lossy().as_ref()))
        .collect();
    for (k, v) in env {
        h.update(k.as_encoded_bytes());
        h.update(b"=");
        h.update(v.as_encoded_bytes());
        h.update(&[0]);
    }

    let dir = target.output_dir();
    let mut files: Vec<_> = WalkDir::new(dir)
        .into_iter()
        .filter_entry(|e| e.file_name() != "TestResults")
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_file())
        .collect();
    files.sort_by(|a, b| a.path().cmp(b.path()));
    for f in files {
        let Ok(meta) = f.metadata() else { continue };
        let modified = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map_or(0, |d| d.as_nanos());
        h.update(f.path().strip_prefix(dir).unwrap_or(f.path()).as_os_str().as_encoded_bytes());
        h.update(&meta.len().to_le_bytes());
        h.update(&modified.to_le_bytes());
    }
    h.finalize().to_hex().to_string()
}
