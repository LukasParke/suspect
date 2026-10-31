//! JSON or YAML input files read through serde.
//!
//! Generator inputs (`codegen --defaults`, `codegen-session --config`,
//! `--mapping`, `--target-config`, `--inputs`, the release manifest) are
//! serde documents. A `.yaml`/`.yml` file is parsed with `LowDoc` and
//! converted to JSON text, then handed to the same serde parser, so both
//! spellings accept exactly the same documents. JSON input is unchanged.

use std::fmt;
use std::path::Path;

/// Whether `path` names a YAML document (`.yaml` / `.yml`).
#[must_use]
pub fn is_yaml(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("yaml") || e.eq_ignore_ascii_case("yml"))
}

/// A parse failure with its 1-based position in the file as written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Located {
    /// 1-based line.
    pub line: usize,
    /// 1-based column.
    pub col: usize,
    /// The parser's message.
    pub message: String,
}

impl fmt::Display for Located {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for Located {}

/// Parses `bytes` with `parse`, converting YAML to JSON text first when
/// `path` names a YAML file.
///
/// For JSON the error is serde's, position included. For YAML a syntax
/// error carries its YAML position; a schema error has no position in the
/// converted text that would mean anything, so it is reported at 1:1
/// without serde's "at line N column M" suffix.
///
/// # Errors
/// Invalid UTF-8, a YAML syntax error, or `parse`'s error.
pub fn parse_with<T>(
    path: &Path,
    bytes: &[u8],
    parse: impl FnOnce(&str) -> Result<T, serde_json::Error>,
) -> Result<T, Located> {
    let at_start = |message: String| Located {
        line: 1,
        col: 1,
        message,
    };
    if !is_yaml(path) {
        let text = std::str::from_utf8(bytes).map_err(|e| at_start(format!("not UTF-8: {e}")))?;
        return parse(text).map_err(|e| Located {
            line: e.line(),
            col: e.column(),
            message: e.to_string(),
        });
    }
    let json = yaml_to_json(bytes)?;
    parse(&json).map_err(|e| at_start(without_position(&e)))
}

/// Parses `bytes` as `T` (JSON, or YAML by extension).
///
/// # Errors
/// As [`parse_with`].
pub fn parse<T: serde::de::DeserializeOwned>(path: &Path, bytes: &[u8]) -> Result<T, Located> {
    parse_with(path, bytes, |text| serde_json::from_str(text))
}

/// Reads `path` and parses it as `T`; errors name the file.
///
/// # Errors
/// IO failures and [`parse_with`] errors, prefixed with `what` and the path.
pub fn read<T: serde::de::DeserializeOwned>(path: &Path, what: &str) -> anyhow::Result<T> {
    let bytes = std::fs::read(path).map_err(|e| anyhow::anyhow!("read {}: {e}", path.display()))?;
    parse(path, &bytes).map_err(|e| anyhow::anyhow!("invalid {what} {}: {e}", path.display()))
}

fn yaml_to_json(bytes: &[u8]) -> Result<String, Located> {
    let uri = suspect_source::Uri::parse("mem://input.yaml").map_err(|e| Located {
        line: 1,
        col: 1,
        message: e.to_string(),
    })?;
    let doc = suspect_low::LowDoc::with_format(
        uri,
        suspect_source::Source::from_vec(bytes.to_vec()),
        suspect_syntax::Format::Yaml,
    );
    if let Some(error) = doc.syntax_errors().first() {
        let inner = doc.inner();
        let (line, col) = inner
            .line_index()
            .line_col(inner.bytes(), error.range.start);
        let (line, col) = (line as usize + 1, col as usize + 1);
        return Err(Located {
            line,
            col,
            message: format!("{} at line {line} column {col}", error.message),
        });
    }
    Ok(suspect_overlay::Value::from_node(doc.root()).to_json())
}

/// serde's message without the trailing " at line N column M", which would
/// point into the converted JSON text rather than the YAML file.
fn without_position(error: &serde_json::Error) -> String {
    let text = error.to_string();
    match text.rsplit_once(" at line ") {
        Some((message, _)) if error.line() > 0 => message.to_owned(),
        _ => text,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, serde::Deserialize, PartialEq)]
    #[serde(deny_unknown_fields)]
    struct Doc {
        name: String,
        #[serde(default)]
        tags: Vec<String>,
    }

    #[test]
    fn yaml_and_json_parse_to_the_same_value() {
        let json: Doc = parse(Path::new("a.json"), br#"{"name": "x", "tags": ["a"]}"#).unwrap();
        let yaml: Doc = parse(Path::new("a.yaml"), b"name: x\ntags: [a]\n").unwrap();
        let yml: Doc = parse(Path::new("a.YML"), b"name: x\ntags:\n  - a\n").unwrap();
        assert_eq!(json, yaml);
        assert_eq!(json, yml);
    }

    #[test]
    fn json_errors_keep_serde_positions() {
        let error = parse::<Doc>(Path::new("a.json"), b"{\n  \"nam\": 1}").unwrap_err();
        assert_eq!(error.line, 2);
        assert!(error.message.contains("unknown field `nam`"), "{error}");
    }

    #[test]
    fn yaml_schema_errors_drop_the_converted_position() {
        let error = parse::<Doc>(Path::new("a.yaml"), b"nam: x\n").unwrap_err();
        assert_eq!((error.line, error.col), (1, 1));
        assert!(error.message.contains("unknown field `nam`"), "{error}");
        assert!(!error.message.contains(" at line "), "{error}");
    }

    #[test]
    fn yaml_syntax_errors_are_reported_as_syntax_errors() {
        let error = parse::<Doc>(Path::new("a.yaml"), b"name: x\ntags: [a\n").unwrap_err();
        assert!(error.message.contains("syntax error"), "{error:?}");
        assert!(
            error
                .message
                .ends_with(&format!(" at line {} column {}", error.line, error.col)),
            "{error:?}"
        );
    }

    #[test]
    fn a_json_file_is_not_read_as_yaml() {
        assert!(parse::<Doc>(Path::new("a.json"), b"name: x\n").is_err());
    }
}
