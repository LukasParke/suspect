//! Row-per-user data feeding and input pools.
//!
//! A [`DataFeeder`] loads a CSV or NDJSON file once and hands each new
//! virtual user one row as workflow inputs, in `random`, `sequence` or
//! `unique` order. CSV cells that parse as JSON numbers or booleans are
//! typed; everything else stays a string.

use std::sync::atomic::{AtomicUsize, Ordering};

use rand::Rng;

use super::profile::{DataOrder, DataSource};
use crate::exec::InputMap;

/// A loaded data file.
pub struct DataFeeder {
    rows: Vec<InputMap>,
    order: DataOrder,
    cursor: AtomicUsize,
}

/// `order: unique` ran out of rows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataExhausted {
    /// Rows the file had.
    pub rows: usize,
}

impl std::fmt::Display for DataExhausted {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "data file exhausted after {} unique rows", self.rows)
    }
}

impl std::error::Error for DataExhausted {}

impl DataFeeder {
    /// Loads the file described by `src`.
    ///
    /// # Errors
    /// Returns a message when the file cannot be read or parsed, or has no
    /// rows.
    pub fn load(src: &DataSource) -> Result<Self, String> {
        let text = std::fs::read_to_string(&src.path)
            .map_err(|e| format!("cannot read data file {}: {e}", src.path.display()))?;
        let is_ndjson = src
            .path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| matches!(e, "ndjson" | "jsonl" | "json"));
        let rows = if is_ndjson {
            parse_ndjson(&text)?
        } else {
            parse_csv(&text, &src.fields, src.skip_header, src.delimiter)?
        };
        if rows.is_empty() {
            return Err(format!("data file {} has no rows", src.path.display()));
        }
        Ok(Self {
            rows,
            order: src.order,
            cursor: AtomicUsize::new(0),
        })
    }

    /// Builds a feeder from in-memory rows (tests, generated data).
    #[must_use]
    pub fn from_rows(rows: Vec<InputMap>, order: DataOrder) -> Self {
        Self {
            rows,
            order,
            cursor: AtomicUsize::new(0),
        }
    }

    /// Number of rows.
    #[must_use]
    pub fn len(&self) -> usize {
        self.rows.len()
    }

    /// Whether the feeder has no rows.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// Hands out the next row for a new user.
    ///
    /// # Errors
    /// Returns [`DataExhausted`] for `unique` order once every row is used.
    pub fn next_row(&self, rng: &mut impl Rng) -> Result<InputMap, DataExhausted> {
        let n = self.rows.len();
        let idx = match self.order {
            DataOrder::Random => rng.gen_range(0..n),
            DataOrder::Sequence => self.cursor.fetch_add(1, Ordering::Relaxed) % n,
            DataOrder::Unique => {
                let i = self.cursor.fetch_add(1, Ordering::Relaxed);
                if i >= n {
                    return Err(DataExhausted { rows: n });
                }
                i
            }
        };
        Ok(self.rows[idx].clone())
    }
}

fn parse_ndjson(text: &str) -> Result<Vec<InputMap>, String> {
    let mut rows = Vec::new();
    for (lineno, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        match serde_json::from_str::<serde_json::Value>(line) {
            Ok(serde_json::Value::Object(map)) => rows.push(map),
            Ok(_) => {
                return Err(format!("line {}: NDJSON rows must be objects", lineno + 1));
            }
            Err(e) => return Err(format!("line {}: {e}", lineno + 1)),
        }
    }
    Ok(rows)
}

fn parse_csv(
    text: &str,
    fields: &[String],
    skip_header: bool,
    delimiter: u8,
) -> Result<Vec<InputMap>, String> {
    let mut reader = csv::ReaderBuilder::new()
        .has_headers(skip_header)
        .delimiter(delimiter)
        .flexible(true)
        .trim(csv::Trim::All)
        .from_reader(text.as_bytes());
    let mut rows = Vec::new();
    for (i, record) in reader.records().enumerate() {
        let record = record.map_err(|e| format!("row {}: {e}", i + 1))?;
        if record.iter().all(str::is_empty) {
            continue;
        }
        let mut row = InputMap::new();
        for (name, cell) in fields.iter().zip(record.iter()) {
            row.insert(name.clone(), cast(cell));
        }
        rows.push(row);
    }
    Ok(rows)
}

/// Numbers and booleans become typed JSON; anything else stays text.
fn cast(cell: &str) -> serde_json::Value {
    match serde_json::from_str::<serde_json::Value>(cell) {
        Ok(v @ (serde_json::Value::Number(_) | serde_json::Value::Bool(_))) => v,
        _ => serde_json::Value::String(cell.to_owned()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;

    fn feeder(order: DataOrder) -> DataFeeder {
        let rows = parse_csv(
            "user,pass,age\nalice,s3cret,30\nbob,hunter2,41\n",
            &["user".into(), "pass".into(), "age".into()],
            true,
            b',',
        )
        .unwrap();
        DataFeeder::from_rows(rows, order)
    }

    #[test]
    fn csv_cells_are_typed_and_header_skipped() {
        let f = feeder(DataOrder::Sequence);
        assert_eq!(f.len(), 2);
        let mut rng = rand::rngs::StdRng::seed_from_u64(1);
        let row = f.next_row(&mut rng).unwrap();
        assert_eq!(row["user"], serde_json::json!("alice"));
        assert_eq!(row["age"], serde_json::json!(30));
        assert_eq!(
            cast("007"),
            serde_json::json!("007"),
            "leading zero stays text"
        );
        assert_eq!(cast("true"), serde_json::json!(true));
    }

    #[test]
    fn sequence_wraps_and_unique_exhausts() {
        let mut rng = rand::rngs::StdRng::seed_from_u64(1);
        let seq = feeder(DataOrder::Sequence);
        let users: Vec<_> = (0..3)
            .map(|_| seq.next_row(&mut rng).unwrap()["user"].clone())
            .collect();
        assert_eq!(users, vec!["alice", "bob", "alice"]);

        let uniq = feeder(DataOrder::Unique);
        assert!(uniq.next_row(&mut rng).is_ok());
        assert!(uniq.next_row(&mut rng).is_ok());
        assert_eq!(
            uniq.next_row(&mut rng).unwrap_err(),
            DataExhausted { rows: 2 }
        );
    }

    #[test]
    fn ndjson_rows_and_errors() {
        let rows = parse_ndjson("{\"a\":1}\n\n{\"a\":2,\"b\":\"x\"}\n").unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[1]["b"], serde_json::json!("x"));
        assert!(parse_ndjson("[1,2]").unwrap_err().contains("objects"));
        assert!(parse_ndjson("{bad").unwrap_err().contains("line 1"));
    }
}
