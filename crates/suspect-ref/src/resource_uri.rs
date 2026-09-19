//! Strict RFC 3986 identifier syntax and resolution, independent of acquisition.
//!
//! Shared lower-layer form of the audited algorithm in suspect-schema's resource
//! resolver. Logical identifiers can be opaque URIs and must not be repaired by
//! a browser URL parser. Resolution folds ASCII scheme/host case, removes only
//! literal dot segments, and otherwise retains encoded path/query spelling.

use iri_string::types::{UriAbsoluteStr, UriReferenceStr};

/// A URI or fragment could not be represented without changing its meaning.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    /// Invalid syntax, including raw whitespace, bad escapes and malformed hosts.
    #[error("invalid RFC 3986 URI-reference syntax")]
    InvalidReference,
    /// A base URI must be absolute and fragment-free.
    #[error("base must be an absolute RFC 3986 URI without a fragment")]
    InvalidBase,
    /// Serializing the result would turn an authorityless path into an authority.
    #[error("RFC 3986 resolution cannot represent this authorityless path")]
    UnrepresentableResolution,
    /// Malformed percent escapes or non-UTF-8 decoded fragment bytes.
    #[error("invalid percent-encoded UTF-8 reference fragment")]
    InvalidFragment,
}

/// Validate a complete URI reference and split its still-encoded fragment.
///
/// # Errors
/// Invalid RFC 3986 syntax in either the document or fragment component.
pub fn split_reference(raw: &str) -> Result<(&str, &str), Error> {
    let reference = UriReferenceStr::new(raw).map_err(|_| Error::InvalidReference)?;
    Ok(match reference.fragment_str() {
        Some(fragment) => (&raw[..raw.len() - fragment.len() - 1], fragment),
        None => (raw, ""),
    })
}

/// Resolve a URI reference and return its fragment-free absolute identifier.
/// This function performs no I/O and grants no permission to retrieve a URI.
///
/// # Errors
/// Invalid base/reference syntax or an unrepresentable authorityless result.
pub fn resolve_document(base: &str, raw: &str) -> Result<String, Error> {
    let base = UriAbsoluteStr::new(base).map_err(|_| Error::InvalidBase)?;
    let reference = UriReferenceStr::new(raw).map_err(|_| Error::InvalidReference)?;
    // RFC 3986 §5.2.2. iri-string's optional normalization also treats encoded
    // dots as dot segments, so use its validated components with the RFC's
    // generic component-resolution algorithm, preserving encoded octets.
    let (scheme, authority, path, query) = if let Some(scheme) = reference.scheme_str() {
        (
            scheme,
            reference.authority_components(),
            remove_dot_segments(reference.path_str()),
            reference.query_str(),
        )
    } else if let Some(authority) = reference.authority_components() {
        (
            base.scheme_str(),
            Some(authority),
            remove_dot_segments(reference.path_str()),
            reference.query_str(),
        )
    } else {
        let (path, query) = if reference.path_str().is_empty() {
            (
                base.path_str().to_owned(),
                reference.query_str().or_else(|| base.query_str()),
            )
        } else {
            let path = if reference.path_str().starts_with('/') {
                reference.path_str().to_owned()
            } else {
                let prefix = if base.authority_str().is_some() && base.path_str().is_empty() {
                    "/"
                } else {
                    base.path_str()
                        .rfind('/')
                        .map_or("", |index| &base.path_str()[..=index])
                };
                format!("{prefix}{}", reference.path_str())
            };
            (remove_dot_segments(&path), reference.query_str())
        };
        (base.scheme_str(), base.authority_components(), path, query)
    };
    if authority.is_none() && path.starts_with("//") {
        return Err(Error::UnrepresentableResolution);
    }
    let mut document = scheme.to_ascii_lowercase();
    document.push(':');
    if let Some(authority) = authority {
        document.push_str("//");
        if let Some(userinfo) = authority.userinfo() {
            document.push_str(userinfo);
            document.push('@');
        }
        document.push_str(&authority.host().to_ascii_lowercase());
        if let Some(port) = authority.port() {
            document.push(':');
            document.push_str(port);
        }
    }
    document.push_str(&path);
    if let Some(query) = query {
        document.push('?');
        document.push_str(query);
    }
    Ok(document)
}

/// Resolve an identifier while preserving its encoded nonempty fragment.
/// An empty fragment names the same primary resource as an absent fragment.
///
/// # Errors
/// The same errors as [`resolve_document`].
pub fn resolve_reference(base: &str, raw: &str) -> Result<String, Error> {
    let (_, fragment) = split_reference(raw)?;
    let document = resolve_document(base, raw)?;
    Ok(if fragment.is_empty() {
        document
    } else {
        format!("{document}#{fragment}")
    })
}

/// Decode a URI fragment once, before interpreting RFC 6901 pointer escaping.
///
/// # Errors
/// Malformed percent escapes or decoded bytes that are not UTF-8.
pub fn decode_fragment(fragment: &str) -> Result<String, Error> {
    let mut decoded = Vec::with_capacity(fragment.len());
    let mut bytes = fragment.bytes();
    while let Some(byte) = bytes.next() {
        decoded.push(if byte == b'%' {
            let hi = char::from(bytes.next().ok_or(Error::InvalidFragment)?)
                .to_digit(16)
                .ok_or(Error::InvalidFragment)?;
            let lo = char::from(bytes.next().ok_or(Error::InvalidFragment)?)
                .to_digit(16)
                .ok_or(Error::InvalidFragment)?;
            u8::try_from(hi * 16 + lo).map_err(|_| Error::InvalidFragment)?
        } else {
            byte
        });
    }
    String::from_utf8(decoded).map_err(|_| Error::InvalidFragment)
}

/// Encode a decoded pointer/name as an RFC 3986 URI fragment, without `#`.
/// Existing `%` characters are data and are encoded as `%25`.
#[must_use]
pub fn encode_fragment(fragment: &str) -> String {
    let mut encoded = String::with_capacity(fragment.len());
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    for byte in fragment.bytes() {
        if byte.is_ascii_alphanumeric() || b"-._~!$&'()*+,;=:@/?".contains(&byte) {
            encoded.push(char::from(byte));
        } else {
            encoded.push('%');
            encoded.push(char::from(HEX[usize::from(byte >> 4)]));
            encoded.push(char::from(HEX[usize::from(byte & 15)]));
        }
    }
    encoded
}

fn remove_dot_segments(mut input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    while !input.is_empty() {
        if let Some(rest) = input
            .strip_prefix("../")
            .or_else(|| input.strip_prefix("./"))
        {
            input = rest;
        } else if input.starts_with("/./") {
            input = &input[2..];
        } else if input == "/." {
            input = "/";
        } else if input.starts_with("/../") || input == "/.." {
            input = if input == "/.." { "/" } else { &input[3..] };
            output.truncate(output.rfind('/').unwrap_or(0));
        } else if input == "." || input == ".." {
            input = "";
        } else {
            let start = usize::from(input.starts_with('/'));
            let end = input[start..]
                .find('/')
                .map_or(input.len(), |index| start + index);
            output.push_str(&input[..end]);
            input = &input[end..];
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc3986_normal_and_abnormal_examples_keep_the_generic_resolution_algorithm() {
        // RFC 3986 §5.4, strict scheme interpretation. Document results omit
        // fragments; none of these expectations comes from URL normalization.
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
                "{reference}"
            );
        }
    }

    #[test]
    fn syntax_is_strict_before_an_unavailable_resource_or_bad_fragment_can_be_ignored() {
        for reference in [
            " leading",
            "trailing ",
            "inside space",
            "a\\b",
            "café",
            "bad%",
            "bad%2",
            "bad%GG",
            "1bad:scheme",
            "https://[broken/a",
            "https://example.test:port/a",
            "https://external.test/a#bad%",
            "a#b#c",
        ] {
            assert!(split_reference(reference).is_err(), "{reference:?}");
            assert!(
                resolve_document("https://example.test/root", reference).is_err(),
                "{reference:?}"
            );
        }
        assert_eq!(
            resolve_document("scheme:", ".///path"),
            Err(Error::UnrepresentableResolution)
        );
    }

    #[test]
    fn encoded_octets_and_fragments_are_decoded_and_encoded_at_their_own_layer() {
        assert_eq!(
            resolve_document("https://a.test/a/", "%2E%2E/x").unwrap(),
            "https://a.test/a/%2E%2E/x"
        );
        assert_eq!(
            resolve_document("https://a.test/a/", "x%2Fy%23z?x=+%2F#part").unwrap(),
            "https://a.test/a/x%2Fy%23z?x=+%2F"
        );
        assert_eq!(
            split_reference("a%23b#%2Fpath").unwrap(),
            ("a%23b", "%2Fpath")
        );
        assert_eq!(
            decode_fragment("%2Fproperties%2Fcaf%C3%A9~1~0%25%20%23").unwrap(),
            "/properties/café~1~0% #"
        );
        assert_eq!(
            encode_fragment("/properties/café~1~0% #"),
            "/properties/caf%C3%A9~1~0%25%20%23"
        );
        assert!(decode_fragment("%FF").is_err());
        assert!(decode_fragment("%").is_err());
        assert_eq!(
            resolve_reference("urn:example:root", "#"),
            Ok("urn:example:root".to_owned())
        );
        assert_eq!(
            resolve_reference("urn:example:root", "#node"),
            Ok("urn:example:root#node".to_owned())
        );
    }
}
