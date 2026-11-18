//! Runs one test assembly with the runner it was built for.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Instant;

use anyhow::{Context, Result};

use crate::report::{AssemblyResult, Counts, Status};
use crate::trx;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Microsoft.Testing.Platform: the assembly is the test host.
    Mtp,
    /// xUnit v3 in-process runner, the default entry point of xUnit v3.
    XunitV3,
    /// VSTest: `dotnet vstest` loads the assembly into a test host.
    VsTest,
}

impl Kind {
    pub fn name(self) -> &'static str {
        match self {
            Kind::Mtp => "mtp",
            Kind::XunitV3 => "xunit-v3",
            Kind::VsTest => "vstest",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Target {
    pub label: String,
    pub dll: PathBuf,
    pub kind: Kind,
}

impl Target {
    pub fn output_dir(&self) -> &Path {
        self.dll.parent().unwrap_or(Path::new("."))
    }
}

/// Detects the runner from the files next to the assembly.
pub fn detect(dll: &Path, uses_mtp_runner: bool) -> Kind {
    let dir = dll.parent().unwrap_or(Path::new("."));
    let has = |file: &str| dir.join(file).is_file();
    if has("xunit.v3.runner.inproc.console.dll") && !uses_mtp_runner {
        Kind::XunitV3
    } else if has("Microsoft.Testing.Platform.dll") && !has("testhost.dll") {
        Kind::Mtp
    } else {
        Kind::VsTest
    }
}

pub struct RunOptions<'a> {
    pub filter: Option<&'a str>,
    pub results_dir: &'a Path,
}

pub fn run(target: &Target, opts: &RunOptions) -> Result<AssemblyResult> {
    let trx_path = opts.results_dir.join(format!("{}.trx", sanitize(&target.label)));
    let _ = std::fs::remove_file(&trx_path);
    let dll = target.dll.to_string_lossy().to_string();
    let has_trx_extension = target.output_dir().join("Microsoft.Testing.Extensions.TrxReport.dll").is_file();

    let mut cmd = Command::new("dotnet");
    match target.kind {
        Kind::VsTest => {
            cmd.arg("vstest").arg(&dll).arg(format!("--logger:trx;LogFileName={}", trx_path.display()));
            if let Some(f) = opts.filter {
                cmd.arg(format!("--TestCaseFilter:FullyQualifiedName~{f}"));
            }
        }
        Kind::XunitV3 => {
            cmd.arg(&dll).args(["-noLogo", "-noColor", "-trx"]).arg(&trx_path);
            if let Some(f) = opts.filter {
                cmd.arg("-method").arg(format!("*{f}*"));
            }
        }
        Kind::Mtp => {
            cmd.arg(&dll).args(["--no-progress", "--no-ansi"]);
            if has_trx_extension {
                cmd.args(["--report-trx", "--report-trx-filename"])
                    .arg(trx_path.file_name().unwrap())
                    .arg("--results-directory")
                    .arg(opts.results_dir);
            }
            if let Some(f) = opts.filter {
                cmd.arg("--filter").arg(format!("FullyQualifiedName~{f}"));
            }
        }
    }
    cmd.current_dir(target.output_dir()).stdin(Stdio::null());

    let start = Instant::now();
    let out = cmd.output().with_context(|| format!("cannot start the tests of {}", target.label))?;
    let duration = start.elapsed();
    let mut output = String::from_utf8_lossy(&out.stdout).to_string();
    output.push_str(&String::from_utf8_lossy(&out.stderr));
    let code = out.status.code();

    let mut result = AssemblyResult {
        label: target.label.clone(),
        status: Status::Crashed,
        counts: Counts::default(),
        failures: Vec::new(),
        duration,
        output,
    };

    if trx_path.is_file() {
        let parsed = trx::read(&trx_path)?;
        result.counts = parsed.counts;
        result.failures = parsed.failures;
        result.status = if parsed.counts.failed > 0 { Status::Failed } else { Status::Passed };
        // A non-zero exit with no failed test means the host itself failed.
        let host_ok = out.status.success() || parsed.counts.failed > 0 || mtp_no_tests(target.kind, code);
        if !host_ok {
            result.status = Status::Crashed;
        }
    } else if target.kind == Kind::Mtp {
        if let Some(counts) = mtp_console_counts(&result.output) {
            result.counts = counts;
            let ok = out.status.success() || mtp_no_tests(target.kind, code);
            result.status = match (ok, counts.failed) {
                (true, 0) => Status::Passed,
                (_, n) if n > 0 => Status::Failed,
                _ => Status::Crashed,
            };
        }
    } else if mtp_no_tests(target.kind, code) || (out.status.success() && opts.filter.is_some()) {
        // Nothing matched the filter.
        result.status = Status::Passed;
    }
    Ok(result)
}

/// Exit code 8 means "zero tests ran", which is fine when filtering.
fn mtp_no_tests(kind: Kind, code: Option<i32>) -> bool {
    kind == Kind::Mtp && code == Some(8)
}

/// Reads the MTP console summary when the TRX extension is not installed.
pub fn mtp_console_counts(output: &str) -> Option<Counts> {
    let start = output.rfind("Test run summary:")?;
    let mut counts = Counts::default();
    let mut seen = false;
    for line in output[start..].lines().skip(1) {
        let Some((key, value)) = line.trim().split_once(':') else { continue };
        let Ok(n) = value.trim().parse::<u32>() else { continue };
        match key.trim() {
            "succeeded" => counts.passed = n,
            "failed" => counts.failed = n,
            "skipped" => counts.skipped = n,
            "total" => {}
            _ => continue,
        }
        seen = true;
    }
    seen.then_some(counts)
}

fn sanitize(label: &str) -> String {
    label.chars().map(|c| if c.is_ascii_alphanumeric() || c == '.' || c == '-' { c } else { '_' }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::Tree;

    fn output(files: &[&str]) -> (Tree, PathBuf) {
        let tree = Tree::new();
        for f in files {
            tree.write(&format!("bin/{f}"), "");
        }
        let dll = tree.write("bin/Garden.Tests.dll", "");
        (tree, dll)
    }

    #[test]
    fn detects_vstest_from_testhost() {
        // VSTest projects also ship the MTP assembly, since Microsoft.NET.Test.Sdk 17.10.
        let (_t, dll) = output(&["testhost.dll", "Microsoft.Testing.Platform.dll"]);
        assert_eq!(detect(&dll, false), Kind::VsTest);
        let (_t, dll) = output(&["nunit.framework.dll"]);
        assert_eq!(detect(&dll, false), Kind::VsTest);
    }

    #[test]
    fn detects_mtp_without_testhost() {
        let (_t, dll) = output(&["Microsoft.Testing.Platform.dll"]);
        assert_eq!(detect(&dll, false), Kind::Mtp);
    }

    #[test]
    fn detects_xunit_v3_native_unless_mtp_is_requested() {
        let files = ["xunit.v3.runner.inproc.console.dll", "Microsoft.Testing.Platform.dll", "xunit.v3.mtp-v1.dll"];
        let (_t, dll) = output(&files);
        assert_eq!(detect(&dll, false), Kind::XunitV3);
        assert_eq!(detect(&dll, true), Kind::Mtp);
    }

    #[test]
    fn reads_the_mtp_console_summary() {
        let out = "NUnit Adapter 5.0.0.0: Test execution started\n\
            failed FailsOnDemand (11ms)\n\
            Test run summary: Failed! - /x/Garden.Tests.dll (net10.0|arm64)\n  \
            total: 4\n  failed: 1\n  succeeded: 3\n  skipped: 0\n  duration: 87ms\n";
        let c = mtp_console_counts(out).unwrap();
        assert_eq!((c.passed, c.failed, c.skipped), (3, 1, 0));
    }

    #[test]
    fn uses_the_last_summary_and_needs_one() {
        let out = "Test run summary: Passed!\n  succeeded: 9\n\
                   Test run summary: Passed!\n  total: 2\n  succeeded: 2\n";
        assert_eq!(mtp_console_counts(out).unwrap().passed, 2);
        assert!(mtp_console_counts("total: 4\nfailed: 1").is_none());
        assert!(mtp_console_counts("Test run summary: crashed\n").is_none());
    }

    #[test]
    fn sanitizes_labels_for_file_names() {
        assert_eq!(sanitize("Garden.Tests (net8.0)"), "Garden.Tests__net8.0_");
    }
}
