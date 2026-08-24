//! `suspect test` — compile an Arazzo suite and execute it against a live
//! server or a recorded cassette.

use std::path::Path;
use std::sync::Arc;

use suspect_journal::Journal;
use suspect_source::Uri;
use suspect_test::reporters;
use suspect_test::transports::ReplayTransport;
use suspect_test::{HttpClient, HttpRequest, HttpResponse, TestEvent, TransportError};

/// Live HTTP transport backed by `reqwest`; the CLI-side implementation of
/// [`HttpClient`] (the core crate stays dependency-free).
struct LiveTransport {
    client: reqwest::Client,
}

#[async_trait::async_trait]
impl HttpClient for LiveTransport {
    async fn execute(&self, req: HttpRequest) -> Result<HttpResponse, TransportError> {
        let method = reqwest::Method::from_bytes(req.method.as_bytes())
            .map_err(|e| TransportError(format!("bad method {}: {e}", req.method)))?;
        let mut builder = self.client.request(method, &req.url);
        for (k, v) in &req.headers {
            builder = builder.header(k.as_str(), v.as_str());
        }
        if !req.body.is_empty() {
            builder = builder.body(req.body.clone());
        }
        let resp = builder
            .send()
            .await
            .map_err(|e| TransportError(e.to_string()))?;
        let status = resp.status().as_u16();
        let headers: Vec<(String, String)> = resp
            .headers()
            .iter()
            .map(|(k, v)| (k.as_str().to_owned(), v.to_str().unwrap_or("").to_owned()))
            .collect();
        let body = resp
            .bytes()
            .await
            .map_err(|e| TransportError(e.to_string()))?;
        Ok(HttpResponse {
            status,
            headers,
            body,
        })
    }
}

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
) -> anyhow::Result<i32> {
    let ws = super::workspace_dir_all(arazzo)?;
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

    rt_run(&plan, base_url, offline_cassette)
}

fn rt_run(
    plan: &suspect_test::Plan,
    base_url: &str,
    offline_cassette: Option<&Path>,
) -> anyhow::Result<i32> {
    let rt = tokio::runtime::Runtime::new()?;
    let outcome = rt.block_on(async move {
        let http: Arc<dyn HttpClient> = match offline_cassette {
            Some(path) => {
                let file = std::fs::File::open(path)?;
                let (_, entries) = suspect_journal::read_cassette(file)?;
                Arc::new(ReplayTransport::new(entries))
            }
            None => Arc::new(LiveTransport {
                client: reqwest::Client::builder()
                    .timeout(std::time::Duration::from_secs(30))
                    .build()?,
            }),
        };
        let (tx, mut rx) = tokio::sync::mpsc::channel::<TestEvent>(256);
        // Drain + print events on a side task; run_plan's cooperative
        // workflow futures are deliberately non-Send, so it is awaited in
        // this task rather than spawned.
        let drainer = tokio::spawn(async move {
            let mut events = Vec::new();
            while let Some(event) = rx.recv().await {
                println!("{}", reporters::event_line(&event));
                events.push(event);
            }
            events
        });
        let summary = suspect_test::run_plan(&plan, base_url, http.as_ref(), tx).await;
        let events = drainer
            .await
            .map_err(|e| anyhow::anyhow!("drainer panicked: {e}"))?;
        Ok::<_, anyhow::Error>((summary, events))
    })?;
    let (summary, events) = outcome;
    print!("{}", reporters::console(&summary, &events));

    let mut journal = Journal::new(Box::new(suspect_journal::StdoutSink));
    let [passed, failed, skipped] = [summary.passed, summary.failed, summary.skipped];
    journal.run_summary(
        "test",
        u32::try_from(passed).unwrap_or(u32::MAX),
        u32::try_from(failed).unwrap_or(u32::MAX),
        u32::try_from(skipped).unwrap_or(u32::MAX),
        summary.duration_ms as f64,
    );
    Ok(i32::from(summary.failed > 0))
}
