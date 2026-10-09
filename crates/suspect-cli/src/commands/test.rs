//! `suspect test` — compile an Arazzo suite and execute it against a live
//! server or a recorded cassette.

use std::path::Path;
use std::sync::Arc;

use super::http::LiveTransport;
use suspect_journal::Journal;
use suspect_source::Uri;
use suspect_test::reporters;
use suspect_test::transports::ReplayTransport;
use suspect_test::{HttpClient, TestEvent};

/// Runs `suspect test` against one Arazzo document.
///
/// # Errors
/// Propagates workspace/plan compilation failures and transport setup
/// errors; assertion failures surface through the exit code instead.
#[allow(clippy::too_many_arguments)]
pub fn test(
    arazzo: &Path,
    base_url: &str,
    filter: Option<&str>,
    offline_cassette: Option<&Path>,
    ndjson: bool,
) -> anyhow::Result<i32> {
    // The plain entry point discovers credentials like the CLI does, so
    // library callers inherit the same secure source without wiring.
    test_with_messages(
        arazzo,
        base_url,
        filter,
        &serde_json::Map::new(),
        offline_cassette,
        ndjson,
        None,
        None,
        None,
    )
}

/// Parses `--input name=value` flags into a workflow input map: each value
/// is parsed as JSON when it parses, else taken as a string.
pub fn parse_inputs(raw: &[String]) -> anyhow::Result<serde_json::Map<String, serde_json::Value>> {
    let mut out = serde_json::Map::new();
    for item in raw {
        let Some((name, value)) = item.split_once('=') else {
            return Err(anyhow::anyhow!("--input expects NAME=VALUE, got {item:?}"));
        };
        if name.is_empty() {
            return Err(anyhow::anyhow!("--input expects NAME=VALUE, got {item:?}"));
        }
        let parsed = serde_json::from_str(value)
            .unwrap_or_else(|_| serde_json::Value::String(value.to_owned()));
        out.insert(name.to_owned(), parsed);
    }
    Ok(out)
}

/// [`test`] with an optional message broker directory for Arazzo 1.1
/// AsyncAPI send/receive steps. The directory holds `inbox.jsonl`
/// (pre-recorded messages the workflow may receive) and collects
/// `outbox.jsonl` (what the workflow published).
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
) -> anyhow::Result<i32> {
    // Credentials: an explicit file wins; otherwise discover, and only
    // then the SUSPECT_CREDENTIALS override. No file is a valid state —
    // workflows whose operations declare no security run fine without
    // one — reported distinctly from a file that exists but fails.
    let auth = match credentials {
        Some(path) => Some(
            suspect_test::auth::load_credentials_file(path).map_err(|e| anyhow::anyhow!("{e}"))?,
        ),
        None => match std::env::var_os("SUSPECT_CREDENTIALS") {
            Some(env_path) => {
                let path = std::path::PathBuf::from(env_path);
                Some(
                    suspect_test::auth::load_credentials_file(&path)
                        .map_err(|e| anyhow::anyhow!("{e}"))?,
                )
            }
            None => {
                suspect_test::auth::discover_credentials(arazzo.parent().unwrap_or(Path::new(".")))
                    .map(|path| {
                        suspect_test::auth::load_credentials_file(&path)
                            .map_err(|e| anyhow::anyhow!("{e}"))
                    })
                    .transpose()?
            }
        },
    };
    let auth = auth.unwrap_or_default();
    let ws = super::workspace_for_entry(arazzo)?;
    let uri = Uri::from_path(arazzo)?;
    let handle = ws
        .get(&uri)
        .ok_or_else(|| anyhow::anyhow!("arazzo document not loaded"))?;
    let doc = handle.doc();
    let mut plan = suspect_test::compile_plan(doc, &ws)?;
    if let Some(want) = filter {
        plan.workflows.retain(|wf| wf.workflow_id.contains(want));
    }
    if plan.workflows.is_empty() {
        eprintln!("no workflows matched filter {filter:?}");
        return Ok(2);
    }

    rt_run(
        &plan,
        base_url,
        inputs,
        offline_cassette,
        ndjson,
        message_broker,
        &auth,
        journal,
    )
}

#[allow(clippy::too_many_arguments)]
fn rt_run(
    plan: &suspect_test::Plan,
    base_url: &str,
    inputs: &serde_json::Map<String, serde_json::Value>,
    offline_cassette: Option<&Path>,
    ndjson: bool,
    message_broker: Option<&Path>,
    auth: &suspect_test::auth::AuthConfig,
    journal: Option<&Path>,
) -> anyhow::Result<i32> {
    let rt = tokio::runtime::Runtime::new()?;
    let outcome = rt.block_on(async move {
        let http: Arc<dyn HttpClient> = match offline_cassette {
            Some(path) => {
                let file = std::fs::File::open(path)?;
                let (_, entries) = suspect_journal::read_cassette(file)?;
                Arc::new(ReplayTransport::new(entries))
            }
            None => Arc::new(LiveTransport::new(std::time::Duration::from_secs(30))?),
        };
        let broker: Option<Arc<dyn suspect_test::messaging::MessageTransport>> =
            match message_broker {
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
        let auth_state = suspect_test::auth::AuthState::default();
        let summary = suspect_test::run_plan_with_auth_and_messages(
            plan,
            base_url,
            http.as_ref(),
            &auth_state,
            auth,
            inputs,
            broker.as_deref(),
            tx,
        )
        .await;
        let events = drainer
            .await
            .map_err(|e| anyhow::anyhow!("drainer panicked: {e}"))?;
        Ok::<_, anyhow::Error>((summary, events))
    })?;
    let (summary, events) = outcome;
    print!("{}", reporters::console(&summary, &events));

    let mut journal = Journal::new(super::sink(journal)?);
    let [passed, failed, skipped] = [summary.passed, summary.failed, summary.skipped];
    journal.run_summary(
        "test",
        u32::try_from(passed).unwrap_or(u32::MAX),
        u32::try_from(failed).unwrap_or(u32::MAX),
        u32::try_from(skipped).unwrap_or(u32::MAX),
        summary.duration_ms as f64,
    );
    journal.flush()?;
    Ok(i32::from(summary.failed > 0))
}
