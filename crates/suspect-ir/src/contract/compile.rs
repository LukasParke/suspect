use std::collections::{BTreeMap, HashSet};
use std::sync::Arc;

use suspect_low::{LowDoc, Pointer, ValueKind};
use suspect_oas::{OasVersion, Session};
use suspect_ref::Workspace;
use suspect_source::Uri;

use super::{
    Contract, ContractDiagnostic, ContractError, ContractReader, ContractSeverity, Document,
    SchemaDialect, SchemaId,
};

pub(super) fn compile(
    workspace: &Arc<Workspace>,
    entry: &Uri,
    reader: ContractReader,
) -> Result<Contract, ContractError> {
    // Provider aliases select one canonical retrieval identity. That identity,
    // rather than a requested redirect alias or cache filename, is the base.
    let canonical_entry = workspace
        .open(entry.as_str())
        .map_err(|error| ContractError(error.to_string()))?
        .uri()
        .clone();
    let entry = &canonical_entry;
    let trace = std::env::var_os("SUSPECT_CONTRACT_TRACE").is_some();
    let mut phase_start = std::time::Instant::now();
    let mut checkpoint = |name: &str| {
        let now = std::time::Instant::now();
        if trace {
            eprintln!("[contract] {name}: {:?}", now.duration_since(phase_start));
        }
        phase_start = now;
    };
    let session = Session::new(Arc::clone(workspace));
    let api = session
        .open(entry.as_str())
        .map_err(|e| ContractError(e.to_string()))?;
    checkpoint("entry parse/open");
    let mut out = snapshot(entry.clone(), String::new());
    out.add_document(workspace, entry, reader)?;
    checkpoint("source materialization and span index");
    out.openapi_version = out.documents[entry].raw["openapi"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    let default_dialect = match api.version() {
        OasVersion::V30 => SchemaDialect::OpenApi30,
        _ => SchemaDialect::Uri(
            out.documents[entry].raw["jsonSchemaDialect"]
                .as_str()
                .unwrap_or("https://spec.openapis.org/oas/3.1/dialect/base")
                .to_owned(),
        ),
    };

    super::walk::index(&mut out, workspace, reader, default_dialect)?;
    checkpoint("structural schema/reference graph");
    // The structural registry retains lexical dialects even when a reference
    // selects a nested schema without adding its containing schema to the graph.
    for id in out.schemas.keys().cloned().collect::<Vec<_>>() {
        let dialect = out.schemas[&id].dialect.clone();
        out.schema_diagnostics(&id, &dialect);
    }
    checkpoint("dialect and diagnostics");
    let (http, diagnostics) = super::http_compile::index(&out);
    out.http = http;
    out.diagnostics.extend(diagnostics);
    checkpoint("HTTP metadata");
    out.roots.sort();
    out.roots.dedup();
    out.diagnostics
        .sort_by(|a, b| (&a.source, a.at.start, a.code).cmp(&(&b.source, b.at.start, b.code)));
    out.diagnostics
        .dedup_by(|a, b| a.source == b.source && a.at == b.at && a.code == b.code);
    Ok(out)
}

/// Empty owned tables, also used for registration-only resource catalogs. A
/// catalog never follows references and is not returned as a compiled Contract.
pub(super) fn snapshot(entry: Uri, openapi_version: String) -> Contract {
    Contract {
        entry,
        openapi_version,
        documents: BTreeMap::new(),
        schemas: BTreeMap::new(),
        roots: Vec::new(),
        diagnostics: Vec::new(),
        reference_targets: BTreeMap::new(),
        source_versions: BTreeMap::new(),
        schema_contexts: BTreeMap::new(),
        resources: super::resources::ResourceIndex::default(),
        http: super::http::HttpIndex::default(),
    }
}

impl Contract {
    pub(super) fn add_document(
        &mut self,
        workspace: &Workspace,
        uri: &Uri,
        reader: ContractReader,
    ) -> Result<(), ContractError> {
        let handle = workspace
            .get(uri)
            .ok_or_else(|| ContractError(format!("document is not loaded: {uri}")))?;
        let uri = handle.uri();
        if self.documents.contains_key(uri) {
            return Ok(());
        }
        self.documents
            .insert(uri.clone(), document(handle.doc(), reader)?);
        Ok(())
    }

    fn schema_diagnostics(&mut self, id: &SchemaId, dialect: &SchemaDialect) {
        let raw = self.documents[&id.document]
            .raw
            .pointer(&id.pointer)
            .expect("indexed schema exists");
        let mut diagnostics = Vec::new();
        let mut report = |code, severity, keyword: Option<&str>, message: String| {
            let at = keyword
                .and_then(|key| self.source_span(&id.child(key)))
                .or_else(|| self.source_span(id))
                .unwrap_or_default();
            diagnostics.push(ContractDiagnostic {
                source: id.clone(),
                at,
                code,
                severity,
                message,
            });
        };
        if let SchemaDialect::Uri(uri) = dialect
            && !matches!(
                uri.strip_suffix('#').unwrap_or(uri),
                "https://spec.openapis.org/oas/3.1/dialect/base"
                    | "https://json-schema.org/draft/2020-12/schema"
            )
        {
            report(
                "unsupported-schema-dialect",
                ContractSeverity::Error,
                Some("$schema"),
                format!("schema dialect `{uri}` is retained but its vocabulary is not implemented"),
            );
        }
        let additional_properties = Pointer::parse(&id.pointer)
            .ok()
            .filter(|pointer| {
                pointer
                    .tokens()
                    .last()
                    .is_some_and(|key| key.as_ref() == "additionalProperties")
            })
            .and_then(|pointer| pointer.parent())
            .is_some_and(|parent| {
                self.schemas
                    .contains_key(&SchemaId::new(id.document.clone(), parent))
            });
        let boolean_allowed = !matches!(dialect, SchemaDialect::OpenApi30) || additional_properties;
        if !(raw.is_object() || raw.is_boolean() && boolean_allowed) {
            report(
                "invalid-schema",
                ContractSeverity::Error,
                None,
                "schema must be an object, or a boolean in OpenAPI 3.1+".to_owned(),
            );
        }
        if let Some(keywords) = raw.as_object() {
            for key in keywords.keys() {
                match key.as_str() {
                    "$schema" if matches!(dialect, SchemaDialect::OpenApi30) => report("unsupported-schema-keyword", ContractSeverity::Error, Some(key), "OpenAPI 3.0 does not support `$schema` dialect overrides".to_owned()),
                    "$anchor" if matches!(dialect, SchemaDialect::OpenApi30) => report("unsupported-schema-keyword", ContractSeverity::Error, Some(key), "OpenAPI 3.0 does not support `$anchor` schema identifiers".to_owned()),
                    "$recursiveRef" | "$recursiveAnchor" => report("unsupported-dynamic-reference", ContractSeverity::Error, Some(key), format!("`{key}` requires legacy recursive-scope semantics that are not implemented")),
                    "$dynamicRef" | "$dynamicAnchor" if !super::resources::supports_dialect(dialect) => report("unsupported-dynamic-reference", ContractSeverity::Error, Some(key), format!("`{key}` requires a supported 2020-12 schema dialect")),
                    "$id" if !super::resources::supports_dialect(dialect) => report("unsupported-schema-resource", ContractSeverity::Error, Some(key), "`$id` resource semantics require a supported 2020-12 schema dialect".to_owned()),
                    "$vocabulary" => report("unsupported-schema-vocabulary", ContractSeverity::Error, Some(key), "`$vocabulary` is retained; custom vocabulary semantics are not implemented".to_owned()),
                    "definitions" | "dependencies" | "additionalItems" => report("unsupported-schema-keyword", ContractSeverity::Error, Some(key), format!("legacy keyword `{key}` is retained; its dialect-specific traversal is not implemented")),
                    _ if key.starts_with("x-") || known_keyword(key) => {},
                    _ => report("unknown-schema-keyword", ContractSeverity::Warning, Some(key), format!("unknown keyword `{key}` is retained as an uninterpreted annotation")),
                }
            }
        }
        self.diagnostics.extend(diagnostics);
    }
}

fn known_keyword(keyword: &str) -> bool {
    matches!(
        keyword,
        "$schema"
            | "$id"
            | "$dynamicRef"
            | "$dynamicAnchor"
            | "$ref"
            | "$anchor"
            | "$comment"
            | "$defs"
            | "type"
            | "enum"
            | "const"
            | "title"
            | "description"
            | "default"
            | "deprecated"
            | "readOnly"
            | "writeOnly"
            | "examples"
            | "example"
            | "format"
            | "multipleOf"
            | "maximum"
            | "exclusiveMaximum"
            | "minimum"
            | "exclusiveMinimum"
            | "maxLength"
            | "minLength"
            | "pattern"
            | "maxItems"
            | "minItems"
            | "uniqueItems"
            | "maxContains"
            | "minContains"
            | "maxProperties"
            | "minProperties"
            | "required"
            | "dependentRequired"
            | "properties"
            | "patternProperties"
            | "additionalProperties"
            | "propertyNames"
            | "dependentSchemas"
            | "unevaluatedProperties"
            | "unevaluatedItems"
            | "items"
            | "prefixItems"
            | "contains"
            | "allOf"
            | "anyOf"
            | "oneOf"
            | "not"
            | "if"
            | "then"
            | "else"
            | "contentEncoding"
            | "contentMediaType"
            | "contentSchema"
            | "nullable"
            | "discriminator"
            | "xml"
            | "externalDocs"
    )
}

pub(super) fn document(doc: &LowDoc, reader: ContractReader) -> Result<Document, ContractError> {
    let trace = std::env::var_os("SUSPECT_CONTRACT_TRACE").is_some();
    let start = std::time::Instant::now();
    if doc.inner().has_errors() {
        return Err(ContractError(format!("syntax errors in {}", doc.uri())));
    }
    let mut by_range = BTreeMap::new();
    let mut spans = BTreeMap::new();
    let mut on_path = HashSet::new();
    let mut pending = vec![(doc.root(), Pointer::root(), false)];
    while let Some((node, pointer, exit)) = pending.pop() {
        let node = node.resolved();
        let range = node.byte_range();
        let range_key = (range.start, range.end);
        if exit {
            on_path.remove(&range_key);
            continue;
        }
        if pointer.tokens().len() > 256 || !on_path.insert(range_key) {
            return Err(ContractError(format!(
                "cyclic YAML alias or excessive document depth at {}{}",
                doc.uri(),
                pointer
            )));
        }
        let path = pointer.to_path();
        if let Some(previous) = by_range.insert(range_key, path.clone())
            && previous != path
        {
            return Err(ContractError(format!(
                "ambiguous YAML alias expansion in {}: one source value appears at {previous} and {path}; occurrence-aware source identity is not implemented",
                doc.uri()
            )));
        }
        spans.insert(path, range);
        pending.push((node, pointer.clone(), true));
        match node.kind() {
            ValueKind::Object => {
                let mut keys = HashSet::new();
                for entry in node.entries().into_iter().rev() {
                    let bytes = entry
                        .key_node
                        .try_decoded_scalar()
                        .ok_or_else(|| ContractError(format!("invalid key in {}", doc.uri())))?;
                    let key =
                        std::str::from_utf8(&bytes).map_err(|e| ContractError(e.to_string()))?;
                    if !keys.insert(key.to_owned()) {
                        return Err(ContractError(format!(
                            "duplicate mapping key `{key}` at {}{}",
                            doc.uri(),
                            pointer
                        )));
                    }
                    if let Some(value) = entry.value {
                        pending.push((value, pointer.push(key), false));
                    } else {
                        // YAML `key:` has a semantic null value but no value
                        // token. OAS missing-slot views anchor at the key.
                        let range = entry.key_node.byte_range();
                        let path = pointer.push(key).to_path();
                        by_range.insert((range.start, range.end), path.clone());
                        spans.insert(path, range);
                    }
                }
            }
            ValueKind::Array => {
                for (i, value) in node.items().into_iter().enumerate().rev() {
                    pending.push((value, pointer.push(&i.to_string()), false));
                }
            }
            ValueKind::Float => {
                let raw = std::str::from_utf8(node.scalar_bytes())
                    .map_err(|e| ContractError(e.to_string()))?;
                if !crate::common::scalar_json(raw, false).is_number() {
                    return Err(ContractError(format!(
                        "non-JSON numeric value at {}{}",
                        doc.uri(),
                        pointer
                    )));
                }
            }
            _ => {}
        }
    }
    if trace {
        eprintln!(
            "[contract] document source index ({} values): {:?}",
            spans.len(),
            start.elapsed()
        );
    }
    let start = std::time::Instant::now();
    let raw = reader.document_value(doc.uri(), doc)?;
    if trace {
        eprintln!("[contract] document normalized JSON: {:?}", start.elapsed());
    }
    Ok(Document { raw, spans })
}
