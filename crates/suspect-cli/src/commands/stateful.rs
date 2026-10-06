//! `suspect stateful`: generate and run stateful dependency-graph test
//! sequences against a live server.
//!
//! The generator (`suspect-test::stateful`) auto-discovers resource
//! dependencies from path semantics — POST /users/{userId}/posts creates a
//! post that needs a user — builds the dependency DAG, and emits
//! setup → exercise → teardown sequences. This command runs them,
//! threading each step's created ids into the next step's path.

use std::path::Path;

use suspect_test::stateful;

/// Runs `suspect stateful`.
///
/// # Errors
/// Workspace loading and IR compilation failures.
pub fn stateful(
    spec: &Path,
    base_url: &str,
    filter: Option<&str>,
    emit: bool,
) -> anyhow::Result<i32> {
    let ws = super::workspace_for_entry(spec)?;
    let uri = suspect_source::Uri::from_path(spec)?;
    ws.get(&uri)
        .ok_or_else(|| anyhow::anyhow!("spec document not loaded: {uri}"))?;
    let ir = suspect_ir::IrSpec::from_workspace(&ws, &uri)
        .map_err(|e| anyhow::anyhow!("IR compilation failed: {e}"))?;
    let mut sequences = stateful::generate_sequences(&ir);
    if let Some(want) = filter {
        sequences.retain(|seq| seq.target.id.as_deref().is_some_and(|id| id.contains(want)));
    }
    if sequences.is_empty() {
        eprintln!("no sequences matched filter {filter:?}");
        return Ok(2);
    }

    if emit {
        println!(
            "{}",
            serde_json::to_string_pretty(&sequences).map_err(|e| anyhow::anyhow!("{e}"))?
        );
        return Ok(0);
    }

    let rt = tokio::runtime::Runtime::new()?;
    let http = super::http::LiveTransport::new(std::time::Duration::from_secs(30))?;
    let mut passed = 0usize;
    let mut failed = 0usize;
    rt.block_on(async {
        for seq in &sequences {
            let outcome = stateful::run_sequence(seq, base_url, &http).await;
            let label = &outcome.target;
            if outcome.passed {
                passed += 1;
                println!("{label:<44} steps: {} PASS", outcome.steps);
            } else {
                failed += 1;
                println!(
                    "{label:<44} steps: {} FAIL — {}",
                    outcome.steps,
                    outcome.message.as_deref().unwrap_or("unknown failure")
                );
            }
        }
    });
    println!();
    println!(
        "stateful complete: {} sequences, {failed} failed",
        sequences.len()
    );
    Ok(i32::from(failed > 0))
}
