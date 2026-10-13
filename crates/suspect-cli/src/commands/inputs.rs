//! Workflow input binding shared by `suspect test` and `suspect load`:
//! `--input key=value` pairs and `--inputs file.json` objects become the
//! caller-supplied [`InputMap`] that overrides Arazzo `parameters` values
//! and schema defaults.

use std::path::Path;

use suspect_test::InputMap;

/// Builds an input map from a JSON file (an object) and `key=value` pairs.
/// Pairs apply after the file, so they override it. Values that parse as
/// JSON (`42`, `true`, `{"a":1}`) are kept typed; anything else is a string.
///
/// # Errors
/// Returns an error when the file is unreadable or not a JSON object, or a
/// pair lacks `=` or a name.
pub fn parse_inputs(file: Option<&Path>, pairs: &[String]) -> anyhow::Result<InputMap> {
    let mut inputs = InputMap::new();
    if let Some(path) = file {
        let text = std::fs::read_to_string(path)
            .map_err(|e| anyhow::anyhow!("cannot read inputs file {}: {e}", path.display()))?;
        let value: serde_json::Value = serde_json::from_str(&text)
            .map_err(|e| anyhow::anyhow!("inputs file {} is not JSON: {e}", path.display()))?;
        match value {
            serde_json::Value::Object(map) => inputs.extend(map),
            _ => anyhow::bail!("inputs file {} must contain a JSON object", path.display()),
        }
    }
    for pair in pairs {
        let Some((name, raw)) = pair.split_once('=') else {
            anyhow::bail!("--input expects NAME=VALUE, got {pair:?}");
        };
        if name.is_empty() {
            anyhow::bail!("--input expects NAME=VALUE, got {pair:?}");
        }
        inputs.insert(name.to_owned(), coerce(raw));
    }
    Ok(inputs)
}

/// JSON when it parses, otherwise a plain string.
#[must_use]
pub fn coerce(raw: &str) -> serde_json::Value {
    serde_json::from_str(raw).unwrap_or_else(|_| serde_json::Value::String(raw.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pairs_override_file_and_coerce_json() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("in.json");
        std::fs::write(&file, r#"{"a": 1, "b": "x"}"#).unwrap();
        let map = parse_inputs(
            Some(&file),
            &[
                "b=2".to_owned(),
                "c=true".to_owned(),
                "d=hello world".to_owned(),
            ],
        )
        .unwrap();
        assert_eq!(map["a"], serde_json::json!(1));
        assert_eq!(map["b"], serde_json::json!(2));
        assert_eq!(map["c"], serde_json::json!(true));
        assert_eq!(map["d"], serde_json::json!("hello world"));
    }

    #[test]
    fn rejects_bad_shapes() {
        assert!(parse_inputs(None, &["novalue".to_owned()]).is_err());
        assert!(parse_inputs(None, &["=1".to_owned()]).is_err());
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("arr.json");
        std::fs::write(&file, "[1,2]").unwrap();
        assert!(parse_inputs(Some(&file), &[]).is_err());
    }
}
