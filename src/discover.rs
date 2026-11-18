//! Finds the test projects under a path, a solution file or a project file.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use walkdir::WalkDir;

use crate::project::Project;

const PROJECT_EXTENSIONS: &[&str] = &["csproj", "fsproj", "vbproj"];
const SKIPPED_DIRS: &[&str] = &["bin", "obj", "node_modules", ".git", ".rouille", "TestResults"];

/// The directory where rouille keeps its state, and the test projects.
pub struct Workspace {
    pub root: PathBuf,
    pub projects: Vec<Project>,
}

pub fn discover(target: &Path) -> Result<Workspace> {
    let target = target
        .canonicalize()
        .with_context(|| format!("{} does not exist", target.display()))?;

    let (root, paths) = if target.is_dir() {
        let solutions = solutions_in(&target)?;
        match solutions.as_slice() {
            [one] => (target.clone(), projects_in_solution(one)?),
            _ => (target.clone(), projects_under(&target)),
        }
    } else {
        let parent = target.parent().unwrap_or(Path::new("/")).to_path_buf();
        match extension(&target).as_str() {
            "sln" | "slnx" => (parent, projects_in_solution(&target)?),
            ext if PROJECT_EXTENSIONS.contains(&ext) => (parent, vec![target.clone()]),
            _ => bail!("{} is not a solution or a project file", target.display()),
        }
    };

    let mut projects = Vec::new();
    for path in paths {
        let project = Project::load(&path)?;
        if project.is_test {
            projects.push(project);
        }
    }
    projects.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(Workspace { root, projects })
}

fn extension(path: &Path) -> String {
    path.extension().unwrap_or_default().to_string_lossy().to_ascii_lowercase()
}

fn solutions_in(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut found = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        if matches!(extension(&path).as_str(), "sln" | "slnx") {
            found.push(path);
        }
    }
    Ok(found)
}

fn projects_under(dir: &Path) -> Vec<PathBuf> {
    WalkDir::new(dir)
        .into_iter()
        .filter_entry(|e| {
            !(e.file_type().is_dir() && SKIPPED_DIRS.contains(&e.file_name().to_string_lossy().as_ref()))
        })
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_file() && PROJECT_EXTENSIONS.contains(&extension(e.path()).as_str()))
        .map(|e| e.into_path())
        .collect()
}

fn projects_in_solution(solution: &Path) -> Result<Vec<PathBuf>> {
    let text = std::fs::read_to_string(solution)
        .with_context(|| format!("cannot read {}", solution.display()))?;
    let dir = solution.parent().unwrap_or(Path::new("."));
    let relative: Vec<String> = if extension(solution) == "slnx" {
        let doc = roxmltree::Document::parse(text.trim_start_matches('\u{feff}'))
            .with_context(|| format!("cannot parse {}", solution.display()))?;
        doc.descendants()
            .filter(|n| n.has_tag_name("Project"))
            .filter_map(|n| n.attribute("Path").map(String::from))
            .collect()
    } else {
        // Classic format: Project("{type}") = "Name", "relative\path.csproj", "{guid}"
        text.lines()
            .filter(|l| l.starts_with("Project("))
            .filter_map(|l| l.split('"').nth(5).map(String::from))
            .collect()
    };

    Ok(relative
        .into_iter()
        .map(|p| dir.join(p.replace('\\', "/")))
        .filter(|p| PROJECT_EXTENSIONS.contains(&extension(p).as_str()) && p.is_file())
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{Tree, names};

    const TEST_PROJECT: &str = r#"<Project><PropertyGroup><IsTestProject>true</IsTestProject></PropertyGroup></Project>"#;
    const LIBRARY: &str = r#"<Project><PropertyGroup><TargetFramework>net10.0</TargetFramework></PropertyGroup></Project>"#;

    fn found(ws: &Workspace) -> Vec<String> {
        let paths: Vec<_> = ws.projects.iter().map(|p| p.path.clone()).collect();
        names(&paths, &ws.root)
    }

    #[test]
    fn classic_solution_with_folders_and_backslashes() {
        let tree = Tree::new();
        tree.write("src/Garden.Domain/Garden.Domain.csproj", LIBRARY);
        tree.write("src/Garden.Domain.Tests/Garden.Domain.Tests.csproj", TEST_PROJECT);
        tree.write("src/Garden.Web.Tests/Garden.Web.Tests.csproj", TEST_PROJECT);
        // A test project that the solution does not list.
        tree.write("src/Scratch.Tests/Scratch.Tests.csproj", TEST_PROJECT);
        tree.write(
            "src/Garden.sln",
            "\u{feff}\nMicrosoft Visual Studio Solution File, Format Version 12.00\n\
             Project(\"{2150E333-8FDC-42A3-9474-1A3956D46DE8}\") = \"tests\", \"tests\", \"{11111111-1111-1111-1111-111111111111}\"\n\
             EndProject\n\
             Project(\"{FAE04EC0-301F-11D3-BF4B-00C04F79EFBC}\") = \"Garden.Domain\", \"Garden.Domain\\Garden.Domain.csproj\", \"{22222222-2222-2222-2222-222222222222}\"\n\
             EndProject\n\
             Project(\"{9A19103F-16F7-4668-BE54-9A1E7A4F7556}\") = \"Garden.Domain.Tests\", \"Garden.Domain.Tests\\Garden.Domain.Tests.csproj\", \"{33333333-3333-3333-3333-333333333333}\"\n\
             EndProject\n\
             Project(\"{9A19103F-16F7-4668-BE54-9A1E7A4F7556}\") = \"Garden.Web.Tests\", \"Garden.Web.Tests\\Garden.Web.Tests.csproj\", \"{44444444-4444-4444-4444-444444444444}\"\n\
             EndProject\n",
        );
        let ws = discover(&tree.path("src")).unwrap();
        assert_eq!(ws.root, tree.path("src"));
        assert_eq!(
            found(&ws),
            vec!["Garden.Domain.Tests/Garden.Domain.Tests.csproj", "Garden.Web.Tests/Garden.Web.Tests.csproj"]
        );
    }

    #[test]
    fn slnx_solution() {
        let tree = Tree::new();
        tree.write("A.Tests/A.Tests.csproj", TEST_PROJECT);
        tree.write("B/B.csproj", LIBRARY);
        tree.write(
            "All.slnx",
            r#"<Solution>
              <Folder Name="/tests/"><Project Path="A.Tests/A.Tests.csproj" /></Folder>
              <Project Path="B/B.csproj" />
              <Project Path="web/package.esproj" />
            </Solution>"#,
        );
        let ws = discover(&tree.path("All.slnx")).unwrap();
        assert_eq!(found(&ws), vec!["A.Tests/A.Tests.csproj"]);
    }

    #[test]
    fn directory_without_solution_skips_build_folders() {
        let tree = Tree::new();
        tree.write("src/A.Tests/A.Tests.csproj", TEST_PROJECT);
        tree.write("src/A.Tests/obj/Copy.Tests.csproj", TEST_PROJECT);
        tree.write("node_modules/pkg/Pkg.Tests.csproj", TEST_PROJECT);
        tree.write("tools/fixtures/Fixtures.csproj", LIBRARY);
        let ws = discover(&tree.root()).unwrap();
        assert_eq!(found(&ws), vec!["src/A.Tests/A.Tests.csproj"]);
    }

    #[test]
    fn directory_with_two_solutions_walks_the_tree() {
        let tree = Tree::new();
        tree.write("A.Tests/A.Tests.csproj", TEST_PROJECT);
        tree.write("B.Tests/B.Tests.csproj", TEST_PROJECT);
        tree.write("One.slnx", r#"<Solution><Project Path="A.Tests/A.Tests.csproj" /></Solution>"#);
        tree.write("Two.slnx", r#"<Solution><Project Path="B.Tests/B.Tests.csproj" /></Solution>"#);
        let ws = discover(&tree.root()).unwrap();
        assert_eq!(found(&ws), vec!["A.Tests/A.Tests.csproj", "B.Tests/B.Tests.csproj"]);
    }

    #[test]
    fn project_file_target() {
        let tree = Tree::new();
        let p = tree.write("Only.Tests/Only.Tests.csproj", TEST_PROJECT);
        let ws = discover(&p).unwrap();
        assert_eq!(ws.root, tree.path("Only.Tests"));
        assert_eq!(found(&ws), vec!["Only.Tests.csproj"]);
    }

    #[test]
    fn other_files_are_rejected() {
        let tree = Tree::new();
        let p = tree.write("README.md", "# hello");
        assert!(discover(&p).is_err());
        assert!(discover(&tree.path("missing")).is_err());
    }
}
