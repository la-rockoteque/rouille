//! Result types and terminal output.

use std::io::IsTerminal;
use std::time::Duration;

use serde::{Deserialize, Serialize};

#[derive(Debug, Default, Clone, Copy, Serialize, Deserialize)]
pub struct Counts {
    pub passed: u32,
    pub failed: u32,
    pub skipped: u32,
}

impl Counts {
    pub fn add(&mut self, other: Counts) {
        self.passed += other.passed;
        self.failed += other.failed;
        self.skipped += other.skipped;
    }

    pub fn total(&self) -> u32 {
        self.passed + self.failed + self.skipped
    }
}

#[derive(Debug, Clone)]
pub struct Failure {
    pub name: String,
    pub message: String,
    pub stack: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Passed,
    Failed,
    /// The process failed and rouille has no per-test results.
    Crashed,
    /// Skipped: the same build already passed.
    Cached,
}

#[derive(Debug, Clone)]
pub struct AssemblyResult {
    pub label: String,
    pub status: Status,
    pub counts: Counts,
    pub failures: Vec<Failure>,
    pub duration: Duration,
    /// Process output, shown when the run crashed.
    pub output: String,
}

pub struct Printer {
    color: bool,
}

impl Printer {
    pub fn new() -> Printer {
        let color = std::io::stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none();
        Printer { color }
    }

    fn paint(&self, code: &str, text: &str) -> String {
        if self.color { format!("\x1b[{code}m{text}\x1b[0m") } else { text.to_string() }
    }

    pub fn status(&self, verb: &str, text: &str) {
        println!("{} {}", self.paint("1;32", &format!("{verb:>10}")), text);
    }

    pub fn assembly(&self, r: &AssemblyResult) {
        let (tag, code) = match r.status {
            Status::Passed => ("PASS", "1;32"),
            Status::Failed => ("FAIL", "1;31"),
            Status::Crashed => ("CRASH", "1;31"),
            Status::Cached => ("CACHED", "1;36"),
        };
        let mut detail = format!("{} passed", r.counts.passed);
        if r.counts.failed > 0 {
            detail.push_str(&format!(", {} failed", r.counts.failed));
        }
        if r.counts.skipped > 0 {
            detail.push_str(&format!(", {} skipped", r.counts.skipped));
        }
        let time = if r.status == Status::Cached { "     -".to_string() } else { secs(r.duration) };
        println!("{} [{}] {} ({})", self.paint(code, &format!("{tag:>10}")), time, r.label, detail);

        for f in &r.failures {
            println!("{}  {}", self.paint("31", "           ✗"), self.paint("1", &f.name));
            for line in f.message.lines().chain(f.stack.lines().take(4)) {
                println!("               {}", line.trim());
            }
        }
        // Without per-test results, the process output is the only detail.
        let no_detail = r.status == Status::Failed && r.failures.is_empty();
        if r.status == Status::Crashed || no_detail {
            for line in r.output.lines().rev().take(25).collect::<Vec<_>>().into_iter().rev() {
                println!("               {}", line.trim_end());
            }
        }
    }

    pub fn summary(&self, results: &[AssemblyResult], elapsed: Duration) {
        let mut counts = Counts::default();
        for r in results {
            counts.add(r.counts);
        }
        let failed = results.iter().filter(|r| matches!(r.status, Status::Failed | Status::Crashed)).count();
        let cached = results.iter().filter(|r| r.status == Status::Cached).count();
        let mut line = format!(
            "{} tests: {} passed, {} failed, {} skipped",
            counts.total(),
            counts.passed,
            counts.failed,
            counts.skipped
        );
        line.push_str(&format!("; {} assemblies", results.len()));
        if cached > 0 {
            line.push_str(&format!(" ({cached} cached)"));
        }
        line.push_str(&format!(" in {:.2}s", elapsed.as_secs_f64()));
        println!("{}", "-".repeat(60));
        let (verb, code) = if failed == 0 { ("Summary", "1;32") } else { ("Summary", "1;31") };
        println!("{} {}", self.paint(code, &format!("{verb:>10}")), line);
    }
}

fn secs(d: Duration) -> String {
    format!("{:>5.2}s", d.as_secs_f64())
}
