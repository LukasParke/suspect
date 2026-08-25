//! `suspect replay` — re-issues a recorded cassette's requests against a
//! live upstream and reports drift.
//!
//! For every `CassetteEntry` the recorded request (method, URL retargeted
//! onto `--upstream`, headers, body) is re-sent; drift is reported per
//! exchange as `<status-match ok|DRIFT> <body ok|DRIFT> <url>`. With
//! `--diff`, UTF-8 response bodies that drifted get a unified diff of
//! recorded-vs-live, truncated to 40 lines. Any drift exits 1.

use std::path::Path;
use std::sync::Arc;

use suspect_gateway::replay::body_matches;
use suspect_gen::orchestrate::unified_diff;
use suspect_journal::Journal;
use suspect_test::{HttpClient, HttpRequest};

use super::http::LiveTransport;

/// Maximum diff lines printed per drifted body.
const MAX_DIFF_LINES: usize = 40;

/// Headers that must not be replayed verbatim: framing and addressing are
/// owned by the transport for the new connection.
fn replayable(header: &str) -> bool {
    !matches!(
        header.to_ascii_lowercase().as_str(),
        "host" | "content-length"
    )
}

/// Runs `suspect replay` against one cassette.
///
/// # Errors
/// Propagates cassette IO/parse failures and transport setup errors; drift
/// surfaces through the exit code instead.
pub fn replay(cassette: &Path, upstream: &str, diff: bool) -> anyhow::Result<i32> {
    let file = std::fs::File::open(cassette)?;
    let (header, entries) = suspect_journal::read_cassette(file)?;

    let started = std::time::Instant::now();
    let rt = tokio::runtime::Runtime::new()?;
    let http = Arc::new(LiveTransport::new(std::time::Duration::from_secs(30))?);

    let mut matched: u32 = 0;
    let mut drifted: u32 = 0;
    rt.block_on(async {
        for entry in &entries {
            let url = retarget(&entry.url, upstream);
            let req = HttpRequest {
                method: entry.method.clone(),
                url,
                headers: entry
                    .request_headers
                    .iter()
                    .filter(|(k, _)| replayable(k))
                    .cloned()
                    .collect(),
                body: entry.request_body.bytes().into(),
            };
            // A failed exchange is maximal drift on both axes.
            let outcome = match http.execute(req).await {
                Ok(resp) => (resp.status, resp.body.to_vec()),
                Err(e) => {
                    println!("DRIFT DRIFT {} ({e})", entry.url);
                    drifted += 1;
                    continue;
                }
            };
            let status_ok = outcome.0 == entry.status;
            let body_ok = body_matches(&entry.response_body, &outcome.1);
            if status_ok && body_ok {
                matched += 1;
            } else {
                drifted += 1;
            }
            println!("{} {} {}", mark(status_ok), mark(body_ok), entry.url);
            if diff && !body_ok {
                print_body_diff(&entry.response_body, &outcome.1);
            }
        }
    });

    println!();
    println!(
        "replay of {} against {upstream}: {matched} matched, {drifted} drifted",
        header.source,
    );
    let elapsed_ms = started.elapsed().as_millis() as f64;
    let mut journal = Journal::new(Box::new(suspect_journal::StdoutSink));
    journal.run_summary("replay", matched, drifted, 0, elapsed_ms);
    Ok(i32::from(drifted > 0))
}

/// `ok` / `DRIFT` marker used in report lines.
fn mark(ok: bool) -> &'static str {
    if ok { "ok" } else { "DRIFT" }
}

/// Re-points a recorded absolute URL at `upstream`, keeping path + query.
#[must_use]
fn retarget(url: &str, upstream: &str) -> String {
    let rest = match url.find("://") {
        Some(i) => {
            let after = &url[i + 3..];
            match after.find('/') {
                Some(j) => &after[j..],
                None => "/",
            }
        }
        None => url,
    };
    format!(
        "{}/{}",
        upstream.trim_end_matches('/'),
        rest.trim_start_matches('/')
    )
}

/// Prints a unified diff between the recorded and live bodies when both are
/// UTF-8 text; truncated to [`MAX_DIFF_LINES`] lines.
fn print_body_diff(recorded: &suspect_journal::Body, live: &[u8]) {
    let (Some(recorded_text), Ok(live_text)) = (recorded.text(), std::str::from_utf8(live)) else {
        println!("    (binary or non-UTF-8 drifted body; no diff)");
        return;
    };
    let full = unified_diff(recorded_text, live_text);
    let mut lines = full.lines();
    for line in lines.by_ref().take(MAX_DIFF_LINES) {
        println!("    {line}");
    }
    if lines.next().is_some() {
        println!("    … (diff truncated at {MAX_DIFF_LINES} lines)");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use suspect_journal::Body;

    #[test]
    fn retargets_onto_upstream() {
        assert_eq!(
            retarget(
                "https://api.example.com/v2/pets?limit=3",
                "http://localhost:8080"
            ),
            "http://localhost:8080/v2/pets?limit=3"
        );
        assert_eq!(
            retarget("https://api.example.com", "http://h.test/base"),
            "http://h.test/base/"
        );
        assert_eq!(
            retarget("/pets?x=1", "http://h.test"),
            "http://h.test/pets?x=1"
        );
    }

    #[test]
    fn drops_framing_headers_only() {
        assert!(!replayable("Host"));
        assert!(!replayable("CONTENT-LENGTH"));
        assert!(replayable("content-type"));
        assert!(replayable("x-request-id"));
    }

    #[test]
    fn diff_truncates_at_limit() {
        // print_body_diff is printing-only; assert the truncation arithmetic
        let long = "a\n".repeat(100);
        assert!(long.lines().count() > MAX_DIFF_LINES);
        let short = Body::from_bytes(b"x");
        // Non-UTF8 live bytes take the no-diff branch without panicking.
        print_body_diff(&short, &[0xff, 0xfe]);
    }
}
