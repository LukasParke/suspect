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
) -> anyhow::Result<i32> {
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
        other => anyhow::bail!("unknown mode {other:?} (mock|proxy|validate|record|replay)"),
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
    };

    let rt = tokio::runtime::Runtime::new()?;
    eprintln!("gateway listening on 127.0.0.1:{port} ({mode})");
    let journal = Arc::new(tokio::sync::Mutex::new(Journal::new(Box::new(
        suspect_journal::StdoutSink,
    ))));
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
