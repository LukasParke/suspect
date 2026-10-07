//! Incremental re-parse equivalence: for any edit series, a reparse must
//! equal a from-scratch parse of the same final text — same tree, same
//! byte ranges, same kinds, same decoded scalars. The incremental path is
//! an optimization, never a different tree.

use suspect_low::LowDoc;
use suspect_source::Source;
use suspect_syntax::{Edit, SNode};

/// The whole raw syntax tree, flattened: (kind, byte range, text bytes).
/// Comparing here is the strongest equivalence: every node, every span,
/// every byte of trivia.
fn flatten(node: &SNode<'_>, out: &mut Vec<(String, std::ops::Range<usize>, Vec<u8>)>) {
    out.push((
        format!("{:?}", node.kind()),
        node.byte_range(),
        node.text().to_vec(),
    ));
    for child in node.children() {
        flatten(&child, out);
    }
}

fn tree_shape(doc: &LowDoc) -> Vec<(String, std::ops::Range<usize>, Vec<u8>)> {
    let mut out = Vec::new();
    flatten(&doc.inner().root(), &mut out);
    out
}

fn reparse_equals_parse(initial: &str, edits: &[(usize, usize, &str)]) {
    let mut doc = LowDoc::parse(
        "mem://equivalence.yaml".into(),
        Source::from_vec(initial.as_bytes().to_vec()),
    );
    let mut text = initial.as_bytes().to_vec();
    for (start, end, replacement) in edits {
        let edit = Edit::from_replacement(doc.inner(), *start, *end, replacement.as_bytes());
        let mut next = text.clone();
        next.splice(start..end, replacement.bytes().collect::<Vec<u8>>());
        doc = doc.reparse(Source::from_vec(next.clone()), std::slice::from_ref(&edit));
        text = next;
    }
    let incremental = tree_shape(&doc);
    let fresh = LowDoc::parse(
        "mem://equivalence.yaml".into(),
        Source::from_vec(text.clone()),
    );
    let from_scratch = tree_shape(&fresh);
    assert_eq!(
        incremental.len(),
        from_scratch.len(),
        "same node count for edits {edits:?}"
    );
    for (index, (inc, full)) in incremental.iter().zip(&from_scratch).enumerate() {
        assert_eq!(
            inc.0, full.0,
            "node {index} kind differs for edits {edits:?}"
        );
        assert_eq!(
            inc.1, full.1,
            "node {index} span differs for edits {edits:?}: incremental {inc:?} vs full {full:?}"
        );
        assert_eq!(
            inc.2, full.2,
            "node {index} content differs for edits {edits:?}"
        );
    }
}

#[test]
fn single_line_edits_match_a_fresh_parse() {
    let spec = "openapi: 3.1.0\ninfo:\n  title: Equivalence\n  version: \"1\"\npaths:\n  /a:\n    get:\n      summary: One\n      responses:\n        '200':\n          description: ok\n";
    reparse_equals_parse(spec, &[(spec.len() - 3, spec.len(), "fine")]);
    reparse_equals_parse(
        spec,
        &[(
            spec.find("title: E").unwrap() + 7,
            spec.find("title: E").unwrap() + 8,
            "X",
        )],
    );
}

#[test]
fn line_count_changing_edits_match_a_fresh_parse() {
    let spec = "openapi: 3.1.0\ninfo:\n  title: Equivalence\n  version: \"1\"\npaths:\n  /a:\n    get:\n      summary: One\n      responses:\n        '200':\n          description: ok\n";
    // Insert several new lines inside a block.
    let at = spec.find("summary: One").unwrap();
    reparse_equals_parse(
        spec,
        &[(
            at,
            at + 3,
            "summary_lines:\n      - one\n      - two\n      # comment\n      summary",
        )],
    );
    // Delete across lines.
    let start = spec.find("  title").unwrap();
    let end = spec.find("paths:").unwrap();
    reparse_equals_parse(spec, &[(start, end, "")]);
    // Replace lines with a different count.
    reparse_equals_parse(
        spec,
        &[(at, at + 10, "summary: Two\n      deprecated: true\n")],
    );
}

#[test]
fn multi_edit_series_match_a_fresh_parse() {
    let spec = "openapi: 3.1.0\ninfo:\n  title: Equivalence\n  version: \"1\"\npaths:\n  /a:\n    get:\n      summary: One\n      responses:\n        '200':\n          description: ok\n";
    // A realistic edit session: type, extend to new lines, delete back.
    let at = spec.find("summary: One").unwrap();
    reparse_equals_parse(
        spec,
        &[
            (at + 10, at + 10, "\n      tags: [demo]"),
            (at + 10, at + 22, ""),
            (
                spec.find("description: ok").unwrap(),
                spec.find("description: ok").unwrap() + 2,
                "DE",
            ),
            (at, at + 3, "summary"),
            (
                spec.find('\'').unwrap(),
                spec.find('\'').unwrap() + 5,
                "'418'",
            ),
        ],
    );
}

#[test]
fn anchors_and_aliases_survive_incremental_edits() {
    let spec = "openapi: 3.1.0\ncomponents:\n  schemas:\n    A: &anchor\n      type: string\n    B: *anchor\n";
    // Edit inside the anchored schema; the alias target must stay resolved.
    let at = spec.find("string").unwrap();
    reparse_equals_parse(spec, &[(at, at + 6, "integer")]);
    let mut doc = LowDoc::parse(
        "mem://a.yaml".into(),
        Source::from_vec(spec.as_bytes().to_vec()),
    );
    let edit = Edit::from_replacement(doc.inner(), at, at + 6, b"integer");
    let next = spec.replace("string", "integer");
    doc = doc.reparse(
        Source::from_vec(next.clone().into_bytes()),
        std::slice::from_ref(&edit),
    );
    // The alias resolves to the edited schema through the semantic layer.
    let fresh = LowDoc::parse("mem://a.yaml".into(), Source::from_vec(next.into_bytes()));
    assert_eq!(tree_shape(&doc), tree_shape(&fresh));
}

#[test]
fn block_scalar_and_comment_edits_match_a_fresh_parse() {
    let spec = "openapi: 3.1.0\ninfo:\n  description: |\n    line one\n    line two\n  version: \"1\"\n# trailing comment\n";
    // Edit inside the block scalar.
    let at = spec.find("line one").unwrap();
    reparse_equals_parse(spec, &[(at, at + 8, "edited!")]);
    // Edit the trailing comment.
    let at = spec.find("trailing").unwrap();
    reparse_equals_parse(spec, &[(at, at + 8, "changed")]);
}
