//! Helpers for unit tests.

use std::path::{Path, PathBuf};

/// A temporary directory that tests fill with files.
pub struct Tree {
    dir: tempfile::TempDir,
}

impl Tree {
    pub fn new() -> Tree {
        Tree { dir: tempfile::tempdir().unwrap() }
    }

    /// The canonical root, so paths compare equal to discovered ones.
    pub fn root(&self) -> PathBuf {
        self.dir.path().canonicalize().unwrap()
    }

    pub fn write(&self, relative: &str, content: &str) -> PathBuf {
        let path = self.root().join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, content).unwrap();
        path
    }

    pub fn path(&self, relative: &str) -> PathBuf {
        self.root().join(relative)
    }
}

pub fn names(paths: &[PathBuf], root: &Path) -> Vec<String> {
    let mut names: Vec<String> = paths
        .iter()
        // Forward slashes on every platform, so expected values read the same.
        .map(|p| p.strip_prefix(root).unwrap_or(p).display().to_string().replace('\\', "/"))
        .collect();
    names.sort();
    names
}
