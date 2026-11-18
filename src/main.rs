mod build;
mod cache;
mod discover;
mod project;
mod report;
mod runner;
#[cfg(test)]
mod testutil;
mod trx;

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::{Mutex, mpsc};
use std::time::{Duration, Instant};

use anyhow::{Result, bail};
use clap::{Args, Parser, Subcommand};

use crate::cache::{Cache, Entry};
use crate::report::{AssemblyResult, Printer, Status};
use crate::runner::{RunOptions, Target};

#[derive(Parser)]
#[command(name = "rouille", version, about = "A fast .NET toolchain")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Build and run the test projects in parallel.
    Test(TestArgs),
    /// Any other command goes to `dotnet`.
    #[command(external_subcommand)]
    Dotnet(Vec<String>),
}

#[derive(Args)]
struct TestArgs {
    /// A directory, solution or project file.
    #[arg(default_value = ".")]
    path: PathBuf,

    /// Run only tests whose fully qualified name contains this text.
    #[arg(long)]
    filter: Option<String>,

    /// The build configuration.
    #[arg(short, long, default_value = "Debug")]
    configuration: String,

    /// Do not build. Use the existing build output.
    #[arg(long)]
    no_build: bool,

    /// Run every assembly, even when the same build already passed.
    #[arg(long)]
    no_cache: bool,

    /// How many test assemblies run at the same time.
    #[arg(short, long)]
    jobs: Option<usize>,

    /// List the test assemblies and their runners, then stop.
    #[arg(long)]
    list: bool,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let outcome = match cli.command {
        Command::Test(args) => test(args),
        Command::Dotnet(args) => passthrough(&args),
    };
    match outcome {
        Ok(code) => code,
        Err(err) => {
            eprintln!("error: {err:#}");
            ExitCode::from(2)
        }
    }
}

fn passthrough(args: &[String]) -> Result<ExitCode> {
    let status = std::process::Command::new("dotnet").args(args).status()?;
    Ok(ExitCode::from(status.code().unwrap_or(1).clamp(0, 255) as u8))
}

fn test(args: TestArgs) -> Result<ExitCode> {
    let started = Instant::now();
    let printer = Printer::new();
    let workspace = discover::discover(&args.path)?;
    if workspace.projects.is_empty() {
        bail!("no test project found in {}", args.path.display());
    }
    let state_dir = state_dir(&workspace.root);
    let results_dir = state_dir.join("results");
    std::fs::create_dir_all(&results_dir)?;

    let mut cache = Cache::load(&state_dir);
    if !args.no_build && !args.list {
        let sources = build::source_fingerprint(&workspace.root, &workspace.projects, &args.configuration);
        if !args.no_cache && cache.build.as_deref() == Some(sources.as_str()) {
            printer.status("Fresh", "no source changed since the last build");
        } else {
            printer.status("Building", &format!("{} test projects", workspace.projects.len()));
            cache.build = None;
            build::build(&state_dir, &workspace.projects, &args.configuration)?;
            cache.build = Some(sources);
        }
    }

    let mut targets = Vec::new();
    for project in &workspace.projects {
        let outputs = match project.locate(&args.configuration) {
            Ok(outputs) => outputs,
            Err(_) if args.list => {
                println!("{:<9} {:<30} (not built)", "-", project.name);
                continue;
            }
            Err(err) => return Err(err),
        };
        let many = outputs.len() > 1;
        for (tfm, dll) in outputs {
            let label = if many { format!("{} ({tfm})", project.name) } else { project.name.clone() };
            let kind = runner::detect(&dll, project.property_is_true("UseMicrosoftTestingPlatformRunner"));
            targets.push(Target { label, dll, kind });
        }
    }

    if args.list {
        for t in &targets {
            println!("{:<9} {:<30} {}", t.kind.name(), t.label, rel(&workspace.root, &t.dll));
        }
        return Ok(ExitCode::SUCCESS);
    }

    let filter = args.filter.as_deref();
    let mut results = Vec::new();
    let mut queue = Vec::new();
    for t in targets {
        let fp = cache::fingerprint(&t, filter);
        match cache.get(&t) {
            Some(e) if !args.no_cache && e.passed && e.fingerprint == fp => {
                results.push(AssemblyResult {
                    label: t.label.clone(),
                    status: Status::Cached,
                    counts: e.counts,
                    failures: Vec::new(),
                    duration: Duration::ZERO,
                    output: String::new(),
                });
            }
            previous => {
                let last = previous.map_or(u64::MAX, |e| e.duration_ms);
                queue.push((t, fp, last));
            }
        }
    }
    for r in &results {
        printer.assembly(r);
    }

    // Slowest first, so the longest assembly does not start last.
    queue.sort_by_key(|a| std::cmp::Reverse(a.2));
    let jobs = args
        .jobs
        .unwrap_or_else(|| std::thread::available_parallelism().map_or(4, |n| n.get()))
        .max(1);
    if !queue.is_empty() {
        printer.status("Running", &format!("{} assemblies, {} at a time", queue.len(), jobs.min(queue.len())));
    }

    let opts = RunOptions { filter, results_dir: &results_dir };
    let work = Mutex::new(queue.into_iter().collect::<VecDeque<_>>());
    let (tx, rx) = mpsc::channel();
    std::thread::scope(|scope| {
        for _ in 0..jobs {
            let tx = tx.clone();
            let (work, opts) = (&work, &opts);
            scope.spawn(move || {
                loop {
                    let Some((target, fp, _)) = work.lock().unwrap().pop_front() else { break };
                    let result = runner::run(&target, opts);
                    if tx.send((target, fp, result)).is_err() {
                        break;
                    }
                }
            });
        }
        drop(tx);
        for (target, fingerprint, result) in rx {
            let result = result.unwrap_or_else(|err| AssemblyResult {
                label: target.label.clone(),
                status: Status::Crashed,
                counts: Default::default(),
                failures: Vec::new(),
                duration: Duration::ZERO,
                output: format!("{err:#}"),
            });
            printer.assembly(&result);
            cache.put(
                &target,
                Entry {
                    fingerprint,
                    passed: result.status == Status::Passed,
                    counts: result.counts,
                    duration_ms: result.duration.as_millis() as u64,
                },
            );
            results.push(result);
        }
    });

    if let Err(err) = cache.save() {
        eprintln!("warning: cannot save the test cache: {err}");
    }
    printer.summary(&results, started.elapsed());
    let ok = results.iter().all(|r| matches!(r.status, Status::Passed | Status::Cached));
    Ok(if ok { ExitCode::SUCCESS } else { ExitCode::FAILURE })
}

/// Keeps rouille's state out of the repository, in the user cache directory.
fn state_dir(root: &Path) -> PathBuf {
    let base = std::env::var_os("ROUILLE_CACHE_DIR").map(PathBuf::from).unwrap_or_else(|| {
        let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
        if cfg!(target_os = "macos") {
            home.join("Library/Caches/rouille")
        } else if cfg!(windows) {
            std::env::var_os("LOCALAPPDATA").map_or(home, PathBuf::from).join("rouille")
        } else {
            std::env::var_os("XDG_CACHE_HOME").map_or(home.join(".cache"), PathBuf::from).join("rouille")
        }
    });
    let hash = blake3::hash(root.as_os_str().as_encoded_bytes()).to_hex();
    let name = root.file_name().unwrap_or_default().to_string_lossy();
    base.join(format!("{name}-{}", &hash[..12]))
}

fn rel(root: &Path, path: &Path) -> String {
    path.strip_prefix(root).unwrap_or(path).display().to_string()
}
