//! Reads the parts of an MSBuild project file that rouille needs.
//!
//! This is a static read. It ignores `Condition` attributes and imported files
//! such as `Directory.Build.props`. When the result is not enough to find the
//! build output, `locate` falls back to asking MSBuild.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};

/// Package references that mark a project as a test project.
const TEST_PACKAGES: &[&str] = &[
    "microsoft.net.test.sdk",
    "xunit.v3",
    "xunit.v3.core",
    "mstest",
    "mstest.testadapter",
    "nunit3testadapter",
    "tunit",
    "microsoft.testing.platform",
];

#[derive(Debug, Clone)]
pub struct Project {
    pub path: PathBuf,
    pub name: String,
    pub assembly_name: String,
    pub target_frameworks: Vec<String>,
    pub is_test: bool,
    pub properties: HashMap<String, String>,
    /// Paths of the projects in `ProjectReference` items.
    pub references: Vec<PathBuf>,
}

impl Project {
    pub fn load(path: &Path) -> Result<Project> {
        let raw = std::fs::read_to_string(path)
            .with_context(|| format!("cannot read {}", path.display()))?;
        let text = raw.trim_start_matches('\u{feff}');
        let doc = roxmltree::Document::parse(text)
            .with_context(|| format!("cannot parse {}", path.display()))?;
        let root = doc.root_element();

        let mut properties = HashMap::new();
        let mut packages = Vec::new();
        let mut references = Vec::new();
        let dir = path.parent().unwrap_or(Path::new("."));
        for node in root.descendants().filter(|n| n.is_element()) {
            let parent = node.parent_element().map(|p| p.tag_name().name());
            if parent == Some("PropertyGroup") {
                let value = node.text().unwrap_or("").trim().to_string();
                properties.insert(node.tag_name().name().to_string(), value);
            } else if node.tag_name().name() == "PackageReference"
                && let Some(id) = node.attribute("Include")
            {
                packages.push(id.to_ascii_lowercase());
            } else if node.tag_name().name() == "ProjectReference"
                && let Some(include) = node.attribute("Include")
            {
                let target = dir.join(include.replace('\\', "/"));
                references.push(target.canonicalize().unwrap_or(target));
            }
        }

        let sdk = root.attribute("Sdk").unwrap_or("").to_ascii_lowercase();
        let declared = properties.get("IsTestProject").map(|v| v.eq_ignore_ascii_case("true"));
        let is_test = declared.unwrap_or_else(|| {
            sdk.starts_with("mstest.sdk")
                || packages.iter().any(|p| TEST_PACKAGES.contains(&p.as_str()))
        });

        let name = path.file_stem().unwrap_or_default().to_string_lossy().to_string();
        let assembly_name = properties.get("AssemblyName").cloned().unwrap_or_else(|| name.clone());
        let target_frameworks = match (properties.get("TargetFrameworks"), properties.get("TargetFramework")) {
            (Some(many), _) if !many.is_empty() => many
                .split(';')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(String::from)
                .collect(),
            (_, Some(one)) if !one.is_empty() => vec![one.clone()],
            _ => Vec::new(),
        };

        Ok(Project {
            path: path.to_path_buf(),
            name,
            assembly_name,
            target_frameworks,
            is_test,
            properties,
            references,
        })
    }

    pub fn dir(&self) -> &Path {
        self.path.parent().unwrap_or(Path::new("."))
    }

    pub fn property_is_true(&self, key: &str) -> bool {
        self.properties.get(key).is_some_and(|v| v.eq_ignore_ascii_case("true"))
    }

    /// Returns the built assembly for each target framework.
    pub fn locate(&self, configuration: &str) -> Result<Vec<(String, PathBuf)>> {
        let mut found = Vec::new();
        for tfm in &self.target_frameworks {
            let guess = self
                .dir()
                .join("bin")
                .join(configuration)
                .join(tfm)
                .join(format!("{}.dll", self.assembly_name));
            if guess.is_file() {
                found.push((tfm.clone(), guess));
            } else {
                found.push((tfm.clone(), self.ask_msbuild(configuration, Some(tfm))?));
            }
        }
        if found.is_empty() {
            // No framework in the file itself: it comes from an import.
            let path = self.ask_msbuild(configuration, None)?;
            let tfm = path
                .parent()
                .and_then(|p| p.file_name())
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default();
            found.push((tfm, path));
        }
        Ok(found)
    }

    fn ask_msbuild(&self, configuration: &str, tfm: Option<&str>) -> Result<PathBuf> {
        let mut cmd = Command::new("dotnet");
        cmd.arg("msbuild")
            .arg(&self.path)
            .arg("-getProperty:TargetPath")
            .arg(format!("-p:Configuration={configuration}"));
        if let Some(tfm) = tfm {
            cmd.arg(format!("-p:TargetFramework={tfm}"));
        }
        let out = cmd.output().context("cannot run dotnet msbuild")?;
        let path = PathBuf::from(String::from_utf8_lossy(&out.stdout).trim());
        if !out.status.success() || !path.is_file() {
            bail!(
                "cannot find the build output of {} (did the build succeed?)",
                self.path.display()
            );
        }
        Ok(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::Tree;

    fn load(tree: &Tree, relative: &str, xml: &str) -> Project {
        Project::load(&tree.write(relative, xml)).unwrap()
    }

    #[test]
    fn declared_test_project_with_xunit_v2() {
        let tree = Tree::new();
        let p = load(
            &tree,
            "Garden.Web.Tests/Garden.Web.Tests.csproj",
            r#"<Project Sdk="Microsoft.NET.Sdk">
              <PropertyGroup>
                <TargetFramework>net10.0</TargetFramework>
                <IsTestProject>true</IsTestProject>
              </PropertyGroup>
              <!-- a comment between groups -->
              <ItemGroup>
                <PackageReference Include="Microsoft.NET.Test.Sdk" Version="17.12.0" />
                <PackageReference Include="xunit" Version="2.9.3" />
              </ItemGroup>
            </Project>"#,
        );
        assert!(p.is_test);
        assert_eq!(p.name, "Garden.Web.Tests");
        assert_eq!(p.assembly_name, "Garden.Web.Tests");
        assert_eq!(p.target_frameworks, vec!["net10.0"]);
    }

    #[test]
    fn nunit_packages_without_is_test_project() {
        let tree = Tree::new();
        let p = load(
            &tree,
            "src/Garden.Checks/Garden.Checks.csproj",
            r#"<Project Sdk="Microsoft.NET.Sdk">
              <PropertyGroup><TargetFramework>net10.0</TargetFramework></PropertyGroup>
              <ItemGroup>
                <PackageReference Include="NUnit" Version="4.2.2" />
                <PackageReference Include="NUnit3TestAdapter" Version="4.6.0" />
                <PackageReference Include="coverlet.collector" Version="6.0.2">
                  <PrivateAssets>all</PrivateAssets>
                </PackageReference>
              </ItemGroup>
            </Project>"#,
        );
        assert!(p.is_test);
    }

    #[test]
    fn console_app_with_only_a_project_reference_is_not_a_test() {
        let tree = Tree::new();
        let lib = tree.write("src/Garden.Storage/Garden.Storage.csproj", "<Project />");
        let p = load(
            &tree,
            "tools/fixtures/Fixtures.csproj",
            r#"<Project Sdk="Microsoft.NET.Sdk">
              <PropertyGroup>
                <OutputType>Exe</OutputType>
                <TargetFramework>net10.0</TargetFramework>
              </PropertyGroup>
              <ItemGroup>
                <ProjectReference Include="../../src/Garden.Storage/Garden.Storage.csproj" />
              </ItemGroup>
            </Project>"#,
        );
        assert!(!p.is_test);
        assert_eq!(p.references, vec![lib]);
    }

    #[test]
    fn support_library_with_backslash_references_is_not_a_test() {
        let tree = Tree::new();
        let storage = tree.write("Garden.Storage/Garden.Storage.csproj", "<Project />");
        let missing = tree.path("Garden.Helpers/../Garden.Missing/Garden.Missing.csproj");
        let p = load(
            &tree,
            "Garden.Helpers/Garden.Helpers.csproj",
            r#"<Project Sdk="Microsoft.NET.Sdk">
              <PropertyGroup><TargetFramework>net10.0</TargetFramework></PropertyGroup>
              <ItemGroup>
                <ProjectReference Include="..\Garden.Storage\Garden.Storage.csproj" />
                <ProjectReference Include="..\Garden.Missing\Garden.Missing.csproj" />
              </ItemGroup>
            </Project>"#,
        );
        assert!(!p.is_test);
        assert_eq!(p.references, vec![storage, missing]);
    }

    #[test]
    fn is_test_project_false_wins_over_packages() {
        let tree = Tree::new();
        let p = load(
            &tree,
            "Helpers/Helpers.csproj",
            r#"<Project Sdk="Microsoft.NET.Sdk">
              <PropertyGroup><IsTestProject>false</IsTestProject></PropertyGroup>
              <ItemGroup><PackageReference Include="xunit.v3" Version="3.2.0" /></ItemGroup>
            </Project>"#,
        );
        assert!(!p.is_test);
    }

    #[test]
    fn mstest_sdk_marks_a_test_project() {
        let tree = Tree::new();
        let p = load(&tree, "T/T.csproj", r#"<Project Sdk="MSTest.Sdk/4.0.2"></Project>"#);
        assert!(p.is_test);
    }

    #[test]
    fn byte_order_mark_assembly_name_and_many_frameworks() {
        let tree = Tree::new();
        let p = load(
            &tree,
            "Lib.Tests/Lib.Tests.csproj",
            "\u{feff}<Project Sdk=\"Microsoft.NET.Sdk\"><PropertyGroup>\
             <TargetFrameworks> net8.0;net10.0; </TargetFrameworks>\
             <AssemblyName>Lib.Specs</AssemblyName>\
             <IsTestProject>True</IsTestProject>\
             </PropertyGroup></Project>",
        );
        assert!(p.is_test);
        assert_eq!(p.assembly_name, "Lib.Specs");
        assert_eq!(p.target_frameworks, vec!["net8.0", "net10.0"]);
    }

    #[test]
    fn locate_finds_the_conventional_output() {
        let tree = Tree::new();
        let p = load(
            &tree,
            "Lib.Tests/Lib.Tests.csproj",
            r#"<Project><PropertyGroup><TargetFramework>net10.0</TargetFramework></PropertyGroup></Project>"#,
        );
        let dll = tree.write("Lib.Tests/bin/Release/net10.0/Lib.Tests.dll", "");
        assert_eq!(p.locate("Release").unwrap(), vec![("net10.0".to_string(), dll)]);
    }
}
