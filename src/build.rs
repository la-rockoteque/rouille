//! Builds only the test projects, in one MSBuild invocation.
//!
//! rouille writes a solution that lists the test projects and builds it.
//! MSBuild then builds them and their references in parallel.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};

use crate::project::Project;

pub fn build(state_dir: &Path, projects: &[Project], configuration: &str) -> Result<()> {
    let solution = state_dir.join("tests.slnx");
    let mut xml = String::from("<Solution>\n");
    for project in projects {
        xml.push_str(&format!("  <Project Path=\"{}\" />\n", escape(&project.path.to_string_lossy())));
    }
    xml.push_str("</Solution>\n");
    std::fs::write(&solution, xml).context("cannot write the build solution")?;

    let out = Command::new("dotnet")
        .arg("build")
        .arg(&solution)
        .args(["-c", configuration, "-nologo", "-v:q", "-tl:off", "-clp:ErrorsOnly"])
        .output()
        .context("cannot run dotnet build")?;
    if !out.status.success() {
        let mut log = String::from_utf8_lossy(&out.stdout).to_string();
        log.push_str(&String::from_utf8_lossy(&out.stderr));
        bail!("build failed\n{}", log.trim_end());
    }
    Ok(())
}

const SKIPPED_DIRS: &[&str] = &["bin", "obj", ".rouille", ".git", "TestResults", "node_modules", ".vs", ".idea"];

/// Files above a project that MSBuild or the SDK read.
const SHARED_FILES: &[&str] = &[
    "Directory.Build.props",
    "Directory.Build.targets",
    "Directory.Packages.props",
    "global.json",
    "NuGet.config",
    "nuget.config",
];

/// Fingerprints the build inputs, so rouille can skip a build when nothing
/// changed. It covers every file under the workspace root, under each project
/// the test projects reference, and the shared build files above them.
pub fn source_fingerprint(root: &Path, projects: &[Project], configuration: &str) -> String {
    let mut h = blake3::Hasher::new();
    h.update(env!("CARGO_PKG_VERSION").as_bytes());
    h.update(configuration.as_bytes());
    for project in projects {
        h.update(project.path.as_os_str().as_encoded_bytes());
    }
    let mut files: Vec<_> = input_dirs(root, projects)
        .iter()
        .flat_map(|dir| {
            walkdir::WalkDir::new(dir)
                .into_iter()
                .filter_entry(|e| {
                    !(e.file_type().is_dir()
                        && SKIPPED_DIRS.contains(&e.file_name().to_string_lossy().as_ref()))
                })
                .filter_map(Result::ok)
                .filter(|e| e.file_type().is_file())
                .map(|e| e.into_path())
        })
        .chain(shared_files(root, projects))
        .collect();
    files.sort();
    files.dedup();
    for path in files {
        let Ok(meta) = std::fs::metadata(&path) else { continue };
        let modified = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map_or(0, |d| d.as_nanos());
        h.update(path.as_os_str().as_encoded_bytes());
        h.update(&meta.len().to_le_bytes());
        h.update(&modified.to_le_bytes());
    }
    h.finalize().to_hex().to_string()
}

/// The root and the folder of every project the test projects reach through
/// `ProjectReference`, without folders that are inside another one.
fn input_dirs(root: &Path, projects: &[Project]) -> Vec<PathBuf> {
    let mut seen = HashSet::new();
    let mut stack: Vec<PathBuf> = projects.iter().map(|p| p.path.clone()).collect();
    let mut dirs = vec![root.to_path_buf()];
    while let Some(path) = stack.pop() {
        if !seen.insert(path.clone()) {
            continue;
        }
        if let Some(dir) = path.parent() {
            dirs.push(dir.to_path_buf());
        }
        if let Ok(project) = Project::load(&path) {
            stack.extend(project.references);
        }
    }
    dirs.sort();
    dirs.dedup();
    let mut kept: Vec<PathBuf> = Vec::new();
    for dir in dirs {
        if !kept.iter().any(|k| dir.starts_with(k)) {
            kept.push(dir);
        }
    }
    kept
}

fn shared_files(root: &Path, projects: &[Project]) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let starts = std::iter::once(root).chain(projects.iter().filter_map(|p| p.path.parent()));
    for start in starts {
        for dir in start.ancestors() {
            for name in SHARED_FILES {
                let file = dir.join(name);
                if file.is_file() {
                    found.push(file);
                }
            }
        }
    }
    found
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('"', "&quot;").replace('<', "&lt;")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{Tree, names};

    /// A test project inside `repo/src`, referencing a library outside it.
    fn layout() -> (Tree, Vec<Project>) {
        let tree = Tree::new();
        tree.write("repo/global.json", "{}");
        tree.write("repo/shared/Lib/Lib.csproj", "<Project />");
        tree.write("repo/shared/Lib/Lib.cs", "class Lib {}");
        tree.write("repo/src/Garden/Garden.csproj", r#"<Project><ItemGroup>
            <ProjectReference Include="..\..\shared\Lib\Lib.csproj" /></ItemGroup></Project>"#);
        let tests = tree.write("repo/src/Garden.Tests/Garden.Tests.csproj", r#"<Project><ItemGroup>
            <ProjectReference Include="../Garden/Garden.csproj" /></ItemGroup></Project>"#);
        tree.write("repo/src/Garden.Tests/obj/project.assets.json", "{}");
        (tree, vec![Project::load(&tests).unwrap()])
    }

    #[test]
    fn input_dirs_follow_references_outside_the_root() {
        let (tree, projects) = layout();
        let dirs = input_dirs(&tree.path("repo/src"), &projects);
        assert_eq!(names(&dirs, &tree.root()), vec!["repo/shared/Lib", "repo/src"]);
    }

    #[test]
    fn shared_files_come_from_ancestors() {
        let (tree, projects) = layout();
        let files = shared_files(&tree.path("repo/src"), &projects);
        assert!(files.contains(&tree.path("repo/global.json")));
    }

    #[test]
    fn fingerprint_sees_referenced_and_shared_files_but_not_obj() {
        let (tree, projects) = layout();
        let root = tree.path("repo/src");
        let fp = || source_fingerprint(&root, &projects, "Debug");
        let first = fp();
        assert_eq!(fp(), first);

        tree.write("repo/src/Garden.Tests/obj/project.assets.json", "{\"changed\": true}");
        assert_eq!(fp(), first, "obj/ must not count");

        tree.write("repo/shared/Lib/Lib.cs", "class Lib { int x; }");
        let second = fp();
        assert_ne!(second, first, "a referenced project outside the root changed");

        tree.write("repo/global.json", "{\"sdk\": {}}");
        assert_ne!(fp(), second, "global.json changed");

        assert_ne!(source_fingerprint(&root, &projects, "Release"), fp());
    }
}
