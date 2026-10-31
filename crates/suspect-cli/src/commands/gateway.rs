//! `suspect traffic gateway` — run the local mock/proxy/validate/record/replay
//! server against a spec.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use suspect_gateway::{FaultConfig, GatewayConfig, Mode};
use suspect_journal::Journal;

/// `--mode` values; `gateway.mode` in config uses the same names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
#[value(rename_all = "lower")]
pub enum GatewayMode {
    /// Serve spec-derived mock responses.
    Mock,
    /// Forward to `--upstream`.
    Proxy,
    /// Forward to `--upstream`, validating requests and responses.
    Validate,
    /// Forward to `--upstream` and record a cassette.
    Record,
    /// Serve a recorded cassette.
    Replay,
    /// Serve the scripted steps of `--scenario`.
    Scenario,
}

impl GatewayMode {
    /// The mode's name, as config files spell it.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Mock => "mock",
            Self::Proxy => "proxy",
            Self::Validate => "validate",
            Self::Record => "record",
            Self::Replay => "replay",
            Self::Scenario => "scenario",
        }
    }
}

/// Resolves the upstream URL or fails with a mode-specific message.
fn require_upstream(upstream: Option<&str>) -> anyhow::Result<String> {
    upstream
        .map(str::to_owned)
        .ok_or_else(|| anyhow::anyhow!("--upstream http://host[:port] is required for this mode"))
}

/// Runs `suspect traffic gateway` until interrupted.
///
/// # Errors
/// Propagates bind/spec-loading failures from the gateway.
#[allow(clippy::too_many_arguments)]
pub fn gateway(
    spec: &Path,
    port: u16,
    mode: &str,
    upstream: Option<&str>,
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
        let steps = script.steps;
        return rt.block_on(async move {
            let listener = tokio::net::TcpListener::bind(("127.0.0.1", port))
                .await
                .map_err(|e| anyhow::anyhow!("cannot bind 127.0.0.1:{port}: {e}"))?;
            // Announce only once bound, with the real address (`--port 0`
            // resolves here): the line is the readiness signal.
            eprintln!(
                "scenario gateway listening on {} ({} steps)",
                listener.local_addr()?,
                steps.len()
            );
            tokio::select! {
                result = suspect_gateway::scenario::serve_scenario_on(listener, steps) => {
                    result.map(|()| 0).map_err(|e| anyhow::anyhow!("{e}"))
                }
                () = shutdown_signal() => Ok(0),
            }
        });
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
    let journal = Arc::new(tokio::sync::Mutex::new(Journal::new(super::sink(
        journal_file,
    )?)));
    // Serve until the first Ctrl-C, then return 0.
    rt.block_on(async move {
        let bound = suspect_gateway::bind(cfg, journal)
            .await
            .map_err(|e| anyhow::anyhow!(e))?;
        // Announce only once the router is built and the port is bound,
        // with the real address (`--port 0` resolves here): the line is
        // the readiness signal.
        eprintln!("gateway listening on {} ({mode})", bound.local_addr()?);
        tokio::select! {
            result = bound.serve() => match result {
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

/// Resolves on the first Ctrl-C. Ctrl-C is how the gateway stops, so the
/// command then returns 0 (`crate::interrupt`).
async fn shutdown_signal() {
    // The sender is a static, so `wait_for` only returns on Ctrl-C.
    let _ = crate::interrupt::graceful()
        .wait_for(|interrupted| *interrupted)
        .await;
}
