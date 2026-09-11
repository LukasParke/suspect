//! Source and lexical vocabulary context for schema declaration validation.

use rustc_hash::{FxHashMap, FxHashSet};
use suspect_low::{NodeRef, ValueKind};
use suspect_oas::{OasVersion, OpenApi, SchemaView};
use suspect_source::Uri;

use super::diag_at;
use crate::{Diagnostic, Severity};

type Address = (Uri, usize, usize, usize);

#[derive(Default)]
pub(super) struct Contexts {
    cache: FxHashMap<Address, Option<bool>>,
    reported: FxHashSet<Address>,
}

impl Contexts {
    pub fn resolve(
        &mut self,
        schema: SchemaView<'_>,
        api: &OpenApi<'_>,
        out: &mut Vec<Diagnostic>,
    ) -> Option<bool> {
        let key = address(schema.node());
        if let Some(context) = self.cache.get(&key) {
            return *context;
        }
        let result = self.resolve_uncached(schema, api, out);
        self.cache.insert(key, result);
        result
    }

    fn resolve_uncached(
        &mut self,
        schema: SchemaView<'_>,
        api: &OpenApi<'_>,
        out: &mut Vec<Diagnostic>,
    ) -> Option<bool> {
        let document = api
            .session()
            .workspace()
            .get(schema.node().syntax().doc().uri())
            .expect("schema source belongs to session");
        let source_version = OasVersion::sniff(document.doc());
        if source_version.unwrap_or(api.version()) == OasVersion::V30 {
            return Some(false);
        }
        let default_root = if source_version.is_some() {
            document.doc().root()
        } else {
            api.root()
        };
        let mut declaration = field(default_root, "jsonSchemaDialect");
        for ancestor in schema.lexical_ancestors() {
            if let Some(value) = field(ancestor.node(), "$schema") {
                declaration = Some(value);
            }
        }
        let Some((at, value)) = declaration else {
            return Some(true);
        };
        let uri = value
            .filter(|node| node.kind() == ValueKind::Str)
            .and_then(|node| node.try_decoded_scalar())
            .and_then(|bytes| String::from_utf8(bytes.into_owned()).ok());
        let Some(uri) = uri else {
            self.report(
                at,
                "oas-schema-invalid-keyword",
                "schema dialect declaration must be a string URI",
                out,
            );
            return None;
        };
        if matches!(
            uri.as_str(),
            "https://spec.openapis.org/oas/3.1/dialect/base"
                | "https://json-schema.org/draft/2020-12/schema"
        ) {
            Some(true)
        } else {
            self.report(
                at,
                "oas-schema-unsupported-dialect",
                &format!(
                    "schema dialect {uri:?} is not supported; declaration rules are not guessed"
                ),
                out,
            );
            None
        }
    }

    fn report(
        &mut self,
        at: NodeRef<'_>,
        code: &'static str,
        message: &str,
        out: &mut Vec<Diagnostic>,
    ) {
        if self.reported.insert(address(at)) {
            out.push(diag_at(at, code, Severity::Error, at.byte_range(), message));
        }
    }
}

fn address(node: NodeRef<'_>) -> Address {
    let range = node.byte_range();
    (
        node.syntax().doc().uri().clone(),
        range.start,
        range.end,
        node.syntax().raw().id(),
    )
}

fn field<'s>(node: NodeRef<'s>, name: &str) -> Option<(NodeRef<'s>, Option<NodeRef<'s>>)> {
    let entry = node.entries().into_iter().find(|entry| {
        entry
            .key_node
            .try_decoded_scalar()
            .is_some_and(|key| key.as_ref() == name.as_bytes())
    })?;
    Some((entry.value.unwrap_or(entry.key_node), entry.value))
}
