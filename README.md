# rouille

A fast .NET toolchain, written in Rust. The first command is `rouille test`.

## `rouille test`

`rouille test` builds the test projects and runs them in parallel, one process per test assembly.

```sh
rouille test                      # current directory, solution or projects under it
rouille test path/to/App.slnx     # a solution or a project file
rouille test --filter Parser      # tests whose fully qualified name contains "Parser"
rouille test --list               # show the test assemblies and their runners
```

Options:

| Option | Effect |
|---|---|
| `--filter <text>` | Run only tests whose fully qualified name contains the text. |
| `-c, --configuration <name>` | Build configuration. The default is `Debug`. |
| `--no-build` | Use the existing build output. |
| `--no-cache` | Build and run everything, even when nothing changed. |
| `-j, --jobs <n>` | Number of assemblies that run at the same time. The default is the CPU count. |

Exit codes: `0` when all tests pass, `1` when a test fails, `2` when rouille cannot run.

### Runners

rouille finds the runner from the files in the build output:

| Runner | Detected by | How rouille runs it |
|---|---|---|
| Microsoft.Testing.Platform (MTP) | `Microsoft.Testing.Platform.dll` and no `testhost.dll` | `dotnet <dll>`, TRX report when the extension is present, else the console summary |
| xUnit v3 | `xunit.v3.runner.inproc.console.dll` | `dotnet <dll> -trx` |
| VSTest (xUnit v2, NUnit, MSTest) | `testhost.dll` | `dotnet vstest <dll> --logger:trx` |

### Cache

rouille keeps its state out of the repository, in the user cache directory: `~/Library/Caches/rouille` on macOS, `$XDG_CACHE_HOME/rouille` on Linux and `%LOCALAPPDATA%\rouille` on Windows. Set `ROUILLE_CACHE_DIR` to use another directory.

- When no file under the root changed since the last successful build, rouille does not start MSBuild.
- When an assembly's output directory, the filter and the environment did not change since it last passed, rouille does not run it again.

The build check covers the workspace root, every project the test projects reference, and the shared build files above them (`Directory.Build.props`, `Directory.Packages.props`, `global.json`, `NuGet.config`). Use `--no-cache` when another input changes, such as a data file outside these directories.

## Other commands

rouille sends any other command to `dotnet`. For example, `rouille new console` runs `dotnet new console`.

## Development

```sh
cargo test
cargo build --release
./target/release/rouille test tests/fixtures/sample
ROUILLE_FIXTURE_FAIL=1 ./target/release/rouille test tests/fixtures/sample   # one failing test per assembly
```
