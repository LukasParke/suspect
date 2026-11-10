//! `suspect test stateful`: generate and run stateful dependency-graph test
//! sequences against a live server.
//!
//! The generator (`suspect-test::stateful`) auto-discovers resource
//! dependencies from path semantics — POST /users/{userId}/posts creates a
//! post that needs a user — builds the dependency DAG, and emits
//! setup → exercise → teardown sequences. This command runs them,
//! threading each step's created ids into the next step's path.

use std::path::Path;

use suspect_test::reporters::Case;
use suspect_test::stateful;

/// Runs `suspect test stateful`.
///
/// Every 2xx reply of a sequence is held to the response its operation
/// declares at `conformance`; a sequence fails on a non-2xx status, a
/// transport error, or a reply that breaks its contract.
///
/// # Errors
/// Workspace loading and IR compilation failures.
pub fn stateful(
    spec: &Path,
    base_url: &str,
    filter: Option<&str>,
    emit: bool,
    conformance: suspect_test::Conformance,
    report_files: &crate::ReportFiles,
) -> anyhow::Result<i32> {
    let ws = super::workspace_for_entry(spec)?;
    let uri = suspect_source::Uri::from_path(spec)?;
    ws.get(&uri)
        .ok_or_else(|| anyhow::anyhow!("spec document not loaded: {uri}"))?;
    let ir = suspect_ir::IrSpec::from_workspace(&ws, &uri)
        .map_err(|e| anyhow::anyhow!("IR compilation failed: {e}"))?;
    let mut sequences = stateful::generate_sequences(&ir);
    if sequences.is_empty() {
        eprintln!(
            "{}: no operation that creates, changes or deletes a resource, so there is \
             no stateful sequence to generate; `suspect test run` and `suspect test fuzz` \
             cover read-only operations",
            spec.display()
        );
        return Ok(2);
    }
    if let Some(want) = filter {
        sequences.retain(|seq| seq.target.id.as_deref().is_some_and(|id| id.contains(want)));
        if sequences.is_empty() {
            eprintln!("no sequence targets an operationId containing {want:?}");
            return Ok(2);
        }
    }

    if emit {
        println!(
            "{}",
            serde_json::to_string_pretty(&sequences).map_err(|e| anyhow::anyhow!("{e}"))?
        );
        return Ok(0);
    }

    let started = std::time::Instant::now();
    let rt = tokio::runtime::Runtime::new()?;
    let http = super::http::LiveTransport::new(std::time::Duration::from_secs(30))?;
    let schemas = suspect_test::Schemas::from_spec(&ir);
    let contract = stateful::SequenceContract {
        spec: &ir,
        schemas: &schemas,
        level: conformance,
    };
    let mut cases: Vec<Case> = Vec::with_capacity(sequences.len());
    rt.block_on(async {
        for seq in &sequences {
            let outcome = stateful::run_sequence_checked(seq, base_url, &http, contract).await;
            let label = &outcome.target;
            if outcome.passed {
                println!("{label:<44} steps: {} PASS", outcome.steps);
                cases.push(Case::passed("stateful", label));
            } else {
                let message = outcome.message.as_deref().unwrap_or("unknown failure");
                println!("{label:<44} steps: {} FAIL — {message}", outcome.steps);
                cases.push(Case::failed(
                    "stateful",
                    label,
                    "sequence",
                    "every step to succeed and conform",
                    message,
                ));
            }
        }
    });
    let failed = cases.iter().filter(|case| case.is_failed()).count();
    println!();
    println!(
        "stateful complete: {} sequences, {failed} failed",
        sequences.len()
    );
    report_files.write_cases("stateful", &spec.display().to_string(), &cases, started)?;
    Ok(i32::from(failed > 0))
}
