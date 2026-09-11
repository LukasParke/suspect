//! Explicit pinned acquisition and cache-only verification.

use std::io::{self, Write};
use std::path::Path;

use serde_json::{Value, json};
use suspect_ref::acquire::{
    AcquireError, AcquireErrorKind, AcquireOptions, AcquiredClosure, RefreshPolicy, acquire,
    redact_uri,
};

use crate::OutputFormat;

/// Acquires or verifies one immutable manifest and prints a versioned report.
/// Shared CLI argument parsing/registration supplies the complete options value.
/// Ctrl-C cancels the active transfer, waits for its child to be reaped, and exits
/// 130. Other acquisition findings exit 1; a complete verified closure exits 0.
///
/// # Errors
/// Propagates runtime/signal setup, worker failures, and report output errors.
pub fn run(
    manifest_path: &Path,
    options: AcquireOptions,
    format: OutputFormat,
) -> anyhow::Result<i32> {
    let offline = options.offline;
    let refresh = options.refresh;
    let cancellation = options.cancellation.clone();
    let path = manifest_path.to_path_buf();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let result = runtime.block_on(async move {
        let mut work = tokio::task::spawn_blocking(move || acquire(&path, options));
        tokio::select! {
            result = &mut work => Ok::<_, anyhow::Error>(result?),
            signal = tokio::signal::ctrl_c() => {
                cancellation.cancel();
                let result = work.await?;
                signal?;
                Ok(result)
            }
        }
    })?;
    let (report, exit) = match result {
        Ok(closure) => (success(&closure, manifest_path, offline, refresh), 0),
        Err(error) => {
            let exit = if matches!(error.kind(), AcquireErrorKind::Cancelled) {
                130
            } else {
                1
            };
            (failure(&error, offline), exit)
        }
    };
    let mut output = io::stdout().lock();
    match format {
        OutputFormat::Json => {
            serde_json::to_writer_pretty(&mut output, &report)?;
            writeln!(output)?;
        }
        OutputFormat::Text => {
            writeln!(
                output,
                "Pinned closure {}: {} documents, {} network requests",
                report["status"].as_str().unwrap_or("failed"),
                report["documents"].as_array().map_or(0, Vec::len),
                report["networkRequests"].as_u64().unwrap_or(0)
            )?;
            if let Some(fingerprint) = report["fingerprint"].as_str() {
                writeln!(
                    output,
                    "Manifest: {} ({fingerprint})",
                    manifest_path.display()
                )?;
            }
            for diagnostic in report["diagnostics"].as_array().into_iter().flatten() {
                writeln!(
                    output,
                    "{}",
                    diagnostic["message"]
                        .as_str()
                        .unwrap_or("acquisition failed")
                )?;
            }
        }
    }
    output.flush()?;
    Ok(exit)
}

fn success(closure: &AcquiredClosure, path: &Path, offline: bool, refresh: RefreshPolicy) -> Value {
    let all_cached = closure.records().iter().all(|record| record.from_cache());
    let status = if all_cached {
        "verified"
    } else if refresh != RefreshPolicy::Never {
        "refreshed"
    } else {
        "acquired"
    };
    json!({
        "format": "suspect.acquire.v1",
        "status": status,
        "offline": offline,
        "manifest": path,
        "fingerprint": closure.fingerprint(),
        "requestedEntry": redact_uri(closure.requested_entry()),
        "effectiveEntry": redact_uri(closure.entry()),
        "cacheManifest": closure.cache_manifest_path(),
        "networkRequests": closure.records().iter().map(|record| record.attempts()).sum::<usize>(),
        "documents": closure.documents().iter().map(|document| json!({
            "requestedUri": redact_uri(document.requested_uri()),
            "effectiveUri": redact_uri(document.effective_uri()),
            "digest": document.digest(),
            "bytes": document.byte_len(),
            "mediaType": document.media_type(),
            "cachePath": document.cache_path(),
            "fingerprint": document.fingerprint()
        })).collect::<Vec<_>>(),
        "retrievals": closure.records().iter().map(|record| json!({
            "requestedUri": redact_uri(record.requested_uri()),
            "fromCache": record.from_cache(),
            "attempts": record.attempts(),
            "redirects": record.redirects().iter().map(|hop| json!({
                "fromUri": redact_uri(hop.from_uri()),
                "toUri": redact_uri(hop.to_uri()),
                "status": hop.status()
            })).collect::<Vec<_>>()
        })).collect::<Vec<_>>(),
        "diagnostics": []
    })
}

fn failure(error: &AcquireError, offline: bool) -> Value {
    json!({
        "format": "suspect.acquire.v1",
        "status": "failed",
        "offline": offline,
        "manifest": error.manifest_path(),
        "documents": [],
        "diagnostics": [{
            "code": error.code(),
            "message": error.to_string(),
            "uri": error.redacted_uri(),
            "resourceIndex": error.resource_index(),
            "line": error.line(),
            "file": error.file_path(),
            "redirects": error.redirects().iter().map(|hop| json!({
                "fromUri": redact_uri(hop.from_uri()),
                "toUri": redact_uri(hop.to_uri()),
                "status": hop.status()
            })).collect::<Vec<_>>()
        }]
    })
}
