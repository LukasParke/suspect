//! Text, JSON, SARIF 2.1.0 and JUnit XML renderers over [`Finding`]s.
//!
//! Renderers never read files: a [`Sources`] maps documents to display
//! paths and byte offsets to positions, and a [`RuleLookup`] supplies rule
//! descriptions for SARIF.

use serde_json::{Value, json};
use suspect_source::Uri;

use crate::{Finding, RuleInfo, Severity};

/// Document lookups the renderers need.
pub trait Sources {
    /// 0-based line and column of `offset` in `doc`, when the document is
    /// known.
    fn line_col(&self, doc: &Uri, offset: usize) -> Option<(u32, u32)>;
    /// The path to show for `doc` (as given on the command line, say).
    fn display_path(&self, doc: &Uri) -> String;
}

/// Rule metadata lookup for SARIF `rules` entries.
pub trait RuleLookup {
    /// Metadata for `code`, when registered.
    fn rule(&self, code: &str) -> Option<RuleInfo>;
}

/// A [`RuleLookup`] that knows no rules: SARIF rule descriptions fall back
/// to the code itself.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoRules;

impl RuleLookup for NoRules {
    fn rule(&self, _code: &str) -> Option<RuleInfo> {
        None
    }
}

/// 1-based start line and column: the finding's own position, else its
/// range start resolved through `src`, else `1:1`.
#[must_use]
pub fn start(finding: &Finding, src: &dyn Sources) -> (u32, u32) {
    if let Some(position) = finding.position {
        return (position.line + 1, position.col + 1);
    }
    finding
        .range
        .as_ref()
        .and_then(|range| src.line_col(&finding.document, range.start))
        .map_or((1, 1), |(line, col)| (line + 1, col + 1))
}

/// One `file:line:col [SEVERITY] code: message` line per finding.
///
/// # Errors
/// Propagates write failures.
pub fn text(
    findings: &[Finding],
    src: &dyn Sources,
    out: &mut dyn std::io::Write,
) -> std::io::Result<()> {
    for f in findings {
        let (line, col) = start(f, src);
        writeln!(
            out,
            "{}:{}:{} [{}] {}: {}",
            src.display_path(&f.document),
            line,
            col,
            f.severity.upper(),
            f.code,
            f.message
        )?;
    }
    Ok(())
}

/// A JSON array with one object per finding: `code`, `severity`, `message`,
/// `check`, `file`, 1-based `line`/`col`, and `range`/`pointer` when known.
#[must_use]
pub fn json(findings: &[Finding], src: &dyn Sources) -> Value {
    findings
        .iter()
        .map(|f| {
            let (line, col) = start(f, src);
            let mut entry = json!({
                "code": f.code,
                "severity": f.severity,
                "message": f.message,
                "check": f.check,
                "file": src.display_path(&f.document),
                "line": line,
                "col": col,
            });
            if let Some(range) = &f.range {
                entry["range"] = json!({"start": range.start, "end": range.end});
            }
            if let Some(pointer) = &f.pointer {
                entry["pointer"] = json!(pointer);
            }
            entry
        })
        .collect()
}

/// The JSON findings report: `findings` as [`json`] renders them, and the
/// findings inline or extension suppressions hid under `suppressed`, so a
/// reviewer can audit them.
#[must_use]
pub fn json_report(findings: &[Finding], suppressed: &[Finding], src: &dyn Sources) -> Value {
    json!({
        "findings": json(findings, src),
        "suppressed": json(suppressed, src),
    })
}

/// Maps a severity to a SARIF level (`note`, `warning`, `error`).
#[must_use]
pub fn sarif_level(severity: Severity) -> &'static str {
    match severity {
        Severity::Error => "error",
        Severity::Warning => "warning",
        Severity::Info | Severity::Hint => "note",
    }
}

/// A SARIF 2.1.0 log. Rules are collected from the findings in first-seen
/// order; each result references its rule by index and is located at
/// `file:startLine:startColumn`. A rule found in `rules` gets its summary
/// as `shortDescription` and its fix text as `help`; otherwise the
/// description is the code.
#[must_use]
pub fn sarif(findings: &[Finding], src: &dyn Sources, rules: &dyn RuleLookup) -> Value {
    let mut rule_ids: Vec<&str> = Vec::new();
    let mut rule_entries: Vec<Value> = Vec::new();
    for f in findings {
        if !rule_ids.contains(&&*f.code) {
            rule_ids.push(&f.code);
            let info = rules.rule(&f.code);
            let description = info
                .map(|i| i.summary)
                .filter(|s| !s.is_empty())
                .unwrap_or(&f.code);
            let mut entry = json!({
                "id": f.code,
                "shortDescription": {"text": description},
            });
            if let Some(help) = info.map(|i| i.how_to_fix).filter(|s| !s.is_empty()) {
                entry["help"] = json!({"text": help});
            }
            rule_entries.push(entry);
        }
    }
    let results: Vec<Value> = findings
        .iter()
        .map(|f| {
            let (line, col) = start(f, src);
            json!({
                "ruleId": f.code,
                "ruleIndex": rule_ids.iter().position(|r| *r == f.code).unwrap_or(0),
                "level": sarif_level(f.severity),
                "message": {"text": f.message},
                "locations": [{
                    "physicalLocation": {
                        "artifactLocation": {"uri": src.display_path(&f.document)},
                        "region": {
                            "startLine": line,
                            "startColumn": col,
                        },
                    },
                }],
            })
        })
        .collect();
    json!({
        "$schema": "https://json.schemastore.org/sarif-2.1.0.json",
        "version": "2.1.0",
        "runs": [{
            "tool": {
                "driver": {
                    "name": "suspect",
                    "informationUri": "https://github.com/LukasParke/suspect",
                    "rules": rule_entries,
                },
            },
            "results": results,
        }],
    })
}

/// A JUnit XML report: one `testsuite` per document (first-seen order),
/// one `testcase` per finding, named by its code, with `file:line:col` as
/// its `classname`. Error and warning findings carry a `failure` whose
/// `type` is the severity; info and hint findings are passing cases with
/// the message in `system-out`. No findings give an empty `testsuites`.
#[must_use]
pub fn junit(findings: &[Finding], src: &dyn Sources) -> String {
    use std::fmt::Write as _;

    let fails = |f: &Finding| matches!(f.severity, Severity::Error | Severity::Warning);
    let mut suites: Vec<(String, Vec<&Finding>)> = Vec::new();
    for f in findings {
        let path = src.display_path(&f.document);
        match suites.iter_mut().find(|(p, _)| *p == path) {
            Some((_, list)) => list.push(f),
            None => suites.push((path, vec![f])),
        }
    }
    let failures = findings.iter().filter(|f| fails(f)).count();
    let mut out = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    // Writing into a String cannot fail.
    let _ = writeln!(
        out,
        "<testsuites name=\"suspect\" tests=\"{}\" failures=\"{failures}\" errors=\"0\">",
        findings.len()
    );
    for (path, list) in &suites {
        let path = xml_escape(path);
        let suite_failures = list.iter().filter(|f| fails(f)).count();
        let _ = writeln!(
            out,
            "  <testsuite name=\"{path}\" tests=\"{}\" failures=\"{suite_failures}\" errors=\"0\">",
            list.len()
        );
        for f in list {
            let (line, col) = start(f, src);
            let _ = writeln!(
                out,
                "    <testcase name=\"{}\" classname=\"{path}:{line}:{col}\">",
                xml_escape(&f.code)
            );
            if fails(f) {
                let _ = writeln!(
                    out,
                    "      <failure type=\"{}\" message=\"{}\"/>",
                    f.severity.label(),
                    xml_escape(&f.message)
                );
            } else {
                let _ = writeln!(
                    out,
                    "      <system-out>[{}] {}</system-out>",
                    f.severity.upper(),
                    xml_escape(&f.message)
                );
            }
            out.push_str("    </testcase>\n");
        }
        out.push_str("  </testsuite>\n");
    }
    out.push_str("</testsuites>\n");
    out
}

/// Escapes the five XML special characters in text and attribute values,
/// plus line breaks, which attribute normalization would turn into spaces.
fn xml_escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '\n' => out.push_str("&#10;"),
            '\r' => out.push_str("&#13;"),
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CheckId, Position, RuleClass};

    /// Offsets resolve against one in-memory text per document.
    struct Texts(Vec<(Uri, &'static str, &'static str)>);

    impl Sources for Texts {
        fn line_col(&self, doc: &Uri, offset: usize) -> Option<(u32, u32)> {
            let (_, _, text) = self.0.iter().find(|(u, _, _)| u == doc)?;
            let before = &text[..offset];
            let line = before.matches('\n').count();
            let col = before.len() - before.rfind('\n').map_or(0, |i| i + 1);
            Some((line as u32, col as u32))
        }
        fn display_path(&self, doc: &Uri) -> String {
            self.0
                .iter()
                .find(|(u, _, _)| u == doc)
                .map_or_else(|| doc.to_string(), |(_, shown, _)| (*shown).to_owned())
        }
    }

    fn uri() -> Uri {
        Uri::parse("file:///work/spec.yaml").unwrap()
    }

    fn sources() -> Texts {
        Texts(vec![(
            uri(),
            "spec.yaml",
            "openapi: 3.1.0\ninfo:\n  title: t\npaths: {}\n",
        )])
    }

    fn sample() -> Vec<Finding> {
        vec![
            Finding {
                position: Some(Position { line: 11, col: 2 }),
                ..Finding::new(
                    CheckId::Validate,
                    "oas-schema-instance-invalid",
                    Severity::Error,
                    uri(),
                    "default violates the schema",
                )
            },
            Finding {
                position: Some(Position { line: 19, col: 4 }),
                ..Finding::new(
                    CheckId::Validate,
                    "oas-schema-instance-invalid",
                    Severity::Warning,
                    uri(),
                    "example violates the schema",
                )
            },
        ]
    }

    #[test]
    fn text_lines_resolve_ranges_and_positions() {
        let findings = vec![
            Finding::new(
                CheckId::Lint,
                "info-contact",
                Severity::Warning,
                uri(),
                "Info object should contain a contact object.",
            )
            .with_range(23..34),
            Finding {
                position: Some(Position { line: 7, col: 6 }),
                ..Finding::new(
                    CheckId::Command,
                    "breaking-change",
                    Severity::Error,
                    uri(),
                    "removed",
                )
            },
            Finding::new(CheckId::Command, "io-error", Severity::Error, uri(), "gone"),
        ];
        let mut out = Vec::new();
        text(&findings, &sources(), &mut out).unwrap();
        assert_eq!(
            String::from_utf8(out).unwrap(),
            "spec.yaml:3:3 [WARNING] info-contact: Info object should contain a contact object.\n\
             spec.yaml:8:7 [ERROR] breaking-change: removed\n\
             spec.yaml:1:1 [ERROR] io-error: gone\n"
        );
    }

    #[test]
    fn json_carries_locations_and_check() {
        let mut finding = Finding::new(
            CheckId::Lint,
            "info-contact",
            Severity::Warning,
            uri(),
            "no contact",
        )
        .with_range(23..34);
        finding.pointer = Some("/info".into());
        let value = json(&[finding], &sources());
        assert_eq!(
            value,
            json!([{
                "code": "info-contact",
                "severity": "warning",
                "message": "no contact",
                "check": "lint",
                "file": "spec.yaml",
                "line": 3,
                "col": 3,
                "range": {"start": 23, "end": 34},
                "pointer": "/info",
            }])
        );
    }

    #[test]
    fn the_report_lists_suppressed_findings_apart() {
        let shown = Finding::new(CheckId::Lint, "a", Severity::Warning, uri(), "shown");
        let hidden = Finding::new(CheckId::Lint, "b", Severity::Warning, uri(), "hidden");
        let report = json_report(&[shown], &[hidden], &sources());
        assert_eq!(report["findings"][0]["code"], "a");
        assert_eq!(report["suppressed"][0]["code"], "b");
        assert_eq!(report["suppressed"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn log_is_valid_sarif_210() {
        let log = sarif(&sample(), &sources(), &NoRules);
        assert_eq!(log["version"], "2.1.0");
        assert_eq!(
            log["$schema"],
            "https://json.schemastore.org/sarif-2.1.0.json"
        );
        let run = &log["runs"][0];
        assert_eq!(run["tool"]["driver"]["name"], "suspect");
        // Two findings share one rule: one deduplicated rule entry.
        let rules = run["tool"]["driver"]["rules"].as_array().unwrap();
        assert_eq!(rules.len(), 1);
        assert_eq!(
            rules[0]["shortDescription"]["text"],
            "oas-schema-instance-invalid"
        );
        assert!(rules[0].get("help").is_none());
        let results = run["results"].as_array().unwrap();
        assert_eq!(results.len(), 2);
        assert_eq!(results[0]["level"], "error");
        assert_eq!(results[1]["level"], "warning");
        assert_eq!(results[1]["ruleIndex"], 0);
        let location = &results[0]["locations"][0]["physicalLocation"];
        assert_eq!(location["artifactLocation"]["uri"], "spec.yaml");
        assert_eq!(location["region"]["startLine"], 12);
        assert_eq!(location["region"]["startColumn"], 3);
        // Required SARIF 2.1.0 members on every result.
        for result in results {
            for key in ["ruleId", "message", "locations", "level"] {
                assert!(result.get(key).is_some(), "result without {key}");
            }
            assert!(result["message"]["text"].is_string());
        }
    }

    #[test]
    fn sarif_uses_registered_rule_text() {
        struct One;
        impl RuleLookup for One {
            fn rule(&self, code: &str) -> Option<RuleInfo> {
                (code == "oas-schema-instance-invalid").then_some(RuleInfo {
                    how_to_fix: "Fix the value.",
                    class: RuleClass::Document,
                    ..RuleInfo::new(
                        "oas-schema-instance-invalid",
                        CheckId::Validate,
                        Severity::Warning,
                        "Value violates its schema",
                    )
                })
            }
        }
        let log = sarif(&sample(), &sources(), &One);
        let rule = &log["runs"][0]["tool"]["driver"]["rules"][0];
        assert_eq!(
            rule["shortDescription"]["text"],
            "Value violates its schema"
        );
        assert_eq!(rule["help"]["text"], "Fix the value.");
    }

    #[test]
    fn junit_groups_by_document_and_fails_on_warnings() {
        let other = Uri::parse("file:///work/other.yaml").unwrap();
        let mut findings = sample();
        findings.push(Finding {
            position: Some(Position { line: 0, col: 0 }),
            ..Finding::new(
                CheckId::Lint,
                "info-description",
                Severity::Hint,
                other,
                "a <b> & \"c\"",
            )
        });
        let expected = [
            r#"<?xml version="1.0" encoding="UTF-8"?>"#,
            r#"<testsuites name="suspect" tests="3" failures="2" errors="0">"#,
            r#"  <testsuite name="spec.yaml" tests="2" failures="2" errors="0">"#,
            r#"    <testcase name="oas-schema-instance-invalid" classname="spec.yaml:12:3">"#,
            r#"      <failure type="error" message="default violates the schema"/>"#,
            r#"    </testcase>"#,
            r#"    <testcase name="oas-schema-instance-invalid" classname="spec.yaml:20:5">"#,
            r#"      <failure type="warning" message="example violates the schema"/>"#,
            r#"    </testcase>"#,
            r#"  </testsuite>"#,
            r#"  <testsuite name="file:///work/other.yaml" tests="1" failures="0" errors="0">"#,
            r#"    <testcase name="info-description" classname="file:///work/other.yaml:1:1">"#,
            r#"      <system-out>[HINT] a &lt;b&gt; &amp; &quot;c&quot;</system-out>"#,
            r#"    </testcase>"#,
            r#"  </testsuite>"#,
            r#"</testsuites>"#,
            "",
        ]
        .join("\n");
        assert_eq!(junit(&findings, &sources()), expected);
        assert_eq!(xml_escape("a\nb\r\n'"), "a&#10;b&#13;&#10;&apos;");
        assert_eq!(
            junit(&[], &sources()),
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
             <testsuites name=\"suspect\" tests=\"0\" failures=\"0\" errors=\"0\">\n\
             </testsuites>\n"
        );
    }

    #[test]
    fn sarif_levels() {
        assert_eq!(sarif_level(Severity::Error), "error");
        assert_eq!(sarif_level(Severity::Warning), "warning");
        assert_eq!(sarif_level(Severity::Info), "note");
        assert_eq!(sarif_level(Severity::Hint), "note");
    }
}
