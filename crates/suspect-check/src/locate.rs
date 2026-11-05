//! Pointers for findings whose producer reports only a byte range.

use std::ops::Range;

use suspect_low::{LowDoc, NodeRef, Pointer, ValueKind};

/// The RFC 6901 pointer to the deepest value whose source range contains
/// all of `range`. A range inside a mapping key names that key's entry,
/// so a finding on a path key and one on its path item share a pointer.
/// A range no value contains (or an empty document) gives `""`.
///
/// O(depth x width); the CLI calls it for baseline keys, not the editor.
#[must_use]
pub fn pointer_at(doc: &LowDoc, range: &Range<usize>) -> Box<str> {
    let contains = |outer: Range<usize>| outer.start <= range.start && range.end <= outer.end;
    let mut tokens: Vec<Box<str>> = Vec::new();
    let mut node: NodeRef<'_> = doc.root().resolved();
    loop {
        let next = match node.kind() {
            ValueKind::Object => node.entries().into_iter().find_map(|entry| {
                let value = entry.value?;
                (contains(value.byte_range()) || contains(entry.key_node.byte_range())).then(|| {
                    let key =
                        String::from_utf8_lossy(&entry.key_node.decoded_scalar()).into_owned();
                    (key.into_boxed_str(), value)
                })
            }),
            ValueKind::Array => node
                .items()
                .into_iter()
                .enumerate()
                .find(|(_, item)| contains(item.byte_range()))
                .map(|(i, item)| (i.to_string().into_boxed_str(), item)),
            _ => None,
        };
        let Some((token, child)) = next else { break };
        tokens.push(token);
        // An alias resolves elsewhere; stop at the alias itself.
        if child.is_alias() {
            break;
        }
        node = child;
    }
    Pointer::from_tokens(tokens).to_string().into_boxed_str()
}

#[cfg(test)]
mod tests {
    use super::*;
    use suspect_source::{Source, Uri};

    fn doc(text: &str) -> LowDoc {
        LowDoc::parse(
            Uri::parse("mem://api.yaml").unwrap(),
            Source::from_vec(text.as_bytes().to_vec()),
        )
    }

    #[test]
    fn ranges_map_to_the_deepest_containing_value() {
        let text = "openapi: 3.1.0\npaths:\n  /pets/{id}:\n    get:\n      summary: s\n      tags: [a, b]\n";
        let low = doc(text);
        let at = |needle: &str, len: usize| {
            let start = text.find(needle).unwrap();
            pointer_at(&low, &(start..start + len))
        };
        // The operation mapping starts at its first key.
        let get = text.find("summary").unwrap();
        let end = text.find("tags").unwrap() + "tags: [a, b]".len();
        assert_eq!(&*pointer_at(&low, &(get..end)), "/paths/~1pets~1{id}/get");
        assert_eq!(&*at("/pets/{id}", 10), "/paths/~1pets~1{id}");
        assert_eq!(&*at("b]", 1), "/paths/~1pets~1{id}/get/tags/1");
        assert_eq!(&*pointer_at(&low, &(0..text.len())), "");
    }
}
