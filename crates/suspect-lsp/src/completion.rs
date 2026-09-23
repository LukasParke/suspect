//! Context-aware key and `$ref`-target completion.

use suspect_low::NodeRef;
use suspect_ref::Workspace;
use suspect_source::Uri;
use suspect_syntax::{SNode, SyntaxKind};
use tower_lsp::lsp_types::{CompletionItem, CompletionItemKind};

use crate::navigation::node_at;

/// Keys valid inside an operation object.
pub const OPERATION_KEYS: &[&str] = &[
    "tags",
    "summary",
    "description",
    "externalDocs",
    "operationId",
    "parameters",
    "requestBody",
    "responses",
    "callbacks",
    "deprecated",
    "security",
    "servers",
];

/// Keys valid inside a schema object.
pub const SCHEMA_KEYS: &[&str] = &[
    "type",
    "format",
    "title",
    "description",
    "default",
    "example",
    "enum",
    "const",
    "required",
    "properties",
    "items",
    "prefixItems",
    "additionalProperties",
    "patternProperties",
    "allOf",
    "anyOf",
    "oneOf",
    "not",
    "$ref",
    "$defs",
    "discriminator",
    "xml",
    "deprecated",
    "readOnly",
    "writeOnly",
    "minLength",
    "maxLength",
    "pattern",
    "minimum",
    "maximum",
    "exclusiveMinimum",
    "exclusiveMaximum",
    "multipleOf",
    "minItems",
    "maxItems",
    "uniqueItems",
    "contains",
    "minContains",
    "maxContains",
    "minProperties",
    "maxProperties",
    "propertyNames",
    "unevaluatedProperties",
    "unevaluatedItems",
    "dependentSchemas",
    "dependentRequired",
    "if",
    "then",
    "else",
    "$schema",
    "$id",
    "$anchor",
    "examples",
    "contentMediaType",
    "contentSchema",
];

const METHODS: &[&str] = &[
    "get", "put", "post", "delete", "options", "head", "patch", "trace",
];

/// Keys whose value is always a schema — seeing one among the ancestor keys
/// means we are completing inside a schema. Includes `schema` itself, whose
/// value is a schema in media types and parameter objects.
const SCHEMA_PARENT_KEYS: &[&str] = &[
    "properties",
    "schema",
    "items",
    "prefixItems",
    "additionalProperties",
    "patternProperties",
    "allOf",
    "anyOf",
    "oneOf",
    "not",
    "$defs",
];

/// Swagger 2.0 root document keys.
pub const SWAGGER_ROOT_KEYS: &[&str] = &[
    "swagger",
    "info",
    "host",
    "basePath",
    "schemes",
    "consumes",
    "produces",
    "paths",
    "definitions",
    "parameters",
    "responses",
    "securityDefinitions",
    "security",
    "tags",
    "externalDocs",
];

/// Keys valid inside a Swagger 2.0 operation object.
pub const SWAGGER_OPERATION_KEYS: &[&str] = &[
    "tags",
    "summary",
    "description",
    "externalDocs",
    "operationId",
    "consumes",
    "produces",
    "parameters",
    "responses",
    "schemes",
    "deprecated",
    "security",
];

/// Keys valid inside a Swagger 2.0 parameter object (schema-carrying).
pub const SWAGGER_PARAM_KEYS: &[&str] = &[
    "name",
    "in",
    "description",
    "required",
    "schema",
    "type",
    "format",
    "items",
    "collectionFormat",
    "default",
    "minimum",
    "exclusiveMinimum",
    "maximum",
    "exclusiveMaximum",
    "minLength",
    "maxLength",
    "pattern",
    "minItems",
    "maxItems",
    "uniqueItems",
    "enum",
    "example",
];

/// Keys valid inside a Swagger 2.0 definition/schema object.
pub const SWAGGER_SCHEMA_KEYS: &[&str] = &[
    "$ref",
    "format",
    "title",
    "description",
    "default",
    "example",
    "enum",
    "required",
    "items",
    "properties",
    "additionalProperties",
    "type",
    "discriminator",
    "xml",
    "readOnly",
    "externalDocs",
    "minimum",
    "exclusiveMinimum",
    "maximum",
    "exclusiveMaximum",
    "multipleOf",
    "minLength",
    "maxLength",
    "pattern",
    "maxItems",
    "minItems",
    "uniqueItems",
    "maxProperties",
    "minProperties",
];

/// Keys valid inside a security scheme object.
pub const SCHEME_KEYS: &[&str] = &[
    "type",
    "description",
    "name",
    "in",
    "scheme",
    "bearerFormat",
    "flows",
    "openIdConnectUrl",
];

/// Keys valid directly under a path item or webhook mapping: the HTTP
/// methods plus the path-item-level fields.
const PATH_ITEM_KEYS: &[&str] = &[
    "get",
    "put",
    "post",
    "delete",
    "options",
    "head",
    "patch",
    "trace",
    "$ref",
    "summary",
    "description",
    "servers",
    "parameters",
];

/// Component sections that hold named entries addressable via `$ref`.
const COMPONENT_SECTIONS: &[&str] = &[
    "schemas",
    "responses",
    "parameters",
    "examples",
    "requestBodies",
    "headers",
    "securitySchemes",
    "links",
    "callbacks",
    "pathItems",
];

/// JSON-Schema primitive type names offered for `type:` under a schema.
pub const SCHEMA_TYPES: &[&str] = &[
    "string", "number", "integer", "boolean", "object", "array", "null",
];

/// Security scheme types offered for `type:` under a security scheme.
pub const SCHEME_TYPES: &[&str] = &["apiKey", "http", "oauth2", "openIdConnect"];

/// Parameter locations offered for `in:` under a parameter object.
pub const PARAM_IN: &[&str] = &["query", "header", "path", "cookie"];

/// Parameter serialization styles offered for `style:`.
pub const PARAM_STYLE: &[&str] = &[
    "form",
    "simple",
    "spaceDelimited",
    "pipeDelimited",
    "deepObject",
];

/// Common formats offered for `format:` under a schema (subset of the
/// OAS + JSON-Schema format registry that editors actually type).
pub const SCHEMA_FORMATS: &[&str] = &[
    "int32",
    "int64",
    "float",
    "double",
    "byte",
    "binary",
    "date",
    "date-time",
    "password",
    "email",
    "uuid",
    "uri",
    "uri-reference",
    "json-pointer",
    "relative-json-pointer",
    "regex",
    "ipv4",
    "ipv6",
    "duration",
    "time",
];

/// Common media types offered as `content:` map keys.
pub const MEDIA_TYPES: &[&str] = &[
    "application/json",
    "application/xml",
    "application/x-www-form-urlencoded",
    "application/octet-stream",
    "multipart/form-data",
    "text/plain",
    "text/html",
    "text/event-stream",
    "application/yaml",
];

/// What kind of completion applies at `offset`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompletionContext {
    /// Mapping-key position: offer these keys.
    Keys(&'static [&'static str]),
    /// Inside a `$ref` string value: offer component pointers.
    Refs,
    /// Scalar value position with a fixed vocabulary; items are `VALUE`
    /// kind with per-value documentation attached on resolve.
    Values(&'static [&'static str]),
    /// Value/key position naming an entry of `components/<section>`
    /// (security requirement keys, discriminator mapping values): offer the
    /// declared component names, each resolvable to its target preview.
    ComponentNames(&'static str),
    /// Value position of a `links.*.operationId`: offer the operationIds
    /// declared in the current document.
    OperationIds,
    /// Scalar item inside an operation's `tags` sequence: offer the names
    /// declared in the root `tags` list.
    TagNames,
    /// Mapping-key position under a `content:` mapping: offer common
    /// media types.
    MediaTypes,
    /// Scalar item of a schema's `required` array: offer the schema's
    /// declared property names.
    SchemaPropertyNames(Vec<String>),
    /// No opinion.
    None,
}

/// Classifies what should be completed at `offset`.
///
/// A `$ref` string value wins over everything ([`CompletionContext::Refs`]);
/// a scalar value position with a recognizable owner key yields a fixed
/// vocabulary or a workspace-derived name list; a mapping-key position is
/// classified by the owning mapping's pointer path into key or media-type
/// offers; anything else yields [`CompletionContext::None`].
#[must_use]
pub fn context_at(low: &suspect_low::LowDoc, offset: usize) -> CompletionContext {
    if let Some(node) = node_at(low, offset) {
        // Inside a `$ref` string value?
        if let Some(ctx) = ref_value_context(node) {
            return ctx;
        }

        // Scalar value position: vocabulary picked by the owning pair's key and
        // the ancestor chain (schema types, parameter locations, tags, …).
        if let Some(ctx) = value_context(node, offset) {
            return ctx;
        }

        // Mapping-key position: the node (or an ancestor) is the key side of a
        // pair whose key range contains the offset.
        let mut cur = Some(node);
        while let Some(n) = cur {
            if n.kind() == SyntaxKind::Pair {
                if let Some(key) = n.child_by_field("key") {
                    let kr = key.byte_range();
                    if kr.start <= offset && offset <= kr.end {
                        return key_context(low, n);
                    }
                }
                break;
            }
            cur = n.parent();
        }
    }
    // No context from the node at the offset. Line ends and empty value
    // positions (`in:` with the cursor after the colon) sit between nodes:
    // probe backwards to the nearest node and classify by *its* pair, using
    // the original offset for the key-side guard.
    fallback_value_context(low, offset)
}

/// Backward probe for value positions with no node at `offset`.
fn fallback_value_context(low: &suspect_low::LowDoc, offset: usize) -> CompletionContext {
    let bytes = low.inner().bytes();
    let mut probe = offset.min(bytes.len());
    while probe > 0 {
        probe -= 1;
        if let Some(node) = node_at(low, probe)
            && let Some(ctx) = value_context(node, offset)
        {
            return ctx;
        }
    }
    CompletionContext::None
}

/// `Refs` when `offset` sits inside a `$ref` pair's value region.
fn ref_value_context(node: SNode<'_>) -> Option<CompletionContext> {
    let mut cur = Some(node);
    while let Some(n) = cur {
        if n.kind() == SyntaxKind::Pair {
            let key = n.child_by_field("key")?;
            if key.scalar_bytes() != b"$ref" {
                return None;
            }
            let value = n.child_by_field("value")?;
            let (vr, nr) = (value.byte_range(), node.byte_range());
            return (vr.start <= nr.start && nr.end <= vr.end).then_some(CompletionContext::Refs);
        }
        cur = n.parent();
    }
    None
}

/// Nearest keys of the ancestor pairs above `pair`, nearest first.
fn ancestor_keys(pair: SNode<'_>) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = pair.parent();
    while let Some(n) = cur {
        if n.kind() == SyntaxKind::Pair
            && let Some(key) = n.child_by_field("key")
        {
            out.push(String::from_utf8_lossy(key.scalar_bytes()).into_owned());
        }
        cur = n.parent();
    }
    out
}

/// Scalar item under an operation's or webhook's `tags` sequence: offer the
/// root-declared tag names.
fn tags_item_context(node: SNode<'_>) -> Option<CompletionContext> {
    let mut cur = node.parent();
    while let Some(n) = cur {
        if n.kind() == SyntaxKind::Sequence {
            // The sequence's owning pair must be `tags`.
            let mut up = n.parent();
            while let Some(p) = up {
                if p.kind() == SyntaxKind::Pair {
                    let key = p.child_by_field("key")?;
                    if key.scalar_bytes() != b"tags" {
                        return None;
                    }
                    // Operation tags (not the root list): an ancestor pair
                    // is a path item owner.
                    let ancestors = ancestor_keys(p);
                    return ancestors
                        .iter()
                        .any(|k| k == "paths" || k == "webhooks")
                        .then_some(CompletionContext::TagNames);
                }
                up = p.parent();
            }
            return None;
        }
        cur = n.parent();
    }
    None
}

/// Scalar value position: vocabulary picked by the owning pair's key and
/// the ancestor chain (schema types, parameter locations, security scheme
/// names, discriminator mapping values, required property names, …).
fn value_context(node: SNode<'_>, offset: usize) -> Option<CompletionContext> {
    // Bare scalars inside a tags sequence complete tag names.
    if node.kind() == SyntaxKind::Scalar
        && let Some(ctx) = tags_item_context(node)
    {
        return Some(ctx);
    }
    // Required-array items complete the schema's declared properties.
    if node.kind() == SyntaxKind::Scalar
        && let Some(ctx) = required_item_context(node)
    {
        return Some(ctx);
    }
    // Find the nearest pair ancestor; classify by its key when the offset
    // sits in the value region.
    let mut cur = Some(node);
    let pair = loop {
        let n = cur?;
        if n.kind() == SyntaxKind::Pair {
            break n;
        }
        cur = n.parent();
    };
    let key = pair.child_by_field("key")?;
    if offset <= key.byte_range().end {
        return None; // key side — handled by [`key_context`]
    }
    // The value must be scalar-ish; collections have their own rules.
    if let Some(v) = pair.child_by_field("value")
        && matches!(
            NodeRef::new(v.content()).kind(),
            suspect_low::ValueKind::Object | suspect_low::ValueKind::Array
        )
    {
        return None;
    }
    let key_text = String::from_utf8_lossy(key.scalar_bytes());
    let ancestors = ancestor_keys(pair);
    // Schema context: an ancestor key is schema-valued, or the schema is a
    // named component (`components/schemas/<Name>` — its ancestor chain
    // carries the section key).
    let schema_ctx = ancestors
        .iter()
        .any(|k| SCHEMA_PARENT_KEYS.contains(&k.as_str()))
        || ancestors.windows(2).any(|w| w[1] == "schemas");
    // Discriminator mapping entry values name schema components.
    if ancestors.iter().any(|k| k == "mapping") && ancestors.iter().any(|k| k == "discriminator") {
        return Some(CompletionContext::ComponentNames("schemas"));
    }
    // Required-array item scalars: property names of the owning schema.
    Some(match key_text.as_ref() {
        "type" if ancestors.iter().any(|k| k == "securitySchemes") => {
            CompletionContext::Values(SCHEME_TYPES)
        }
        "type" if schema_ctx => CompletionContext::Values(SCHEMA_TYPES),
        "in" if ancestors.iter().any(|k| k == "parameters") => CompletionContext::Values(PARAM_IN),
        "style" if ancestors.iter().any(|k| k == "parameters") => {
            CompletionContext::Values(PARAM_STYLE)
        }
        "format" if schema_ctx => CompletionContext::Values(SCHEMA_FORMATS),
        "operationId" if ancestors.iter().any(|k| k == "links") => CompletionContext::OperationIds,
        _ => return None,
    })
}

/// Items of a schema's `required` sequence: offer the property names
/// declared by the owning schema.
fn required_item_context(node: SNode<'_>) -> Option<CompletionContext> {
    let mut cur = node.parent();
    while let Some(n) = cur {
        if n.kind() == SyntaxKind::Sequence {
            let mut up = n.parent();
            while let Some(p) = up {
                if p.kind() == SyntaxKind::Pair {
                    let key = p.child_by_field("key")?;
                    if key.scalar_bytes() != b"required" {
                        return None;
                    }
                    // The schema mapping is the pair's owning mapping.
                    let properties = NodeRef::new(p.parent()?.content()).get("properties")?;
                    let names: Vec<String> = properties
                        .entries()
                        .iter()
                        .map(|e| e.key.to_owned())
                        .collect();
                    return (!names.is_empty())
                        .then_some(CompletionContext::SchemaPropertyNames(names));
                }
                up = p.parent();
            }
            return None;
        }
        cur = n.parent();
    }
    None
}

/// Classifies the mapping that owns a key-position pair.
fn key_context(low: &suspect_low::LowDoc, pair: suspect_syntax::SNode<'_>) -> CompletionContext {
    // Swagger 2.0 documents route to their own context tree; the root
    // mapping carries no tokens, so root keys complete the 2.0 vocabulary.
    if low.sniff_family() == suspect_low::SpecFamily::Oas2 {
        let ptr = NodeRef::new(pair.parent().map(|p| p.content()).unwrap_or(pair.content()))
            .path_from_root();
        let owned: Vec<String> = ptr.tokens().iter().map(|t| t.to_string()).collect();
        return swagger_context(low, &owned);
    }
    // The owning mapping is the pair's structural ancestor.
    let mut mapping = pair.parent();
    while let Some(m) = mapping {
        if matches!(m.kind(), SyntaxKind::Mapping) {
            break;
        }
        mapping = m.parent();
    }
    let Some(mapping) = mapping else {
        return CompletionContext::None;
    };
    let ptr = NodeRef::new(mapping.content()).path_from_root();
    let tokens = ptr.tokens();

    // Any ancestor key that is schema-valued puts us in schema context.
    // Checked first so inline schemas under operations (e.g. under a
    // media type's `schema` key) classify as schema keys, not operation keys.
    let mut cur = Some(pair);
    while let Some(n) = cur {
        if n.kind() == SyntaxKind::Pair
            && let Some(key) = n.child_by_field("key")
        {
            let k = String::from_utf8_lossy(key.scalar_bytes());
            if SCHEMA_PARENT_KEYS.contains(&k.as_ref()) {
                return CompletionContext::Keys(SCHEMA_KEYS);
            }
        }
        cur = n.parent();
    }
    // Directly under a path item or webhook mapping: methods plus the
    // path-item-level keys.
    if tokens.len() == 2 && matches!(tokens[0].as_ref(), "paths" | "webhooks") {
        return CompletionContext::Keys(PATH_ITEM_KEYS);
    }
    // Media-type keys: the owning mapping sits under `content`.
    if tokens.last().is_some_and(|t| t.as_ref() == "content") {
        return CompletionContext::MediaTypes;
    }
    // Security requirement keys: scheme names from securitySchemes.
    if tokens.len() >= 2 && tokens[tokens.len() - 2].as_ref() == "security" {
        return CompletionContext::ComponentNames("securitySchemes");
    }
    // Security scheme object keys: components / securitySchemes / <name>.
    if tokens.len() >= 2
        && tokens[0].as_ref() == "components"
        && tokens[1].as_ref() == "securitySchemes"
    {
        return CompletionContext::Keys(SCHEME_KEYS);
    }
    // The operation object itself: paths / <path> / <method>. Deeper
    // nesting is handled by the schema-parent scan above or yields nothing.
    if tokens.first().is_some_and(|t| t.as_ref() == "paths") && tokens.len() == 3 {
        let method = tokens[2].as_ref();
        if METHODS.contains(&method) {
            return CompletionContext::Keys(OPERATION_KEYS);
        }
    }
    if tokens.len() >= 2 && tokens[0].as_ref() == "components" && tokens[1].as_ref() == "schemas" {
        return CompletionContext::Keys(SCHEMA_KEYS);
    }
    let _ = low;
    CompletionContext::None
}

/// All `#/components/...` pointers across loaded documents. Same-document
/// candidates are fragment-only; others use a path relative to `current`
/// (`other.yaml#/components/schemas/Name`).
#[must_use]
pub fn ref_candidates(ws: &Workspace, current: &Uri) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for uri in ws.uris() {
        let Some(h) = ws.get(&uri) else { continue };
        let Some(components) = h.doc().root().get("components") else {
            continue;
        };
        for section in COMPONENT_SECTIONS {
            let Some(sec_node) = components.get(section) else {
                continue;
            };
            for entry in sec_node.entries() {
                let prefix = if uri == *current {
                    "#/".to_owned()
                } else {
                    format!("{}#/", relative_ref(current, &uri))
                };
                out.push(format!("{prefix}components/{section}/{}", entry.key));
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

/// Relative path reference from document `from`'s directory to `to`.
fn relative_ref(from: &Uri, to: &Uri) -> String {
    let (Some(f), Some(t)) = (from.as_path(), to.as_path()) else {
        return to.as_str().to_owned();
    };
    let fdir = f.parent().unwrap_or(std::path::Path::new("."));
    if let Ok(rel) = t.strip_prefix(fdir) {
        return rel.to_string_lossy().into_owned();
    }
    let fc: Vec<_> = fdir.components().collect();
    let tc: Vec<_> = t.components().collect();
    let common = fc.iter().zip(tc.iter()).take_while(|(a, b)| a == b).count();
    let mut parts: Vec<String> = fc[common..].iter().map(|_| "..".to_owned()).collect();
    parts.extend(
        tc[common..]
            .iter()
            .map(|c| c.as_os_str().to_string_lossy().into_owned()),
    );
    if parts.is_empty() {
        t.file_name().map_or_else(
            || t.to_string_lossy().into_owned(),
            |n| n.to_string_lossy().into_owned(),
        )
    } else {
        parts.join("/")
    }
}

/// Spec-shaped key contexts for Swagger 2.0 documents: the `swagger: 2.0`
/// root key position offers the root vocabulary; deeper positions resolve
/// by pointer shape (definitions, parameters, operations).
fn swagger_context(low: &suspect_low::LowDoc, tokens: &[String]) -> CompletionContext {
    let _ = low;
    // Root mapping (empty path): the top-level vocabulary.
    if tokens.is_empty() {
        return CompletionContext::Keys(SWAGGER_ROOT_KEYS);
    }
    if tokens == ["swagger"] {
        return CompletionContext::Keys(SWAGGER_ROOT_KEYS);
    }
    if tokens.len() == 2 && matches!(tokens[1].as_str(), "info" | "externalDocs") {
        return CompletionContext::None;
    }
    if tokens.len() == 3 && tokens[0] == "swagger" && tokens[1] == "paths" {
        return CompletionContext::Keys(PATH_ITEM_KEYS);
    }
    if tokens.len() == 4
        && tokens[0] == "swagger"
        && tokens[1] == "paths"
        && METHODS.contains(&tokens[3].as_str())
    {
        return CompletionContext::Keys(SWAGGER_OPERATION_KEYS);
    }
    if tokens.len() == 3 && tokens[0] == "swagger" {
        return match tokens[1].as_str() {
            "definitions" => CompletionContext::Keys(SWAGGER_SCHEMA_KEYS),
            "parameters" => CompletionContext::Keys(SWAGGER_PARAM_KEYS),
            "responses" => CompletionContext::Keys(SWAGGER_SCHEMA_KEYS),
            _ => CompletionContext::None,
        };
    }
    // Nested parameter objects (operation-level parameters items).
    for window in tokens.windows(2) {
        if window[1] == "parameters" {
            return CompletionContext::Keys(SWAGGER_PARAM_KEYS);
        }
    }
    for token in tokens.iter().rev() {
        if METHODS.contains(&token.as_str()) {
            return CompletionContext::Keys(SWAGGER_OPERATION_KEYS);
        }
    }
    CompletionContext::None
}

/// Builds completion items for a key list.
///
/// Items carry a `data` marker so `completionItem/resolve` can attach
/// keyword documentation lazily without bloating the initial response.
#[must_use]
pub fn key_items(keys: &'static [&'static str]) -> Vec<CompletionItem> {
    keys.iter()
        .map(|k| CompletionItem {
            label: (*k).to_owned(),
            kind: Some(CompletionItemKind::PROPERTY),
            data: Some(serde_json::json!({ "suspect": "key", "key": k })),
            ..CompletionItem::default()
        })
        .collect()
}

/// Builds completion items for a fixed scalar vocabulary (schema types,
/// parameter locations, media types, …).
#[must_use]
pub fn value_items(values: &'static [&'static str]) -> Vec<CompletionItem> {
    values
        .iter()
        .map(|v| CompletionItem {
            label: (*v).to_owned(),
            kind: Some(CompletionItemKind::ENUM_MEMBER),
            data: Some(serde_json::json!({ "suspect": "value", "value": v })),
            ..CompletionItem::default()
        })
        .collect()
}

/// Builds completion items for entries of `components/<section>`: each item
/// resolves (via the existing `$ref` resolve path) to a target preview.
#[must_use]
pub fn component_name_items(names: Vec<String>, section: &str, home: &Uri) -> Vec<CompletionItem> {
    names
        .into_iter()
        .map(|n| {
            let raw = format!("#/components/{section}/{n}");
            CompletionItem {
                label: n,
                detail: Some(format!("#/components/{section}")),
                kind: Some(CompletionItemKind::MODULE),
                data: Some(serde_json::json!({
                    "suspect": "ref",
                    "uri": home.as_str(),
                    "raw": raw,
                })),
                ..CompletionItem::default()
            }
        })
        .collect()
}

/// Builds completion items for declared operationIds; the detail line shows
/// `METHOD /path` so the picker disambiguates identical ids.
#[must_use]
pub fn operation_id_items(candidates: Vec<(String, String)>) -> Vec<CompletionItem> {
    candidates
        .into_iter()
        .map(|(id, detail)| CompletionItem {
            label: id,
            detail: Some(detail),
            kind: Some(CompletionItemKind::FUNCTION),
            ..CompletionItem::default()
        })
        .collect()
}

/// Builds completion items for root-declared tag names.
#[must_use]
pub fn tag_name_items(names: Vec<String>) -> Vec<CompletionItem> {
    names
        .into_iter()
        .map(|n| CompletionItem {
            label: n,
            kind: Some(CompletionItemKind::VARIABLE),
            ..CompletionItem::default()
        })
        .collect()
}

/// Builds completion items for a schema's property names (required-array
/// items).
#[must_use]
pub fn property_name_items(names: Vec<String>) -> Vec<CompletionItem> {
    names
        .into_iter()
        .map(|n| CompletionItem {
            label: n,
            kind: Some(CompletionItemKind::PROPERTY),
            ..CompletionItem::default()
        })
        .collect()
}

/// Names declared under `components/<section>` in `low`.
#[must_use]
pub fn component_names(low: &suspect_low::LowDoc, section: &str) -> Vec<String> {
    let mut out: Vec<String> = low
        .root()
        .get("components")
        .and_then(|c| c.get(section))
        .map(|sec| sec.entries().iter().map(|e| e.key.to_owned()).collect())
        .unwrap_or_default();
    out.sort();
    out.dedup();
    out
}

/// Declared operationIds with their `METHOD /path` detail lines, in
/// document order.
#[must_use]
pub fn operation_id_candidates(low: &suspect_low::LowDoc) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    let Some(paths) = low.root().get("paths") else {
        return out;
    };
    for path_entry in paths.entries() {
        let Some(item) = path_entry.value else {
            continue;
        };
        for method in METHODS {
            let Some(op) = item.get(method) else {
                continue;
            };
            let Some(id) = op.get("operationId").and_then(|n| n.as_str()) else {
                continue;
            };
            out.push((
                id.to_owned(),
                format!("{} {}", method.to_ascii_uppercase(), path_entry.key),
            ));
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// Names declared in the root `tags` list, in document order.
#[must_use]
pub fn tag_name_candidates(low: &suspect_low::LowDoc) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let Some(tags) = low.root().get("tags") else {
        return out;
    };
    for item in tags.items() {
        if let Some(name) = item.get("name").and_then(|n| n.as_str()) {
            out.push(name.to_owned());
        }
    }
    out.sort();
    out.dedup();
    out
}

/// Builds completion items for `$ref` pointer candidates.
///
/// Each item stores its owning document plus the raw candidate string in
/// `data`; `completionItem/resolve` turns that into a resolved-target preview.
#[must_use]
pub fn ref_items(candidates: Vec<String>, home: &Uri) -> Vec<CompletionItem> {
    candidates
        .into_iter()
        .map(|c| CompletionItem {
            detail: Some("component reference".to_owned()),
            label: c.clone(),
            kind: Some(CompletionItemKind::MODULE),
            data: Some(serde_json::json!({
                "suspect": "ref",
                "uri": home.as_str(),
                "raw": c,
            })),
            ..CompletionItem::default()
        })
        .collect()
}

/// Fills in `documentation`/`detail` for one completion item.
///
/// `$ref` candidates resolve through the workspace and gain a fenced-code
/// excerpt of the target plus a `→ Name (file)` detail line; unknown or
/// unresolvable items come back unchanged (clients tolerate this).
#[must_use]
pub fn resolve_item(item: CompletionItem, ws: Option<&Workspace>) -> CompletionItem {
    let Some(data) = item.data.clone() else {
        return item;
    };
    let Some(kind) = data.get("suspect").and_then(|s| s.as_str()) else {
        return item;
    };
    match kind {
        // `$ref`-shaped items resolve to a target preview through the
        // workspace.
        "ref" => {
            let (Some(uri), Some(raw)) = (
                data.get("uri").and_then(|u| u.as_str()),
                data.get("raw").and_then(|r| r.as_str()),
            ) else {
                return item;
            };
            resolve_ref_documentation(item, uri, raw, ws)
        }
        // Fixed-vocabulary items carry static documentation.
        "value" => {
            let mut resolved = item;
            if let Some(doc) = data
                .get("value")
                .and_then(|v| v.as_str())
                .and_then(value_doc)
            {
                resolved.documentation = Some(tower_lsp::lsp_types::Documentation::MarkupContent(
                    tower_lsp::lsp_types::MarkupContent {
                        kind: tower_lsp::lsp_types::MarkupKind::Markdown,
                        value: doc.to_owned(),
                    },
                ));
            }
            resolved
        }
        _ => item,
    }
}

/// Static documentation for fixed scalar vocabularies.
fn value_doc(value: &str) -> Option<&'static str> {
    Some(match value {
        "string" => "JSON string value.",
        "number" => "JSON number (any numeric value).",
        "integer" => "JSON number without a fraction part.",
        "boolean" => "`true` or `false`.",
        "object" => "Mapping of string keys to values.",
        "array" => "Ordered list of values.",
        "null" => "JSON `null`.",
        "query" => "Parameter is carried in the URL query string.",
        "path" => "Parameter is part of the path template (always required).",
        "header" => "Parameter is carried in a request or response header.",
        "cookie" => "Parameter is carried in a request cookie.",
        "form" => "Query/cookie style: `a=1&b=2`.",
        "simple" => "Path/header style: comma-separated items.",
        "spaceDelimited" => "Space-separated array style.",
        "pipeDelimited" => "Pipe-separated array style.",
        "deepObject" => "Nested object style: `param[key]=value`.",
        "apiKey" => "Named key sent in a header, query parameter, or cookie.",
        "http" => "Standard HTTP authentication (Basic, Bearer, …).",
        "oauth2" => "OAuth 2.0 flows.",
        "openIdConnect" => "OpenID Connect discovery URL.",
        _ => return None,
    })
}

/// Attaches the resolved-target documentation for a `$ref`-shaped item.
fn resolve_ref_documentation(
    item: CompletionItem,
    uri: &str,
    raw: &str,
    ws: Option<&Workspace>,
) -> CompletionItem {
    let Some(ws) = ws else {
        return item;
    };
    let Ok(home) = Uri::parse(uri) else {
        return item;
    };
    let Some(handle) = crate::links::resolve_ref_string(ws, &home, raw) else {
        return item;
    };
    let mut resolved = item;
    resolved.documentation = Some(tower_lsp::lsp_types::Documentation::MarkupContent(
        tower_lsp::lsp_types::MarkupContent {
            kind: tower_lsp::lsp_types::MarkupKind::Markdown,
            value: handle.markdown_excerpt,
        },
    ));
    resolved.detail = Some(handle.detail);
    resolved
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use suspect_ref::{Workspace, WorkspaceBuilder};

    fn low_of(text: &str) -> suspect_low::LowDoc {
        let uri = Uri::parse("file:///mem/doc.yaml").unwrap();
        suspect_low::LowDoc::parse(
            uri,
            suspect_source::Source::from_vec(text.as_bytes().to_vec()),
        )
    }

    #[test]
    fn operation_key_context() {
        let text = "openapi: 3.1.0\npaths:\n  /pets:\n    get:\n      summary: x\n";
        let low = low_of(text);
        let off = text.find("summary").unwrap() + 2;
        assert_eq!(
            context_at(&low, off),
            CompletionContext::Keys(OPERATION_KEYS)
        );
    }

    #[test]
    fn schema_key_context_via_properties_parent() {
        let text = "components:\n  schemas:\n    Pet:\n      properties:\n        name:\n          type: string\n";
        let low = low_of(text);
        let off = text.find("type").unwrap() + 1;
        assert_eq!(context_at(&low, off), CompletionContext::Keys(SCHEMA_KEYS));
    }

    #[test]
    fn schema_key_context_under_components_schemas() {
        let text = "components:\n  schemas:\n    Pet:\n      required: true\n";
        let low = low_of(text);
        let off = text.find("required").unwrap() + 1;
        assert_eq!(context_at(&low, off), CompletionContext::Keys(SCHEMA_KEYS));
    }

    #[test]
    fn ref_value_context() {
        let text = "components:\n  schemas:\n    A:\n      $ref: '#/x'\n";
        let low = low_of(text);
        let off = text.find("#/x").unwrap();
        assert_eq!(context_at(&low, off), CompletionContext::Refs);
    }

    #[test]
    fn no_context_outside_known_shapes() {
        let text = "info:\n  title: T\n";
        let low = low_of(text);
        let off = text.find("title").unwrap() + 1;
        assert_eq!(context_at(&low, off), CompletionContext::None);
    }

    #[test]
    fn inline_schema_under_operation_is_schema_context() {
        let text = "openapi: 3.1.0\npaths:\n  /pets:\n    get:\n      responses:\n        '200':\n          content:\n            application/json:\n              schema:\n                type: object\n";
        let low = low_of(text);
        let off = text.find("type").unwrap() + 1;
        assert_eq!(context_at(&low, off), CompletionContext::Keys(SCHEMA_KEYS));
    }

    #[test]
    fn media_type_schema_under_request_bodies_is_schema_context() {
        let text = "components:\n  requestBodies:\n    Pet:\n      content:\n        application/json:\n          schema:\n            required: true\n";
        let low = low_of(text);
        let off = text.find("required").unwrap() + 1;
        assert_eq!(context_at(&low, off), CompletionContext::Keys(SCHEMA_KEYS));
    }

    #[test]
    fn path_item_level_offers_methods_and_path_item_keys() {
        let text = "openapi: 3.1.0\npaths:\n  /pets:\n    summary: x\n";
        let low = low_of(text);
        let off = text.find("summary").unwrap() + 1;
        assert_eq!(
            context_at(&low, off),
            CompletionContext::Keys(PATH_ITEM_KEYS)
        );
        assert!(PATH_ITEM_KEYS.contains(&"get"));
        assert!(PATH_ITEM_KEYS.contains(&"parameters"));
    }

    #[test]
    fn webhook_level_offers_path_item_keys() {
        let text = "openapi: 3.1.0\nwebhooks:\n  newPet:\n    description: x\n";
        let low = low_of(text);
        let off = text.find("description").unwrap() + 1;
        assert_eq!(
            context_at(&low, off),
            CompletionContext::Keys(PATH_ITEM_KEYS)
        );
    }

    fn workspace(dir: &std::path::Path) -> Arc<Workspace> {
        std::fs::write(
            dir.join("main.yaml"),
            "components:
  responses:
    Err:
      description: e
",
        )
        .unwrap();
        std::fs::write(
            dir.join("schemas.yaml"),
            "components:
  schemas:
    Pet:
      type: object
    PetList:
      type: array
",
        )
        .unwrap();
        let ws = WorkspaceBuilder::new().root(dir).build().unwrap();
        ws.load_all("main.yaml").unwrap();
        ws.load_all("schemas.yaml").unwrap();
        Arc::new(ws)
    }

    #[test]
    fn ref_candidates_same_and_cross_file() {
        let dir = std::env::temp_dir().join("suspect-lsp-completion");
        std::fs::create_dir_all(&dir).unwrap();
        let ws = workspace(&dir);
        let main_uri = Uri::from_path(&dir.join("main.yaml")).unwrap();
        let cands = ref_candidates(&ws, &main_uri);
        assert!(
            cands.contains(&"#/components/responses/Err".to_owned()),
            "{cands:?}"
        );
        assert!(cands.contains(&"schemas.yaml#/components/schemas/Pet".to_owned()));
        assert!(cands.contains(&"schemas.yaml#/components/schemas/PetList".to_owned()));
        // From the other document, same-file candidates are fragment-only.
        let schemas_uri = Uri::from_path(&dir.join("schemas.yaml")).unwrap();
        let cands2 = ref_candidates(&ws, &schemas_uri);
        assert!(
            cands2.contains(&"#/components/schemas/Pet".to_owned()),
            "{cands2:?}"
        );
        assert!(cands2.contains(&"main.yaml#/components/responses/Err".to_owned()));
    }

    #[test]
    fn resolve_item_attaches_target_excerpt_and_detail() {
        let dir = std::env::temp_dir().join("suspect-lsp-completion-resolve");
        std::fs::create_dir_all(&dir).unwrap();
        let ws = workspace(&dir);
        let main_uri = Uri::from_path(&dir.join("main.yaml")).unwrap();
        let items = ref_items(vec!["#/components/responses/Err".to_owned()], &main_uri);
        let resolved = resolve_item(items.into_iter().next().unwrap(), Some(&ws));
        let doc = match resolved.documentation {
            Some(tower_lsp::lsp_types::Documentation::MarkupContent(m)) => m.value,
            other => panic!("expected markdown documentation, got {other:?}"),
        };
        assert!(doc.contains("```yaml"), "{doc}");
        assert!(
            doc.contains("description"),
            "excerpt shows the target body: {doc}"
        );
        assert_eq!(
            resolved.detail.as_deref(),
            Some("→ Err (main.yaml)"),
            "detail names the target and its file"
        );
    }

    #[test]
    fn resolve_item_passthrough_without_data_or_workspace() {
        let item = CompletionItem {
            label: "type".to_owned(),
            ..CompletionItem::default()
        };
        let out = resolve_item(item.clone(), None);
        assert_eq!(out.label, "type");
        assert!(out.documentation.is_none());
    }

    #[test]
    fn item_kinds_match_context() {
        let keys = key_items(OPERATION_KEYS);
        assert!(
            keys.iter()
                .all(|i| i.kind == Some(CompletionItemKind::PROPERTY))
        );
        let refs = ref_items(
            vec!["#/components/schemas/Pet".to_owned()],
            &Uri::parse("file:///w/main.yaml").unwrap(),
        );
        assert_eq!(refs[0].kind, Some(CompletionItemKind::MODULE));
    }

    // -- value and name completion ------------------------------------------

    const API_DOC: &str = "\
openapi: 3.1.0
info:
  title: T
tags:
  - name: alpha
paths:
  /pets:
    get:
      tags:
        - x
      parameters:
        - name: q
          in:
      responses:
        '200':
          description: ok
      security:
        - ApiKey: []
components:
  securitySchemes:
    ApiKeyAuth:
      type: apiKey
  schemas:
    Pet:
      type: object
      properties:
        name:
          type:
      required:
        - x
      discriminator:
        mapping:
          dog:
";

    #[test]
    fn schema_type_values_complete_after_type_key() {
        // `type:` with the cursor after the colon inside a schema.
        let text = "\
components:
  schemas:
    Pet:
      type: o
";
        let low = low_of(text);
        let off = text.find("type: o").unwrap() + 7;
        assert_eq!(
            context_at(&low, off),
            CompletionContext::Values(SCHEMA_TYPES)
        );
    }

    #[test]
    fn scheme_type_values_complete_under_security_schemes() {
        let text = "\
components:
  securitySchemes:
    ApiKeyAuth:
      type: apiKey
";
        let low = low_of(text);
        let off = text.find("type: apiKey").unwrap() + 6;
        assert_eq!(
            context_at(&low, off),
            CompletionContext::Values(SCHEME_TYPES)
        );
    }

    #[test]
    fn parameter_in_values_complete_under_parameters() {
        let low = low_of(API_DOC);
        let off = API_DOC.find("in:\n").unwrap() + 4;
        assert_eq!(context_at(&low, off), CompletionContext::Values(PARAM_IN));
        // Style values too.
        let text = "\
paths:
  /p:
    get:
      parameters:
        - name: q
          in: query
          style: f
";
        let low = low_of(text);
        let off = text.find("style: f").unwrap() + 8;
        assert_eq!(
            context_at(&low, off),
            CompletionContext::Values(PARAM_STYLE)
        );
    }

    #[test]
    fn format_values_complete_in_schema_context() {
        let text = "\
components:
  schemas:
    Pet:
      properties:
        born:
          format: date-t
";
        let low = low_of(text);
        let off = text.find("format: date-t").unwrap() + 14;
        assert_eq!(
            context_at(&low, off),
            CompletionContext::Values(SCHEMA_FORMATS)
        );
    }

    #[test]
    fn media_type_keys_complete_under_content() {
        let text = "\
paths:
  /p:
    get:
      responses:
        '200':
          content:
            application/json:
              schema:
                type: string
";
        let low = low_of(text);
        let off = text.find("application/json").unwrap() + 2;
        assert_eq!(context_at(&low, off), CompletionContext::MediaTypes);
    }

    #[test]
    fn security_requirement_keys_complete_scheme_names() {
        let low = low_of(API_DOC);
        let off = API_DOC.find("ApiKey:").unwrap() + 2;
        assert_eq!(
            context_at(&low, off),
            CompletionContext::ComponentNames("securitySchemes")
        );
        assert_eq!(
            component_names(&low, "securitySchemes"),
            vec!["ApiKeyAuth".to_owned()]
        );
    }

    #[test]
    fn discriminator_mapping_values_complete_schema_names() {
        let low = low_of(API_DOC);
        let off = API_DOC.find("dog:").unwrap() + 5;
        assert_eq!(
            context_at(&low, off),
            CompletionContext::ComponentNames("schemas")
        );
        assert_eq!(component_names(&low, "schemas"), vec!["Pet".to_owned()]);
    }

    #[test]
    fn tags_items_complete_root_declared_names() {
        let low = low_of(API_DOC);
        let off = API_DOC.find("- x").unwrap() + 3;
        assert_eq!(context_at(&low, off), CompletionContext::TagNames);
        assert_eq!(tag_name_candidates(&low), vec!["alpha".to_owned()]);
        // The root tags list itself is NOT a TagNames context (items there
        // are objects with `name` keys).
        let root_off = API_DOC.find("- name: alpha").unwrap() + 4;
        assert_ne!(context_at(&low, root_off), CompletionContext::TagNames);
    }

    #[test]
    fn required_items_complete_declared_properties() {
        let text = "\
components:
  schemas:
    Pet:
      type: object
      properties:
        name:
          type: string
        age:
          type: integer
      required:
        - ag
";
        let low = low_of(text);
        let off = text.find("- ag").unwrap() + 5;
        assert_eq!(
            context_at(&low, off),
            CompletionContext::SchemaPropertyNames(vec!["name".to_owned(), "age".to_owned()])
        );
        let items = property_name_items(vec!["name".to_owned()]);
        assert_eq!(items[0].kind, Some(CompletionItemKind::PROPERTY));
    }

    #[test]
    fn links_operation_id_values_complete_declared_ids() {
        let text = "\
openapi: 3.1.0
info:
  title: T
paths:
  /pets:
    get:
      operationId: listPets
      responses:
        '200':
          description: ok
          links:
            address:
              operationId: getP
";
        let low = low_of(text);
        let off = text.find("operationId: getP").unwrap() + 16;
        assert_eq!(context_at(&low, off), CompletionContext::OperationIds);
        let candidates = operation_id_candidates(&low);
        assert_eq!(
            candidates,
            vec![("listPets".to_owned(), "GET /pets".to_owned())]
        );
        let items = operation_id_items(candidates);
        assert_eq!(items[0].label, "listPets");
        assert_eq!(items[0].detail.as_deref(), Some("GET /pets"));
    }

    #[test]
    fn component_name_items_resolve_to_target_previews() {
        let names = component_name_items(
            vec!["Pet".to_owned()],
            "schemas",
            &Uri::parse("file:///w/main.yaml").unwrap(),
        );
        assert_eq!(names[0].label, "Pet");
        assert_eq!(names[0].detail.as_deref(), Some("#/components/schemas"));
        assert_eq!(names[0].kind, Some(CompletionItemKind::MODULE));
    }

    #[test]
    fn value_items_resolve_to_static_documentation() {
        let items = value_items(SCHEMA_TYPES);
        let resolved = resolve_item(items[0].clone(), None);
        let Some(tower_lsp::lsp_types::Documentation::MarkupContent(markup)) =
            resolved.documentation
        else {
            panic!("string type has documentation");
        };
        assert_eq!(markup.value, "JSON string value.");
        // Unknown values resolve without documentation.
        let bare = value_items(PARAM_IN);
        let _ = &bare;
        let mut no_doc = items[1].clone();
        no_doc.data = Some(serde_json::json!({"suspect": "value", "value": "mystery"}));
        assert!(resolve_item(no_doc, None).documentation.is_none());
    }

    #[test]
    fn ref_items_still_resolve_through_the_workspace() {
        // The ref resolve path keeps working after the resolve rewrite.
        let names = component_name_items(
            vec!["Pet".to_owned()],
            "schemas",
            &Uri::parse("file:///mem/doc.yaml").unwrap(),
        );
        // No workspace: the item comes back unchanged.
        let resolved = resolve_item(names[0].clone(), None);
        assert_eq!(resolved, names[0]);
    }
}
