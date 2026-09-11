//! References in OpenAPI/schema positions, excluding arbitrary instance data.

use suspect_oas::OpenApi;
use suspect_ref::{RefError, Resolution};

use super::diag_at;
use crate::{Diagnostic, Severity};

pub(crate) fn check_references(api: &OpenApi<'_>, out: &mut Vec<Diagnostic>) {
    for object in api.reference_objects() {
        let Some(reference) = object.entries().into_iter().find(|entry| {
            entry
                .key_node
                .try_decoded_scalar()
                .is_some_and(|key| key.as_ref() == b"$ref")
        }) else {
            continue;
        };
        let at = reference.value.unwrap_or(reference.key_node);
        let error = match reference.value {
            Some(value) => match api.session().resolve_reference(value) {
                Ok(Resolution::Cycle { .. }) => Some((
                    "unresolved-ref",
                    "reference chain has no concrete target (cycle)".to_owned(),
                )),
                Ok(_) => None,
                Err(error) => Some((
                    match error {
                        RefError::InvalidRef { .. } => "invalid-ref",
                        RefError::OutsideAllowlist { .. } => "ref-outside-allowlist",
                        _ => "unresolved-ref",
                    },
                    error.to_string(),
                )),
            },
            None => Some(("invalid-ref", "$ref requires a string value".to_owned())),
        };
        if let Some((code, message)) = error {
            out.push(diag_at(at, code, Severity::Error, at.byte_range(), message));
        }
    }
}
