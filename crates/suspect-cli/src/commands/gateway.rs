//! `suspect gateway` — run the local mock/proxy/validate/record/replay
//! server against a spec.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use suspect_gateway::{FaultConfig, GatewayConfig, Mode};
use suspect_journal::Journal;

/// Resolves the upstream URL or fails with a mode-specific message.
fn require_upstream(upstream: Option<&PathBuf>) -> anyhow::Result<String> {
    upstream
        .map(|p| p.to_string_lossy().into_owned())
        .ok_or_else(|| anyhow::anyhow!("--upstream http://host[:port] is required for this mode"))
}

/// Runs `suspect gateway` until interrupted.
///
/// # Errors
/// Propagates bind/spec-loading failures from the gateway.
#[allow(clippy::too_many_arguments)]
pub fn gateway(
    spec: &Path,
    port: u16,
    mode: &str,
    upstream: Option<&PathBuf>,
    cassette: Option<&PathBuf>,
    enforce: bool,
    delay_ms: u64,
    delay_pct: u8,
    error_status: Option<u16>,
    error_pct: u8,
    scenario: Option<&PathBuf>,
    journal_file: Option<&Path>,
    redact_headers: &[String],
    redact_json_keys: &[String],
) -> anyhow::Result<i32> {
    // Scenario mode serves a scripted sequence instead of the spec-derived
    // router; it needs no spec, no upstream, nothing but the script.
    if mode == "scenario" {
        let scenario_path = scenario
            .ok_or_else(|| anyhow::anyhow!("--scenario <file> is required for scenario mode"))?;
        let script: suspect_gateway::scenario::Scenario = serde_json::from_str(
            &std::fs::read_to_string(scenario_path)
                .map_err(|e| anyhow::anyhow!("read {}: {e}", scenario_path.display()))?,
        )
        .map_err(|e| anyhow::anyhow!("invalid scenario {}: {e}", scenario_path.display()))?;
        anyhow::ensure!(
            !script.steps.is_empty(),
            "scenario {} has no steps",
            scenario_path.display()
        );
        let rt = tokio::runtime::Runtime::new()?;
        eprintln!(
            "scenario gateway listening on 127.0.0.1:{port} ({} steps)",
            script.steps.len()
        );
        return rt
            .block_on(suspect_gateway::scenario::serve_scenario(
                port,
                script.steps,
            ))
            .map(|()| 0)
            .map_err(|e| anyhow::anyhow!("{e}"));
    }
    let gw_mode = match mode {
        "mock" => Mode::Mock,
        "proxy" => Mode::Proxy {
            upstream: require_upstream(upstream)?,
        },
        "validate" => Mode::Validate {
            upstream: require_upstream(upstream)?,
            enforce,
        },
        "record" => Mode::Record {
            upstream: require_upstream(upstream)?,
            cassette: cassette
                .cloned()
                .unwrap_or_else(|| PathBuf::from("suspect-record.scj")),
        },
        "replay" => Mode::Replay {
            cassette: match cassette {
                Some(c) => c.clone(),
                None => anyhow::bail!("--cassette <file> is required for replay mode"),
            },
        },
        other => {
            anyhow::bail!("unknown mode {other:?} (mock|proxy|validate|record|replay|scenario)")
        }
    };

    let cfg = GatewayConfig {
        mode: gw_mode,
        spec: spec.to_path_buf(),
        port,
        faults: FaultConfig {
            delay_ms,
            delay_pct,
            error_status,
            error_pct,
        },
        redact_headers: redact_headers.to_vec(),
        redact_json_keys: redact_json_keys.to_vec(),
    };

    let rt = tokio::runtime::Runtime::new()?;
    eprintln!("gateway listening on 127.0.0.1:{port} ({mode})");
    let journal = Arc::new(tokio::sync::Mutex::new(Journal::new(super::sink(
        journal_file,
    )?)));
    // Serve until Ctrl-C: the runtime blocks on serve(); process exit tears
    // everything down.
    rt.block_on(async move {
        tokio::select! {
            result = suspect_gateway::serve(cfg, journal) => match result {
                Ok(()) => Ok(0),
                Err(e) => Err(anyhow::anyhow!(e)),
            },
            _ = shutdown_signal() => {
                eprintln!("gateway shutting down");
                tokio::time::sleep(Duration::from_millis(50)).await;
                Ok(0)
            }
        }
    })
}

async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
}
