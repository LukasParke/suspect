//! `cargo xtask ci [JOB...]`: the CI jobs from `.github/workflows/ci.yml`
//! and `platform.yml`, one subcommand per workflow job id, so a CI failure
//! names the local command to rerun. Workflows hold only toolchain setup and
//! call `cargo xtask ci <job> --ci`.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};

const USAGE: &str = "\
usage: cargo xtask ci [JOB...] [--list] [--skip-missing] [--fail-fast] [--ci]
  JOB             one or more job names (see --list); none = all, in table order
  --list          print the job table and exit
  --skip-missing  report a job whose tool is missing as SKIP instead of failing
  --fail-fast     stop at the first failing job (default: run all, then summarize)
  --ci            set by the workflows: forbids --skip-missing, uses the CI port
exit: 0 every job passed (or was skipped), 1 a job failed, 2 usage error or a
missing tool";

/// One CI job. `name` is the workflow job id.
pub(crate) struct Job {
    pub name: &'static str,
    pub workflow: &'static str,
    /// The commands, as shown by `--list`.
    pub commands: &'static str,
    pub tools: &'static [Tool],
    pub run: fn(&Ctx) -> Result<()>,
}

/// A tool a job needs before it can run.
#[derive(Clone, Copy)]
pub(crate) enum Tool {
    Rustfmt,
    Clippy,
    CargoDeny,
    Lychee,
    Node,
    Npm,
    Bun,
    Wasm32Wasip1,
}

impl Tool {
    fn name(self) -> &'static str {
        match self {
            Tool::Rustfmt => "rustfmt",
            Tool::Clippy => "clippy",
            Tool::CargoDeny => "cargo-deny",
            Tool::Lychee => "lychee",
            Tool::Node => "node >= 22",
            Tool::Npm => "npm",
            Tool::Bun => "bun",
            Tool::Wasm32Wasip1 => "rustup target wasm32-wasip1",
        }
    }

    fn install(self) -> &'static str {
        match self {
            Tool::Rustfmt => "rustup component add rustfmt",
            Tool::Clippy => "rustup component add clippy",
            Tool::CargoDeny => {
                "cargo install --locked cargo-deny (CI: taiki-e/install-action@v2, tool: cargo-deny)"
            }
            Tool::Lychee => {
                "install lychee 0.24.2 from https://lychee.cli.rs (CI: taiki-e/install-action@v2, tool: lychee)"
            }
            Tool::Node | Tool::Npm => {
                "install Node.js 22 or later from https://nodejs.org (CI: actions/setup-node@v4)"
            }
            Tool::Bun => "install from https://bun.sh (CI: oven-sh/setup-bun@v2)",
            Tool::Wasm32Wasip1 => "rustup target add wasm32-wasip1",
        }
    }

    /// `Ok(())` when the tool is usable, else what was found.
    fn probe(self, ctx: &Ctx) -> std::result::Result<(), String> {
        match self {
            Tool::Rustfmt => probe_ok(Command::new(&ctx.cargo).args(["fmt", "--version"])),
            Tool::Clippy => probe_ok(Command::new(&ctx.cargo).args(["clippy", "--version"])),
            Tool::CargoDeny => probe_ok(Command::new(&ctx.cargo).args(["deny", "--version"])),
            Tool::Lychee => probe_ok(Command::new("lychee").arg("--version")),
            Tool::Npm => probe_ok(Command::new(npm()).arg("--version")),
            Tool::Bun => probe_ok(Command::new("bun").arg("--version")),
            Tool::Node => {
                let out = probe_output(Command::new("node").arg("--version"))?;
                let major = out
                    .trim()
                    .trim_start_matches('v')
                    .split('.')
                    .next()
                    .and_then(|m| m.parse::<u32>().ok());
                match major {
                    Some(m) if m >= 22 => Ok(()),
                    _ => Err(format!("found node {}", out.trim())),
                }
            }
            Tool::Wasm32Wasip1 => {
                let out = probe_output(
                    Command::new("rustup")
                        .args(["target", "list", "--installed"])
                        .current_dir(ctx.root.join("editors/zed")),
                )?;
                if out.lines().any(|l| l.trim() == "wasm32-wasip1") {
                    Ok(())
                } else {
                    Err("not installed".into())
                }
            }
        }
    }
}

fn probe_output(cmd: &mut Command) -> std::result::Result<String, String> {
    match cmd.output() {
        Ok(out) if out.status.success() => Ok(String::from_utf8_lossy(&out.stdout).into_owned()),
        Ok(_) => Err("not working".into()),
        Err(_) => Err("not found".into()),
    }
}

fn probe_ok(cmd: &mut Command) -> std::result::Result<(), String> {
    probe_output(cmd).map(|_| ())
}

const CI_YML: &str = ".github/workflows/ci.yml";
const PLATFORM_YML: &str = ".github/workflows/platform.yml";

/// Every blocking CI job, in the order `cargo xtask ci` runs them.
pub(crate) const JOBS: &[Job] = &[
    Job {
        name: "fmt",
        workflow: CI_YML,
        commands: "cargo fmt --all --check",
        tools: &[Tool::Rustfmt],
        run: |ctx| ctx.cargo(&["fmt", "--all", "--check"]).run(),
    },
    Job {
        name: "clippy",
        workflow: CI_YML,
        commands: "cargo clippy --workspace --all-targets -- -D warnings",
        tools: &[Tool::Clippy],
        run: |ctx| {
            ctx.cargo(&[
                "clippy",
                "--workspace",
                "--all-targets",
                "--",
                "-D",
                "warnings",
            ])
            .run()
        },
    },
    Job {
        name: "docs",
        workflow: CI_YML,
        commands: "RUSTDOCFLAGS='-D warnings' cargo doc --workspace --no-deps",
        tools: &[],
        run: |ctx| {
            ctx.cargo(&["doc", "--workspace", "--no-deps"])
                .env("RUSTDOCFLAGS", "-D warnings")
                .run()
        },
    },
    Job {
        name: "cargo-deny",
        workflow: CI_YML,
        commands: "cargo deny --manifest-path ./Cargo.toml --workspace check",
        tools: &[Tool::CargoDeny],
        run: |ctx| {
            ctx.cargo(&[
                "deny",
                "--manifest-path",
                "./Cargo.toml",
                "--workspace",
                "check",
            ])
            .run()
        },
    },
    Job {
        name: "test",
        workflow: CI_YML,
        commands: "cargo test --workspace (with --ci: SUSPECT_REQUIRE_TOOLS=bun,node,python)",
        tools: &[],
        run: |ctx| {
            let mut step = ctx.cargo(&["test", "--workspace"]);
            if ctx.ci {
                // CI installs these, so a default-suite skip for them is a failure.
                step = step.env("SUSPECT_REQUIRE_TOOLS", "bun,node,python");
            }
            step.run()
        },
    },
    Job {
        name: "bench-smoke",
        workflow: CI_YML,
        commands: "cargo bench --workspace -- --test",
        tools: &[],
        run: |ctx| ctx.cargo(&["bench", "--workspace", "--", "--test"]).run(),
    },
    Job {
        name: "corpus-validation",
        workflow: CI_YML,
        commands: "fetch-corpus (network); SUSPECT_REQUIRE_CORPUS=1 cargo test -p suspect-cli --test corpus_suite",
        tools: &[],
        run: |ctx| {
            println!("$ cargo xtask fetch-corpus");
            crate::fetch_corpus(&[])?;
            ctx.cargo(&["test", "-p", "suspect-cli", "--test", "corpus_suite"])
                .env("SUSPECT_REQUIRE_CORPUS", "1")
                .run()
        },
    },
    Job {
        name: "links",
        workflow: CI_YML,
        commands: "lychee --config lychee.toml -q $(git ls-files '*.md') (offline, relative links)",
        tools: &[Tool::Lychee],
        run: |ctx| {
            let files = crate::tracked_files(&ctx.root, |path| path.ends_with(".md"))?;
            let mut args = vec!["--config", "lychee.toml", "-q"];
            args.extend(files.iter().map(String::as_str));
            let mut step = Step::new("lychee", &args).dir(&ctx.root);
            step.shown = "lychee --config lychee.toml -q <git ls-files '*.md'>".into();
            step.run()
        },
    },
    Job {
        name: "platform-smoke",
        workflow: PLATFORM_YML,
        commands: "cargo build --release -p suspect-cli; gateway mock + Arazzo suite live, then offline against a cassette",
        tools: &[],
        run: platform_smoke,
    },
    Job {
        name: "vscode-extension",
        workflow: PLATFORM_YML,
        commands: "cargo build -p suspect-cli; in editors/vscode: npm ci, npm run compile, npm run test:headless, SUSPECT_TEST_BINARY=target/debug/suspect npm run test:generation",
        tools: &[Tool::Node, Tool::Npm],
        run: |ctx| {
            ctx.cargo(&["build", "-p", "suspect-cli"]).run()?;
            let dir = ctx.root.join("editors/vscode");
            for args in [&["ci"][..], &["run", "compile"], &["run", "test:headless"]] {
                Step::new(npm(), args).dir(&dir).run()?;
            }
            // The generation tests drive the real CLI rather than a spawn mock.
            let binary = ctx
                .target_dir()
                .join("debug")
                .join(format!("suspect{}", std::env::consts::EXE_SUFFIX));
            Step::new(npm(), &["run", "test:generation"])
                .dir(&dir)
                .env("SUSPECT_TEST_BINARY", binary)
                .run()
        },
    },
    Job {
        name: "zed-extension",
        workflow: PLATFORM_YML,
        commands: "cargo build --locked --manifest-path editors/zed/Cargo.toml --target wasm32-wasip1",
        tools: &[Tool::Wasm32Wasip1],
        run: |ctx| {
            // The extension has its own [workspace] and lockfile; the root
            // lockfile is never touched.
            ctx.cargo(&[
                "build",
                "--locked",
                "--manifest-path",
                "editors/zed/Cargo.toml",
                "--target",
                "wasm32-wasip1",
            ])
            .run()
        },
    },
    Job {
        name: "rules-runtime",
        workflow: PLATFORM_YML,
        commands: "in rules-runtime: bun install, bun test",
        tools: &[Tool::Bun],
        run: |ctx| {
            let dir = ctx.root.join("rules-runtime");
            Step::new("bun", &["install"]).dir(&dir).run()?;
            Step::new("bun", &["test"]).dir(&dir).run()
        },
    },
];

/// What every job runs against.
pub(crate) struct Ctx {
    pub root: PathBuf,
    pub ci: bool,
    cargo: OsString,
}

impl Ctx {
    fn new(root: PathBuf, ci: bool) -> Self {
        // `cargo xtask` sets CARGO to the cargo that launched it (and
        // RUSTUP_TOOLCHAIN for `cargo +toolchain`), so every step uses the
        // same toolchain.
        let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
        Self { root, ci, cargo }
    }

    fn cargo(&self, args: &[&str]) -> Step {
        Step::new(self.cargo.clone(), args).dir(&self.root)
    }

    fn target_dir(&self) -> PathBuf {
        std::env::var_os("CARGO_TARGET_DIR")
            .map(PathBuf::from)
            .map(|dir| self.root.join(dir))
            .unwrap_or_else(|| self.root.join("target"))
    }
}

fn npm() -> &'static str {
    if cfg!(windows) { "npm.cmd" } else { "npm" }
}

/// One command of a job, echoed before it runs.
struct Step {
    cmd: Command,
    shown: String,
}

impl Step {
    fn new(program: impl Into<OsString>, args: &[&str]) -> Self {
        let program = program.into();
        let name = Path::new(&program)
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let mut cmd = Command::new(&program);
        cmd.args(args);
        let shown = std::iter::once(name.as_str())
            .chain(args.iter().copied())
            .collect::<Vec<_>>()
            .join(" ");
        Self { cmd, shown }
    }

    fn dir(mut self, dir: &Path) -> Self {
        self.cmd.current_dir(dir);
        self
    }

    fn env(mut self, key: &str, value: impl Into<OsString>) -> Self {
        let value = value.into();
        self.shown = format!("{key}={} {}", value.to_string_lossy(), self.shown);
        self.cmd.env(key, value);
        self
    }

    fn run(mut self) -> Result<()> {
        println!("$ {}", self.shown);
        let status = self
            .cmd
            .status()
            .with_context(|| format!("spawn `{}`", self.shown))?;
        if !status.success() {
            bail!("`{}` exited with {status}", self.shown);
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// platform-smoke
// ---------------------------------------------------------------------------

/// The CI port; locally a free port is used so a running gateway does not
/// collide.
const CI_SMOKE_PORT: u16 = 8091;

const SMOKE_PETSTORE: &str = r#"openapi: "3.0.0"
info:
  title: Smoke Petstore
  version: "1.0.0"
servers:
  - url: http://127.0.0.1:{PORT}
paths:
  /pets/{petId}:
    get:
      operationId: showPetById
      parameters:
        - name: petId
          in: path
          required: true
          schema:
            type: integer
      responses:
        '200':
          description: one pet
          content:
            application/json:
              schema:
                $ref: '#/components/schemas/Pet'
components:
  schemas:
    Pet:
      type: object
      required: [id, name]
      properties:
        id:
          type: integer
        name:
          type: string
        tag:
          type: string
"#;

const SMOKE_SUITE: &str = r#"arazzo: 1.0.0
info:
  title: smoke flows
  version: "1.0"
sourceDescriptions:
  - name: petstore
    url: petstore.yaml
workflows:
  - workflowId: fetch-pet
    steps:
      - stepId: show-pet
        operationId: showPetById
        parameters:
          - name: petId
            in: path
            value: 1
        successCriteria:
          - condition: '{$statusCode} == 200'
"#;

/// Kills the gateway when the job ends, however it ends.
struct Gateway(std::process::Child);

impl Drop for Gateway {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn platform_smoke(ctx: &Ctx) -> Result<()> {
    ctx.cargo(&["build", "--release", "-p", "suspect-cli"])
        .run()?;
    let suspect = ctx
        .target_dir()
        .join("release")
        .join(format!("suspect{}", std::env::consts::EXE_SUFFIX));
    let dir = ctx.target_dir().join("xtask-ci/platform-smoke");
    if dir.exists() {
        std::fs::remove_dir_all(&dir).with_context(|| format!("clear {}", dir.display()))?;
    }
    std::fs::create_dir_all(&dir).with_context(|| format!("create {}", dir.display()))?;

    let port = if ctx.ci { CI_SMOKE_PORT } else { free_port()? };
    let base = format!("http://127.0.0.1:{port}");
    std::fs::write(
        dir.join("petstore.yaml"),
        SMOKE_PETSTORE.replace("{PORT}", &port.to_string()),
    )?;
    let suite = dir.join("suite.arazzo.yaml");
    std::fs::write(&suite, SMOKE_SUITE)?;

    let log_path = dir.join("gateway.log");
    let log = std::fs::File::create(&log_path)?;
    println!("$ suspect gateway petstore.yaml --port {port} --mode mock &");
    let gateway = Gateway(
        Command::new(&suspect)
            .arg("gateway")
            .arg(dir.join("petstore.yaml"))
            .args(["--port", &port.to_string(), "--mode", "mock"])
            .stdout(log.try_clone()?)
            .stderr(log)
            .spawn()
            .with_context(|| format!("spawn {}", suspect.display()))?,
    );
    let result = smoke_against(&suspect, &dir, &suite, &base);
    drop(gateway);
    if result.is_err() {
        let log = std::fs::read_to_string(&log_path).unwrap_or_default();
        let tail = log.lines().rev().take(20).collect::<Vec<_>>();
        eprintln!("gateway log (last lines):");
        for line in tail.into_iter().rev() {
            eprintln!("  {line}");
        }
    }
    result
}

fn smoke_against(suspect: &Path, dir: &Path, suite: &Path, base: &str) -> Result<()> {
    wait_ready(&format!("{base}/pets/1"))?;

    println!("$ suspect test suite.arazzo.yaml --base-url {base} --report ndjson");
    let live = Command::new(suspect)
        .arg("test")
        .arg(suite)
        .args(["--base-url", base, "--report", "ndjson"])
        .output()
        .context("run suspect test (live)")?;
    check_run_done("live", &live)?;

    let cassette = dir.join("smoke.cassette");
    std::fs::write(&cassette, smoke_cassette(base))?;
    println!(
        "$ suspect test suite.arazzo.yaml --base-url {base} --offline --cassette smoke.cassette --report ndjson"
    );
    let offline = Command::new(suspect)
        .arg("test")
        .arg(suite)
        .args(["--base-url", base, "--offline", "--cassette"])
        .arg(&cassette)
        .args(["--report", "ndjson"])
        .output()
        .context("run suspect test (offline)")?;
    check_run_done("offline", &offline)
}

fn free_port() -> Result<u16> {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").context("bind a free port")?;
    Ok(listener.local_addr()?.port())
}

/// Polls `url` until it answers 200, for up to 30 seconds.
fn wait_ready(url: &str) -> Result<()> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(2)))
        .build()
        .new_agent();
    for _ in 0..30 {
        if let Ok(response) = agent.get(url).call()
            && response.status().as_u16() == 200
        {
            println!("gateway ready: GET {url} -> 200");
            return Ok(());
        }
        std::thread::sleep(Duration::from_secs(1));
    }
    bail!("gateway did not answer GET {url} with 200 within 30s")
}

/// The NDJSON stream must carry `run_done` with one pass and no failure
/// (the human report and the run summary follow it on stdout).
fn check_run_done(label: &str, output: &std::process::Output) -> Result<()> {
    let stdout = String::from_utf8_lossy(&output.stdout);
    print!("{stdout}");
    eprint!("{}", String::from_utf8_lossy(&output.stderr));
    let done = stdout
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .rfind(|record| record["event"] == "run_done")
        .with_context(|| format!("{label}: no run_done event in the ndjson report"))?;
    if done["passed"] != 1 || done["failed"] != 0 || !output.status.success() {
        bail!(
            "{label}: expected run_done with passed 1, failed 0 and success; got {done} ({})",
            output.status
        );
    }
    Ok(())
}

/// A one-entry Suspect Cassette answering `GET /pets/1`.
fn smoke_cassette(base: &str) -> String {
    use sha2::{Digest, Sha256};
    let body_obj = |raw: &[u8]| {
        let sha: String = Sha256::digest(raw)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        serde_json::json!({
            "encoding": "utf8",
            "content": String::from_utf8_lossy(raw),
            "sha256": sha,
        })
    };
    let body = serde_json::json!({"id": 1, "name": "Rex", "tag": "happy"}).to_string();
    let header = serde_json::json!({
        "format": "suspect-cassette",
        "version": 1,
        "recorded_at_ms": 0,
        "source": "platform-smoke",
    });
    let entry = serde_json::json!({
        "id": 1,
        "method": "GET",
        "url": format!("{base}/pets/1"),
        "status": 200,
        "request_headers": [],
        "request_body": body_obj(b""),
        "response_headers": [["content-type", "application/json"]],
        "response_body": body_obj(body.as_bytes()),
        "duration_ms": 1.0,
    });
    format!("{header}\n{entry}\n")
}

#[derive(Debug, Default, PartialEq, Eq)]
struct Options {
    jobs: Vec<&'static str>,
    list: bool,
    skip_missing: bool,
    fail_fast: bool,
    ci: bool,
}

fn parse_args(args: &[String]) -> Result<Options> {
    let mut opts = Options::default();
    for arg in args {
        match arg.as_str() {
            "--list" => opts.list = true,
            "--skip-missing" => opts.skip_missing = true,
            "--fail-fast" => opts.fail_fast = true,
            "--ci" => opts.ci = true,
            flag if flag.starts_with('-') => bail!("unknown flag {flag:?}"),
            name => match JOBS.iter().find(|job| job.name == name) {
                Some(job) if !opts.jobs.contains(&job.name) => opts.jobs.push(job.name),
                Some(_) => {}
                None => bail!(
                    "unknown job {name:?}; jobs: {}",
                    JOBS.iter().map(|j| j.name).collect::<Vec<_>>().join(", ")
                ),
            },
        }
    }
    if opts.ci && opts.skip_missing {
        bail!("--skip-missing is not allowed with --ci: CI never reports a skipped job as passed");
    }
    if opts.jobs.is_empty() {
        opts.jobs = JOBS.iter().map(|job| job.name).collect();
    }
    Ok(opts)
}

#[derive(Debug)]
enum Outcome {
    Ok(Duration),
    Failed(Duration, String),
    /// A required tool is missing (an environment error).
    Missing(String),
    /// Missing tool with `--skip-missing`.
    Skipped(String),
}

/// 1 when a job failed, else 2 when a tool was missing, else 0.
fn exit_code(outcomes: &[Outcome]) -> i32 {
    if outcomes.iter().any(|o| matches!(o, Outcome::Failed(..))) {
        1
    } else if outcomes.iter().any(|o| matches!(o, Outcome::Missing(_))) {
        2
    } else {
        0
    }
}

fn summary(results: &[(&str, Outcome)]) -> String {
    let width = results.iter().map(|(n, _)| n.len()).max().unwrap_or(0) + 3;
    let mut out = String::new();
    let (mut ok, mut failed, mut missing, mut skipped) = (0, 0, 0, 0);
    for (name, outcome) in results {
        let dots = ".".repeat(width - name.len());
        let line = match outcome {
            Outcome::Ok(d) => {
                ok += 1;
                format!("ok       ({:.1}s)", d.as_secs_f64())
            }
            Outcome::Failed(d, e) => {
                failed += 1;
                format!("FAILED   ({:.1}s) {e}", d.as_secs_f64())
            }
            Outcome::Missing(why) => {
                missing += 1;
                format!("MISSING  {why}")
            }
            Outcome::Skipped(why) => {
                skipped += 1;
                format!("SKIP     {why}")
            }
        };
        out.push_str(&format!("ci: {name} {dots} {line}\n"));
    }
    out.push_str(&format!(
        "ci: {ok} ok, {failed} failed, {missing} missing a tool, {skipped} skipped."
    ));
    if missing > 0 {
        out.push_str(" Install the missing tools, or pass --skip-missing.");
    }
    if skipped > 0 {
        out.push_str(" A run with skips is not what CI runs.");
    }
    out
}

fn list() -> String {
    let width = JOBS.iter().map(|j| j.name.len()).max().unwrap_or(0);
    let mut out = String::new();
    for job in JOBS {
        out.push_str(&format!(
            "{:width$}  {}\n{:width$}  {}\n",
            job.name, job.workflow, "", job.commands
        ));
        if !job.tools.is_empty() {
            let tools = job.tools.iter().map(|t| t.name()).collect::<Vec<_>>();
            out.push_str(&format!("{:width$}  needs: {}\n", "", tools.join(", ")));
        }
    }
    out
}

/// Entry point for `cargo xtask ci`.
pub fn run(args: &[String]) -> Result<()> {
    if args.iter().any(|a| a == "-h" || a == "--help") {
        println!("{USAGE}");
        return Ok(());
    }
    let opts = match parse_args(args) {
        Ok(opts) => opts,
        Err(error) => {
            eprintln!("ci: {error}\n\n{USAGE}");
            std::process::exit(2);
        }
    };
    if opts.list {
        print!("{}", list());
        return Ok(());
    }
    let root = crate::repo_root();
    // fetch-corpus and the job commands use repository-relative paths.
    std::env::set_current_dir(&root).with_context(|| format!("cd {}", root.display()))?;
    let ctx = Ctx::new(root, opts.ci);
    let mut results = Vec::new();
    for name in &opts.jobs {
        let job = JOBS.iter().find(|j| j.name == *name).expect("parsed job");
        println!("\n==> ci: {}", job.name);
        let missing = job
            .tools
            .iter()
            .filter_map(|tool| {
                tool.probe(&ctx)
                    .err()
                    .map(|found| format!("{} {found}; {}", tool.name(), tool.install()))
            })
            .collect::<Vec<_>>();
        let outcome = if !missing.is_empty() {
            let why = missing.join("; ");
            if opts.skip_missing {
                println!("skip: {why}");
                Outcome::Skipped(why)
            } else {
                eprintln!("error: {why}");
                Outcome::Missing(why)
            }
        } else {
            let start = Instant::now();
            match (job.run)(&ctx) {
                Ok(()) => Outcome::Ok(start.elapsed()),
                Err(error) => {
                    eprintln!("error: {error:#}");
                    Outcome::Failed(start.elapsed(), format!("{error:#}"))
                }
            }
        };
        let stop = opts.fail_fast && matches!(outcome, Outcome::Failed(..) | Outcome::Missing(_));
        results.push((job.name, outcome));
        if stop {
            break;
        }
    }
    println!("\n{}", summary(&results));
    let outcomes = results.into_iter().map(|(_, o)| o).collect::<Vec<_>>();
    match exit_code(&outcomes) {
        0 => Ok(()),
        code => std::process::exit(code),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn no_jobs_means_all_in_table_order() {
        let opts = parse_args(&[]).unwrap();
        assert_eq!(opts.jobs, JOBS.iter().map(|j| j.name).collect::<Vec<_>>());
        assert!(!opts.ci && !opts.skip_missing && !opts.fail_fast && !opts.list);
    }

    #[test]
    fn named_jobs_keep_the_given_order_and_dedupe() {
        let opts = parse_args(&args(&["test", "clippy", "test", "--fail-fast"])).unwrap();
        assert_eq!(opts.jobs, vec!["test", "clippy"]);
        assert!(opts.fail_fast);
    }

    #[test]
    fn unknown_jobs_and_flags_are_usage_errors() {
        let error = parse_args(&args(&["lint"])).unwrap_err().to_string();
        assert!(error.contains("unknown job \"lint\""), "{error}");
        assert!(error.contains("fmt, clippy"), "{error}");
        assert!(parse_args(&args(&["--nope"])).is_err());
    }

    #[test]
    fn ci_forbids_skip_missing() {
        let error = parse_args(&args(&["--ci", "--skip-missing"]))
            .unwrap_err()
            .to_string();
        assert!(error.contains("--skip-missing"), "{error}");
        assert!(parse_args(&args(&["fmt", "--ci"])).unwrap().ci);
    }

    #[test]
    fn exit_code_follows_contributing() {
        let ok = || Outcome::Ok(Duration::ZERO);
        assert_eq!(exit_code(&[ok(), Outcome::Skipped("bun".into())]), 0);
        assert_eq!(exit_code(&[ok(), Outcome::Missing("bun".into())]), 2);
        assert_eq!(
            exit_code(&[
                Outcome::Missing("bun".into()),
                Outcome::Failed(Duration::ZERO, "x".into())
            ]),
            1
        );
        assert_eq!(exit_code(&[]), 0);
    }

    #[test]
    fn summary_counts_and_warns_about_skips() {
        let text = summary(&[
            ("fmt", Outcome::Ok(Duration::from_millis(2100))),
            ("rules-runtime", Outcome::Skipped("bun not found".into())),
        ]);
        assert!(
            text.contains("ci: fmt ............. ok       (2.1s)"),
            "{text}"
        );
        assert!(text.contains("SKIP     bun not found"), "{text}");
        assert!(
            text.ends_with(
                "ci: 1 ok, 0 failed, 0 missing a tool, 1 skipped. A run with skips is not what CI runs."
            ),
            "{text}"
        );
    }

    #[test]
    fn job_names_are_unique_and_listed() {
        let listed = list();
        for (i, job) in JOBS.iter().enumerate() {
            assert!(JOBS[..i].iter().all(|j| j.name != job.name), "{}", job.name);
            assert!(listed.contains(job.name));
        }
    }
    #[test]
    fn smoke_cassette_is_two_json_lines_with_matching_digest() {
        use sha2::{Digest, Sha256};
        let text = smoke_cassette("http://127.0.0.1:1");
        let lines = text.lines().collect::<Vec<_>>();
        assert_eq!(lines.len(), 2);
        let entry: serde_json::Value = serde_json::from_str(lines[1]).unwrap();
        assert_eq!(entry["url"], "http://127.0.0.1:1/pets/1");
        let content = entry["response_body"]["content"].as_str().unwrap();
        let sha: String = Sha256::digest(content.as_bytes())
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        assert_eq!(entry["response_body"]["sha256"], sha.as_str());
    }
}
