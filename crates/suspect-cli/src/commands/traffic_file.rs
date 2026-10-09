//! Reading recorded traffic files: Suspect Cassettes (gateway `--mode
//! record`, with bodies) and Suspect Journals (gateway `--journal`,
//! `suspect test`), recognized line by line in either shape.

use std::path::Path;

use suspect_journal::BodyEncoding;

/// One recorded exchange, in the richest shape either format carries.
#[derive(Debug, Clone)]
pub struct TrafficLine {
    /// Uppercase HTTP method.
    pub method: String,
    /// Full request URL.
    pub url: String,
    /// Response status; `0` when the exchange failed before one.
    pub status: u16,
    /// Request headers, already redacted at record time.
    pub request_headers: Vec<(String, String)>,
    /// Response headers, already redacted at record time.
    pub response_headers: Vec<(String, String)>,
    /// Request body when recorded, with its encoding.
    pub request_body: Option<(BodyEncoding, String)>,
    /// Response body when recorded, with its encoding.
    pub response_body: Option<(BodyEncoding, String)>,
    /// Exchange duration in milliseconds.
    pub duration_ms: f64,
    /// Completion time, Unix epoch milliseconds (journals only; cassettes
    /// record order, not clock time).
    pub ts_ms: u64,
}

/// Reads every exchange in a traffic file. Cassette headers, journal meta
/// and log lines, and anything unrecognizable are skipped.
///
/// # Errors
/// IO failures reading the file.
pub fn read_traffic_lines(path: &Path) -> anyhow::Result<Vec<TrafficLine>> {
    let text = std::fs::read_to_string(path)?;
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        if let Some(entry) = traffic_line(&value) {
            out.push(entry);
        }
    }
    Ok(out)
}

/// Parses one line into a [`TrafficLine`]; `None` for non-exchange
/// records.
fn traffic_line(value: &serde_json::Value) -> Option<TrafficLine> {
    let is_journal = value.get("kind").and_then(serde_json::Value::as_str) == Some("traffic");
    let is_cassette =
        value.get("kind").is_none() && value.get("method").is_some() && value.get("url").is_some();
    if !is_journal && !is_cassette {
        return None;
    }
    let body = |key: &str| -> Option<(BodyEncoding, String)> {
        let body = value.get(key)?;
        let encoding = body
            .get("encoding")
            .and_then(serde_json::Value::as_str)
            .map_or(BodyEncoding::Utf8, |enc| {
                if enc == "base64" {
                    BodyEncoding::Base64
                } else {
                    BodyEncoding::Utf8
                }
            });
        let content = body
            .get("content")
            .and_then(serde_json::Value::as_str)?
            .to_owned();
        Some((encoding, content))
    };
    let headers = |key: &str| -> Vec<(String, String)> {
        value
            .get(key)
            .and_then(serde_json::Value::as_array)
            .map(|pairs| {
                pairs
                    .iter()
                    .filter_map(|pair| {
                        let name = pair.get(0).or_else(|| pair.get("name"))?;
                        let val = pair.get(1).or_else(|| pair.get("value"))?;
                        Some((
                            name.as_str()?.to_owned(),
                            val.as_str().unwrap_or_default().to_owned(),
                        ))
                    })
                    .collect()
            })
            .unwrap_or_default()
    };
    Some(TrafficLine {
        method: value.get("method")?.as_str()?.to_ascii_uppercase(),
        url: value.get("url")?.as_str()?.to_owned(),
        status: value
            .get("status")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0) as u16,
        request_headers: headers("request_headers"),
        response_headers: headers("response_headers"),
        request_body: if is_journal {
            None
        } else {
            body("request_body")
        },
        response_body: if is_journal {
            None
        } else {
            body("response_body")
        },
        duration_ms: value
            .get("duration_ms")
            .and_then(serde_json::Value::as_f64)
            .unwrap_or(0.0),
        ts_ms: value
            .get("ts_ms")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0),
    })
}

/// Unix epoch milliseconds as an RFC 3339 UTC timestamp.
#[must_use]
pub fn iso8601(ts_ms: u64) -> String {
    let secs = (ts_ms / 1000) as i64;
    let millis = ts_ms % 1000;
    let days = secs.div_euclid(86_400);
    let seconds_of_day = secs.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    let hour = seconds_of_day / 3_600;
    let minute = (seconds_of_day % 3_600) / 60;
    let second = seconds_of_day % 60;
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}.{millis:03}Z")
}

/// Days since 1970-01-01 to a civil (year, month, day), via the
/// well-known shift-and-wrap arithmetic.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}
