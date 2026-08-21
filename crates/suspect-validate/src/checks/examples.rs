//! Example-vs-schema type compatibility check.

use suspect_low::ValueKind;
use suspect_oas::{MediaType, OpenApi};

use super::diag;
use crate::diagnostic::{Diagnostic, Severity};

/// TypeSet bits a value of `kind` can legitimately inhabit.
fn compatible_bits(kind: ValueKind) -> u8 {
    use suspect_oas::TypeSet;
    match kind {
        ValueKind::Null => TypeSet::NULL,
        ValueKind::Bool => TypeSet::BOOL,
        // JSON: an integer is also a valid number.
        ValueKind::Int => TypeSet::INTEGER | TypeSet::NUMBER,
        ValueKind::Float => TypeSet::NUMBER,
        ValueKind::Str => TypeSet::STRING,
        ValueKind::Object => TypeSet::OBJECT,
        ValueKind::Array => TypeSet::ARRAY,
    }
}

/// `oas-example-type-mismatch` (Warning): `MediaType.example` contradicts
/// every type in the schema's type set. Skipped for oneOf/anyOf schemas and
/// cyclic refs.
pub(crate) fn check_example_types(api: &OpenApi<'_>, out: &mut Vec<Diagnostic>) {
    for op in api.operations() {
        let mut medias: Vec<MediaType<'_>> = Vec::new();
        if let Some(rb) = op.request_body() {
            medias.extend(rb.content().into_iter().map(|(_, m)| m));
        }
        if let Some(responses) = op.responses() {
            for (_, response) in responses.iter() {
                medias.extend(response.resolved().content().into_iter().map(|(_, m)| m));
            }
        }
        for param in op.parameters() {
            medias.extend(param.resolved().content().into_iter().map(|(_, m)| m));
        }
        for media in medias {
            check_media_example(api, &media, out);
        }
    }
}

fn check_media_example(api: &OpenApi<'_>, media: &MediaType<'_>, out: &mut Vec<Diagnostic>) {
    let Some(example) = media.example() else { return };
    let Some(schema) = media.schema() else { return };
    if schema.is_cyclic() || schema.resolved().is_cyclic() {
        return;
    }
    let r = schema.resolved();
    if !r.one_of().is_empty() || !r.any_of().is_empty() {
        return;
    }
    let Some(set) = schema.type_() else { return };
    if set.is_empty() {
        return;
    }
    let kind = example.kind();
    // Empty YAML values read as null; not worth a diagnostic.
    if kind == ValueKind::Null {
        return;
    }
    if compatible_bits(kind) & set.bits() == 0 {
        out.push(diag(
            api,
            "oas-example-type-mismatch",
            Severity::Warning,
            example.byte_range(),
            format!("example of kind `{kind:?}` contradicts the schema type set"),
        ));
    }
}
