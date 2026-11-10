//! `suspect test run` — compile Arazzo suites and execute them against a
//! live server or a recorded cassette.
//!
//! One invocation runs any number of Arazzo documents as one gate: each
//! document is compiled against the OpenAPI descriptions it names, its
//! workflows run, and every reply is held to the declared contract at the
//! configured [`Conformance`] level. The exit code is `1` when any step of
//! any document fails.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::http::LiveTransport;
use suspect_journal::Journal;
use suspect_source::Uri;
use suspect_test::reporters::{self, Case};
use suspect_test::transports::ReplayTransport;
use suspect_test::{Conformance, HttpClient, RunAuth, RunOpts, RunSummary, TestEvent};

/// File-name endings that mark a document as an Arazzo suite when a
/// directory is searched.
const SUITE_SUFFIXES: [&str; 3] = [".arazzo.yaml", ".arazzo.yml", ".arazzo.json"];

/// How every document of one `suspect test run` invocation is run.
#[derive(Debug, Clone, Copy)]
pub struct Suite<'a> {
    /// Base URL prepended to operation paths.
    pub base_url: &'a str,
    /// Run only workflows whose id contains this substring.
    pub filter: Option<&'a str>,
    /// Workflow inputs supplied to every workflow that declares them.
    pub inputs: &'a serde_json::Map<String, serde_json::Value>,
    /// Run offline against this recorded cassette instead of live HTTP.
    pub cassette: Option<&'a Path>,
    /// Print the event stream as one JSON object per line.
    pub ndjson: bool,
    /// Message broker directory for Arazzo 1.1 AsyncAPI steps.
    pub message_broker: Option<&'a Path>,
    /// Credentials file; `None` discovers `.suspect/credentials.json`.
    pub credentials: Option<&'a Path>,
    /// Append the run summary to this journal file.
    pub journal: Option<&'a Path>,
    /// How much of the declared response contract each reply is held to.
    pub conformance: Conformance,
}

/// Parses `--input name=value` flags into a workflow input map: each value
/// is parsed as JSON when it parses, else taken as a string.
///
/// # Errors
/// Returns an error for a flag without `=` or with an empty name.
pub fn parse_inputs(raw: &[String]) -> anyhow::Result<serde_json::Map<String, serde_json::Value>> {
    super::inputs::parse_inputs(None, raw)
}

/// Resolves the credentials a run injects, the way every testing flow
/// does: an explicit file wins; otherwise `SUSPECT_CREDENTIALS`; otherwise
/// `.suspect/credentials.json` discovered by walking up from the Arazzo
/// document. No file is a valid state — workflows whose operations declare
/// no security run fine without one — reported distinctly from a file that
/// exists but fails to load.
///
/// # Errors
/// Propagates a credentials file that cannot be read, parsed or
/// interpolated.
pub fn load_credentials(
    arazzo: &Path,
    credentials: Option<&Path>,
) -> anyhow::Result<suspect_test::auth::AuthConfig> {
    let auth = match credentials {
        Some(path) => Some(
            suspect_test::auth::load_credentials_file(path).map_err(|e| anyhow::anyhow!("{e}"))?,
        ),
        // `test.credentials` (and `SUSPECT_CREDENTIALS`) arrive as
        // `credentials`, resolved by the caller.
        None => suspect_test::auth::discover_credentials(arazzo.parent().unwrap_or(Path::new(".")))
            .map(|path| {
                suspect_test::auth::load_credentials_file(&path).map_err(|e| anyhow::anyhow!("{e}"))
            })
            .transpose()?,
    };
    Ok(auth.unwrap_or_default())
}

/// The Arazzo documents `targets` name: each file as given, and every
/// suite document under each directory (searched recursively, hidden
/// entries skipped), in a stable order.
///
/// # Errors
/// A directory that holds no suite document, or one that cannot be read.
pub fn discover(targets: &[PathBuf]) -> anyhow::Result<Vec<PathBuf>> {
    let mut documents = Vec::new();
    for target in targets {
        if !target.is_dir() {
            documents.push(target.clone());
            continue;
        }
        let mut found = Vec::new();
        collect_suites(target, &mut found)?;
        if found.is_empty() {
            anyhow::bail!(
                "no Arazzo documents (*{}) under {}",
                SUITE_SUFFIXES.join(", *"),
                target.display()
            );
        }
        found.sort();
        documents.extend(found);
    }
    Ok(documents)
}

fn collect_suites(directory: &Path, found: &mut Vec<PathBuf>) -> anyhow::Result<()> {
    let entries = std::fs::read_dir(directory)
        .map_err(|e| anyhow::anyhow!("cannot read {}: {e}", directory.display()))?;
    for entry in entries {
        let entry =
            entry.map_err(|e| anyhow::anyhow!("cannot read {}: {e}", directory.display()))?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with('.') {
            continue;
        }
        let path = entry.path();
        if path.is_dir() {
            collect_suites(&path, found)?;
        } else if SUITE_SUFFIXES.iter().any(|suffix| name.ends_with(suffix)) {
            found.push(path);
        }
    }
    Ok(())
}

/// A path as reports show it: as given, with `/` separators.
fn shown(path: &Path) -> String {
    let text = path.to_string_lossy().replace('\\', "/");
    text.strip_prefix("./").map_or(text.clone(), str::to_owned)
}

/// Runs one Arazzo document with the default reports: no report files,
/// strict conformance. The entry point of the project and CI stages.
///
/// # Errors
/// As [`run`].
#[allow(clippy::too_many_arguments)]
pub fn test_with_messages(
    arazzo: &Path,
    base_url: &str,
    filter: Option<&str>,
    inputs: &serde_json::Map<String, serde_json::Value>,
    offline_cassette: Option<&Path>,
    ndjson: bool,
    message_broker: Option<&Path>,
    credentials: Option<&Path>,
    journal: Option<&Path>,
    conformance: Conformance,
) -> anyhow::Result<i32> {
    run(
        &[arazzo.to_path_buf()],
        &Suite {
            base_url,
            filter,
            inputs,
            cassette: offline_cassette,
            ndjson,
            message_broker,
            credentials,
            journal,
            conformance,
        },
        &crate::ReportFiles::default(),
    )
}

/// Runs every document `targets` name as one gate and writes the requested
/// report files.
///
/// Returns `0` when every step of every document passed, `1` when any
/// failed, and `2` when nothing could run: no document, or no workflow
/// matching the filter.
///
/// # Errors
/// Propagates workspace/plan compilation failures and transport setup
/// errors; assertion failures surface through the exit code instead.
pub fn run(
    targets: &[PathBuf],
    suite: &Suite<'_>,
    report_files: &crate::ReportFiles,
) -> anyhow::Result<i32> {
    let documents = match discover(targets) {
        Ok(documents) => documents,
        Err(error) => {
            eprintln!("{error}");
            return Ok(2);
        }
    };
    let several = documents.len() > 1;
    let mut total = RunSummary::default();
    let mut cases: Vec<Case> = Vec::new();
    let mut ran = 0usize;
    for document in &documents {
        let name = shown(document);
        if several && !suite.ndjson {
            println!("== {name} ==");
        }
        let Some((summary, document_cases)) = run_document(document, suite)? else {
            continue;
        };
        ran += 1;
        total.passed += summary.passed;
        total.failed += summary.failed;
        total.skipped += summary.skipped;
        total.duration_ms += summary.duration_ms;
        cases.extend(document_cases.into_iter().map(|case| Case {
            // Workflow ids are unique per document, not across documents.
            suite: if several {
                format!("{name}: {}", case.suite)
            } else {
                case.suite
            },
            ..case
        }));
    }
    if ran == 0 {
        eprintln!(
            "no workflows matched filter {:?}",
            suite.filter.unwrap_or("")
        );
        return Ok(2);
    }
    if several && !suite.ndjson {
        println!(
            "total: {} passed, {} failed, {} skipped across {ran} documents",
            total.passed, total.failed, total.skipped
        );
    }

    let mut journal = Journal::new(super::run_sink(suite.journal, suite.ndjson)?);
    let [passed, failed, skipped] = [total.passed, total.failed, total.skipped];
    journal.run_summary(
        "test",
        u32::try_from(passed).unwrap_or(u32::MAX),
        u32::try_from(failed).unwrap_or(u32::MAX),
        u32::try_from(skipped).unwrap_or(u32::MAX),
        total.duration_ms as f64,
    );
    journal.flush()?;
    let report_name = match documents.as_slice() {
        [only] => shown(only),
        _ => "suspect test run".to_owned(),
    };
    report_files.write("test", &report_name, &total, &cases)?;
    Ok(i32::from(total.failed > 0))
}

/// Compiles and runs one document. `None` when the filter leaves it no
/// workflow to run.
fn run_document(
    arazzo: &Path,
    suite: &Suite<'_>,
) -> anyhow::Result<Option<(RunSummary, Vec<Case>)>> {
    let auth = load_credentials(arazzo, suite.credentials)?;
    let ws = super::workspace_for_entry(arazzo)?;
    let uri = Uri::from_path(arazzo)?;
    let handle = ws
        .get(&uri)
        .ok_or_else(|| anyhow::anyhow!("arazzo document not loaded"))?;
    let doc = handle.doc();
    let mut plan = suspect_test::compile_plan(doc, &ws)?;
    if let Some(want) = suite.filter {
        plan.workflows.retain(|wf| wf.workflow_id.contains(want));
    }
    if plan.workflows.is_empty() {
        return Ok(None);
    }

    let opts = RunOpts {
        inputs: suite.inputs.clone(),
        auth: Arc::new(RunAuth::new(auth)),
        conformance: suite.conformance,
        ..RunOpts::new(suite.base_url)
    };
    let ndjson = suite.ndjson;
    let rt = tokio::runtime::Runtime::new()?;
    let (summary, events) = rt.block_on(async {
        let http: Arc<dyn HttpClient> = match suite.cassette {
            Some(path) => {
                let file = std::fs::File::open(path)
                    .map_err(|e| anyhow::anyhow!("cannot open cassette {}: {e}", path.display()))?;
                let (_, entries) = suspect_journal::read_cassette(file)?;
                Arc::new(ReplayTransport::new(entries))
            }
            None => Arc::new(LiveTransport::new(std::time::Duration::from_secs(30))?),
        };
        let broker: Option<Arc<dyn suspect_test::messaging::MessageTransport>> =
            match suite.message_broker {
                Some(dir) => Some(Arc::new(
                    suspect_test::messaging::FileBroker::open(dir)
                        .map_err(|e| anyhow::anyhow!("{e}"))?,
                )),
                None => None,
            };
        let (tx, mut rx) = tokio::sync::mpsc::channel::<TestEvent>(256);
        // Drain + print events on a side task; run_plan's cooperative
        // workflow futures are deliberately non-Send, so it is awaited in
        // this task rather than spawned.
        let drainer = tokio::spawn(async move {
            let mut events = Vec::new();
            while let Some(event) = rx.recv().await {
                if ndjson {
                    println!(
                        "{}",
                        serde_json::to_string(&event).unwrap_or_else(|_| "{}".to_owned())
                    );
                } else {
                    println!("{}", reporters::event_line(&event));
                }
                events.push(event);
            }
            events
        });
        let summary =
            suspect_test::run_plan_with_broker(&plan, &opts, http.as_ref(), broker.as_deref(), tx)
                .await;
        let events = drainer
            .await
            .map_err(|e| anyhow::anyhow!("drainer panicked: {e}"))?;
        Ok::<_, anyhow::Error>((summary, events))
    })?;
    if !ndjson {
        print!("{}", reporters::console(&summary, &events));
    }
    Ok(Some((summary, reporters::workflow_cases(&plan, &events))))
}
