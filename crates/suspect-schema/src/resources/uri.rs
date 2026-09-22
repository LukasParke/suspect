//! Strict RFC 3986 syntax and resolution for schema resource identifiers.
//!
//! URL parsers may repair invalid syntax or apply scheme-specific behavior.
//! Schema identifiers need generic URI-reference resolution: parsing must not
//! trim whitespace, decode path delimiters, or reinterpret an explicit scheme.
//! Resource identity folds ASCII scheme/host case (RFC 3986 section 6.2.2.1).
//! Other optional normalization is deliberately separate from resolution.

// The audited implementation now lives below both IR and schema. Keep this
// source-bound compiler seam and its independent RFC vectors, using precisely
// the same URI rules as the immutable Contract resource registry.
pub(super) use suspect_ir::contract::resource_uri::{resolve_document, split_reference};

#[cfg(test)]
mod tests {
    use super::{resolve_document, split_reference};

    #[test]
    fn rfc3986_section_5_4_resolution_examples() {
        // Primary source: https://www.rfc-editor.org/rfc/rfc3986#section-5.4
        // All normal and abnormal examples, selecting the strict-parser
        // interpretation of `http:g`. This helper returns document identity,
        // so expected fragment components are intentionally absent.
        for (reference, expected) in [
            ("g:h", "g:h"),
            ("g", "http://a/b/c/g"),
            ("./g", "http://a/b/c/g"),
            ("g/", "http://a/b/c/g/"),
            ("/g", "http://a/g"),
            ("//g", "http://g"),
            ("?y", "http://a/b/c/d;p?y"),
            ("g?y", "http://a/b/c/g?y"),
            ("#s", "http://a/b/c/d;p?q"),
            ("g#s", "http://a/b/c/g"),
            ("g?y#s", "http://a/b/c/g?y"),
            (";x", "http://a/b/c/;x"),
            ("g;x", "http://a/b/c/g;x"),
            ("g;x?y#s", "http://a/b/c/g;x?y"),
            ("", "http://a/b/c/d;p?q"),
            (".", "http://a/b/c/"),
            ("./", "http://a/b/c/"),
            ("..", "http://a/b/"),
            ("../", "http://a/b/"),
            ("../g", "http://a/b/g"),
            ("../..", "http://a/"),
            ("../../", "http://a/"),
            ("../../g", "http://a/g"),
            ("../../../g", "http://a/g"),
            ("../../../../g", "http://a/g"),
            ("/./g", "http://a/g"),
            ("/../g", "http://a/g"),
            ("g.", "http://a/b/c/g."),
            (".g", "http://a/b/c/.g"),
            ("g..", "http://a/b/c/g.."),
            ("..g", "http://a/b/c/..g"),
            ("./../g", "http://a/b/g"),
            ("./g/.", "http://a/b/c/g/"),
            ("g/./h", "http://a/b/c/g/h"),
            ("g/../h", "http://a/b/c/h"),
            ("g;x=1/./y", "http://a/b/c/g;x=1/y"),
            ("g;x=1/../y", "http://a/b/c/y"),
            ("g?y/./x", "http://a/b/c/g?y/./x"),
            ("g?y/../x", "http://a/b/c/g?y/../x"),
            ("g#s/./x", "http://a/b/c/g"),
            ("g#s/../x", "http://a/b/c/g"),
            ("http:g", "http:g"),
        ] {
            assert_eq!(
                resolve_document("http://a/b/c/d;p?q", reference).unwrap(),
                expected,
                "reference: {reference}",
            );
        }
    }

    #[test]
    fn resolution_uses_the_rfc_algorithm_without_optional_normalization() {
        // Opaque and encoded paths, query distinctions, and literal spelling.
        for (base, reference, expected) in [
            ("http://a/b/c/d;p?q", "g", "http://a/b/c/g"),
            ("http://a/b/c/d;p?q", "../g", "http://a/b/g"),
            ("http://a/b/c/d;p?q", "../../../g", "http://a/g"),
            ("http://a/b/c/d;p?q", "?y", "http://a/b/c/d;p?y"),
            ("http://a/b/c/d;p?q", "?", "http://a/b/c/d;p?"),
            ("http://a/b/c/d;p?q", "#s", "http://a/b/c/d;p?q"),
            ("http://a/b/c/d;p?q", "g?y/../x", "http://a/b/c/g?y/../x"),
            ("http://a/b/c/d;p?q", "http:g", "http:g"),
            ("urn:example:schema", "?v=2", "urn:example:schema?v=2"),
            ("urn:example:schema", "#anchor", "urn:example:schema"),
            (
                "https://example.test/",
                "HTTPS://User:Pass@[2001:DB8::1]:/Path?Key=Value",
                "https://User:Pass@[2001:db8::1]:/Path?Key=Value",
            ),
            (
                "https://example.test/a/",
                "%2E%2E/x",
                "https://example.test/a/%2E%2E/x",
            ),
            (
                "https://example.test/a/",
                "x%2Fy%23z?x=+%2F#part",
                "https://example.test/a/x%2Fy%23z?x=+%2F",
            ),
        ] {
            assert_eq!(resolve_document(base, reference).unwrap(), expected);
        }
    }

    #[test]
    fn fragment_splitting_preserves_encoded_delimiters() {
        assert_eq!(
            split_reference("a%23b#%2Fpath").unwrap(),
            ("a%23b", "%2Fpath")
        );
        assert_eq!(split_reference("?q=one?two#").unwrap(), ("?q=one?two", ""));
        assert_eq!(split_reference("#").unwrap(), ("", ""));
        assert!(split_reference("a#b#c").is_err());
        assert!(split_reference("https://external.test/a#bad%").is_err());
    }
}
