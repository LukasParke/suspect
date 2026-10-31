//! Inline and extension suppression (`apply_policy` step 5).
//!
//! Three forms, each naming codes or aliases:
//!
//! - `# suspect-disable-next-line <code>... [-- reason]` (or the shorter
//!   `# suspect-disable`) on its own line suppresses findings that start
//!   inside the node beginning on the next non-blank, non-comment line: a
//!   mapping pair or a sequence item, with its subtree. As a trailing
//!   comment it applies to the node its own line begins.
//! - `# suspect-disable-file <code>...` anywhere suppresses the codes for
//!   the whole document.
//! - `x-suspect-disable: [<code>, ...]` on any mapping (YAML or JSON)
//!   suppresses findings inside that mapping, and inside the pair or
//!   sequence item it is the value of.
//!
//! `syntax-error` cannot be suppressed. A code no check knows gives
//! `suppression-unknown-code`; a code that suppressed nothing gives
//! `suppression-unused` when `lint.rules.suppression-unused` turns it on.

use std::ops::Range;

use suspect_diag::{CheckId, Finding, Severity};
use suspect_low::LowDoc;
use suspect_syntax::{SNode, SyntaxKind};

use crate::Registry;

/// An unknown or unsuppressible code in a suppression.
pub const UNKNOWN_CODE: &str = "suppression-unknown-code";

/// A suppressed code that matched no finding (off unless configured).
pub const UNUSED: &str = "suppression-unused";

/// The suppression extension key.
pub const EXTENSION_KEY: &str = "x-suspect-disable";

/// Codes that are never suppressed: the document did not parse, so every
/// other finding is suspect.
const UNSUPPRESSIBLE: &[&str] = &[crate::checks::SYNTAX_CODE];

/// How a suppression is written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Form {
    /// `# suspect-disable-next-line` / `# suspect-disable`.
    NextNode,
    /// `# suspect-disable-file`.
    File,
    /// `x-suspect-disable`.
    Extension,
}

/// One suppressed code as written.
#[derive(Debug, Clone)]
pub struct Code {
    /// The code or alias as written.
    pub written: String,
    /// The codes a finding may carry to match: the written one and the
    /// canonical codes it resolves to.
    matches: Vec<String>,
    /// Whether any check knows the code.
    pub known: bool,
    /// Whether it suppressed a finding.
    pub used: bool,
}

/// One suppression in a document.
#[derive(Debug, Clone)]
pub struct Directive {
    /// How it is written.
    pub form: Form,
    /// Findings starting in this byte range are suppressed; `None` is the
    /// whole document.
    pub scope: Option<Range<usize>>,
    /// Where it is written (the comment, or the extension pair).
    pub at: Range<usize>,
    /// The codes it names.
    pub codes: Vec<Code>,
}

impl Directive {
    fn covers(&self, finding: &Finding) -> bool {
        match (&self.scope, &finding.range) {
            (None, _) => true,
            (Some(scope), Some(range)) => scope.contains(&range.start),
            (Some(_), None) => false,
        }
    }
}

/// Every suppression in `doc`, codes resolved through `registry` (and the
/// custom ruleset's codes, `extra_known`). Cheap when the document names
/// none: one byte scan.
#[must_use]
pub fn collect(
    doc: &LowDoc,
    registry: &Registry,
    extra_known: &dyn Fn(&str) -> bool,
) -> Vec<Directive> {
    let bytes = doc.inner().bytes();
    if !contains(bytes, b"suspect-disable") {
        return Vec::new();
    }
    let code = |written: &str| {
        let mut matches = vec![written.to_owned()];
        matches.extend(registry.resolve(written).into_iter().map(str::to_owned));
        Code {
            written: written.to_owned(),
            matches,
            known: registry.is_known(written) || extra_known(written),
            used: false,
        }
    };
    let root = doc.inner().root();
    let mut out = Vec::new();
    for node in root.descendants() {
        match node.kind() {
            SyntaxKind::Comment => {
                let Some((form, codes)) = parse_comment(node.text()) else {
                    continue;
                };
                let scope = match form {
                    Form::File => None,
                    _ => match next_node(doc, &node) {
                        Some(range) => Some(range),
                        // Nothing follows: matches nothing.
                        None => Some(0..0),
                    },
                };
                out.push(Directive {
                    form,
                    scope,
                    at: node.byte_range(),
                    codes: codes.iter().map(|c| code(c)).collect(),
                });
            }
            SyntaxKind::Pair => {
                let Some(key) = node.child_by_field("key") else {
                    continue;
                };
                if key.content().scalar_bytes() != EXTENSION_KEY.as_bytes() {
                    continue;
                }
                let Some(value) = node.child_by_field("value") else {
                    continue;
                };
                // Only a sequence of strings is honoured; anything else is
                // an `extension-schema` error.
                let items = value.content().sequence_items();
                let written: Vec<String> = items
                    .iter()
                    .filter(|item| item.kind() == SyntaxKind::Scalar)
                    .map(|item| String::from_utf8_lossy(item.scalar_bytes()).into_owned())
                    .filter(|code| !code.is_empty())
                    .collect();
                let Some(mapping) = node.parent() else {
                    continue;
                };
                out.push(Directive {
                    form: Form::Extension,
                    scope: Some(owner(mapping).byte_range()),
                    at: node.byte_range(),
                    codes: written.iter().map(|c| code(c)).collect(),
                });
            }
            _ => {}
        }
    }
    out
}

/// The node a suppression on `mapping` covers: the pair or sequence item
/// the mapping is the value of, else the mapping.
fn owner(mapping: SNode<'_>) -> SNode<'_> {
    let mut node = mapping;
    while let Some(parent) = node.parent() {
        if parent.kind() == SyntaxKind::Pair || parent.raw_kind() == "block_sequence_item" {
            return parent;
        }
        if !matches!(parent.raw_kind(), "block_node" | "flow_node") {
            break;
        }
        node = parent;
    }
    mapping
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|w| w[0] == needle[0] && w == needle)
}

/// The form and codes of a suppression comment; `None` for any other
/// comment.
fn parse_comment(text: &[u8]) -> Option<(Form, Vec<String>)> {
    let text = String::from_utf8_lossy(text);
    let body = text.trim_start_matches('#').trim();
    let (keyword, rest) = body.split_once(char::is_whitespace).unwrap_or((body, ""));
    let form = match keyword {
        "suspect-disable-file" => Form::File,
        "suspect-disable-next-line" | "suspect-disable" => Form::NextNode,
        _ => return None,
    };
    let codes = rest.split(" -- ").next().unwrap_or("");
    let codes = codes
        .split(|c: char| c.is_whitespace() || c == ',')
        .filter(|c| !c.is_empty())
        .map(str::to_owned)
        .collect();
    Some((form, codes))
}

/// The byte range of the node a next-node comment covers.
fn next_node(doc: &LowDoc, comment: &SNode<'_>) -> Option<Range<usize>> {
    let bytes = doc.inner().bytes();
    let line_start = bytes[..comment.start_byte()]
        .iter()
        .rposition(|&b| b == b'\n')
        .map_or(0, |i| i + 1);
    let own_line = bytes[line_start..comment.start_byte()]
        .iter()
        .all(|b| matches!(b, b' ' | b'\t'));
    let offset = if own_line {
        // The first non-blank, non-comment line after the comment.
        let mut at = comment.end_byte();
        loop {
            at = at + bytes[at..].iter().position(|&b| b == b'\n')? + 1;
            let first = at
                + bytes[at..]
                    .iter()
                    .position(|b| !matches!(b, b' ' | b'\t'))?;
            match bytes[first] {
                b'\n' | b'\r' | b'#' => {}
                _ => break first,
            }
        }
    } else {
        line_start
            + bytes[line_start..]
                .iter()
                .position(|b| !matches!(b, b' ' | b'\t'))?
    };
    node_starting_at(doc, offset).map(|n| n.byte_range())
}

/// The outermost pair or sequence item that starts exactly at `offset`.
fn node_starting_at(doc: &LowDoc, offset: usize) -> Option<SNode<'_>> {
    let inner = doc.inner();
    let leaf = inner
        .root()
        .raw()
        .descendant_for_byte_range(offset, offset + 1)?;
    let mut node = SNode::new(inner, leaf);
    let mut found = None;
    loop {
        if node.start_byte() < offset {
            break;
        }
        if node.kind() == SyntaxKind::Pair || node.raw_kind() == "block_sequence_item" {
            found = Some(node);
        }
        match node.parent() {
            Some(parent) => node = parent,
            None => break,
        }
    }
    found
}

/// Moves the findings `directives` cover from `findings` into the
/// returned list, marking the codes that matched.
pub fn apply(
    findings: Vec<Finding>,
    doc: &LowDoc,
    directives: &mut [Directive],
) -> (Vec<Finding>, Vec<Finding>) {
    if directives.is_empty() {
        return (findings, Vec::new());
    }
    let mut kept = Vec::with_capacity(findings.len());
    let mut suppressed = Vec::new();
    for finding in findings {
        if finding.document != *doc.uri() || UNSUPPRESSIBLE.contains(&finding.code.as_ref()) {
            kept.push(finding);
            continue;
        }
        let mut hit = false;
        for directive in directives.iter_mut() {
            if !directive.covers(&finding) {
                continue;
            }
            for code in &mut directive.codes {
                if code.matches.iter().any(|c| *c == finding.code) {
                    code.used = true;
                    hit = true;
                }
            }
        }
        if hit {
            suppressed.push(finding);
        } else {
            kept.push(finding);
        }
    }
    (kept, suppressed)
}

/// Findings about the suppressions themselves: unknown, unsuppressible
/// and (when `unused` is set) unused codes, and directives naming none.
#[must_use]
pub fn problems(doc: &LowDoc, directives: &[Directive], unused: Option<Severity>) -> Vec<Finding> {
    let mut out = Vec::new();
    let finding = |code: &'static str, severity, message: String, at: &Range<usize>| {
        Finding::new(
            CheckId::Suppression,
            code,
            severity,
            doc.uri().clone(),
            message,
        )
        .with_range(at.clone())
    };
    for directive in directives {
        if directive.codes.is_empty() {
            out.push(finding(
                UNKNOWN_CODE,
                Severity::Warning,
                "suppression names no code".to_owned(),
                &directive.at,
            ));
        }
        for code in &directive.codes {
            let written = &code.written;
            if UNSUPPRESSIBLE.contains(&written.as_str()) {
                out.push(finding(
                    UNKNOWN_CODE,
                    Severity::Warning,
                    format!("`{written}` cannot be suppressed"),
                    &directive.at,
                ));
            } else if !code.known {
                out.push(finding(
                    UNKNOWN_CODE,
                    Severity::Warning,
                    format!("suppression names unknown code `{written}`"),
                    &directive.at,
                ));
            } else if let Some(severity) = unused
                && !code.used
            {
                out.push(finding(
                    UNUSED,
                    severity,
                    format!("suppression of `{written}` matched no finding"),
                    &directive.at,
                ));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn comments_parse_form_codes_and_reason() {
        assert_eq!(
            parse_comment(b"# suspect-disable-next-line a b,c -- owned elsewhere"),
            Some((Form::NextNode, vec!["a".into(), "b".into(), "c".into()]))
        );
        assert_eq!(
            parse_comment(b"#suspect-disable x"),
            Some((Form::NextNode, vec!["x".into()]))
        );
        assert_eq!(
            parse_comment(b"# suspect-disable-file y"),
            Some((Form::File, vec!["y".into()]))
        );
        assert_eq!(
            parse_comment(b"# suspect-disable-file"),
            Some((Form::File, vec![]))
        );
        assert_eq!(parse_comment(b"# suspect-disabled x"), None);
        assert_eq!(parse_comment(b"# a note"), None);
    }
}
