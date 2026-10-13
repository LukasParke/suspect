//! Human-readable duration literals shared by think-time extensions and
//! load profiles: `250ms`, `30s`, `2m`, `1.5h`, or a bare number (seconds,
//! matching Artillery's default unit).

use std::time::Duration;

/// Parses a duration literal.
///
/// Accepted forms: `<number>[ms|s|m|h]`, with an optional space before the
/// unit, and `<number>` alone meaning seconds. Numbers may be fractional.
///
/// # Errors
/// Returns a message naming the offending literal when it is empty,
/// non-numeric, negative, or carries an unknown unit.
pub fn parse_duration(text: &str) -> Result<Duration, String> {
    let raw = text.trim();
    if raw.is_empty() {
        return Err("empty duration".to_owned());
    }
    let split = raw
        .find(|c: char| c.is_ascii_alphabetic())
        .unwrap_or(raw.len());
    let (number, unit) = raw.split_at(split);
    let number = number.trim();
    let value: f64 = number
        .parse()
        .map_err(|_| format!("invalid duration `{raw}`: `{number}` is not a number"))?;
    if !value.is_finite() || value < 0.0 {
        return Err(format!(
            "invalid duration `{raw}`: must be a finite, non-negative number"
        ));
    }
    let seconds = match unit.trim() {
        "" | "s" | "sec" | "secs" | "second" | "seconds" => value,
        "ms" | "msec" | "millisecond" | "milliseconds" => value / 1000.0,
        "m" | "min" | "mins" | "minute" | "minutes" => value * 60.0,
        "h" | "hr" | "hrs" | "hour" | "hours" => value * 3600.0,
        other => {
            return Err(format!(
                "invalid duration `{raw}`: unknown unit `{other}` (expected ms, s, m or h)"
            ));
        }
    };
    Ok(Duration::from_secs_f64(seconds))
}

/// Parses `a..b` into an inclusive duration range, or a single literal into
/// a degenerate range.
///
/// # Errors
/// Propagates [`parse_duration`] errors and rejects ranges whose lower bound
/// exceeds the upper bound.
pub fn parse_duration_range(text: &str) -> Result<(Duration, Duration), String> {
    match text.split_once("..") {
        Some((lo, hi)) => {
            let lo = parse_duration(lo)?;
            let hi = parse_duration(hi)?;
            if lo > hi {
                return Err(format!(
                    "invalid duration range `{text}`: lower bound exceeds upper bound"
                ));
            }
            Ok((lo, hi))
        }
        None => {
            let d = parse_duration(text)?;
            Ok((d, d))
        }
    }
}

/// Renders a duration compactly for reports (`250ms`, `1.5s`, `2m30s`).
#[must_use]
pub fn format_duration(d: Duration) -> String {
    let ms = d.as_millis();
    if ms < 1000 {
        return format!("{ms}ms");
    }
    let secs = d.as_secs_f64();
    if secs < 60.0 {
        return format!("{secs:.1}s");
    }
    let total = d.as_secs();
    let (h, m, s) = (total / 3600, (total % 3600) / 60, total % 60);
    if h > 0 {
        format!("{h}h{m:02}m{s:02}s")
    } else {
        format!("{m}m{s:02}s")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_units_and_bare_seconds() {
        assert_eq!(parse_duration("250ms").unwrap(), Duration::from_millis(250));
        assert_eq!(parse_duration("30s").unwrap(), Duration::from_secs(30));
        assert_eq!(parse_duration("2m").unwrap(), Duration::from_secs(120));
        assert_eq!(parse_duration("1.5h").unwrap(), Duration::from_secs(5400));
        assert_eq!(parse_duration("45").unwrap(), Duration::from_secs(45));
        assert_eq!(
            parse_duration(" 3 minutes ").unwrap(),
            Duration::from_secs(180)
        );
    }

    #[test]
    fn rejects_garbage() {
        assert!(parse_duration("").is_err());
        assert!(parse_duration("fast").is_err());
        assert!(parse_duration("-1s").is_err());
        assert!(parse_duration("10 parsecs").is_err());
    }

    #[test]
    fn ranges() {
        assert_eq!(
            parse_duration_range("500ms..2s").unwrap(),
            (Duration::from_millis(500), Duration::from_secs(2))
        );
        assert_eq!(
            parse_duration_range("1s").unwrap(),
            (Duration::from_secs(1), Duration::from_secs(1))
        );
        assert!(parse_duration_range("2s..1s").is_err());
    }

    #[test]
    fn formats() {
        assert_eq!(format_duration(Duration::from_millis(250)), "250ms");
        assert_eq!(format_duration(Duration::from_millis(1500)), "1.5s");
        assert_eq!(format_duration(Duration::from_secs(150)), "2m30s");
        assert_eq!(format_duration(Duration::from_secs(3725)), "1h02m05s");
    }
}
