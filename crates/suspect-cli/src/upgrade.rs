//! Swagger 2.0 → OpenAPI 3.1 upgrader.
//!
//! Translates a materialized Swagger 2.0 document into an OpenAPI 3.1
//! document: `host`/`basePath`/`schemes` fold into `servers`,
//! `definitions` moves to `components/schemas`, `securityDefinitions` to
//! `components/securitySchemes` (with OAuth flow renames), body/formData
//! parameters become request bodies, `response.schema` becomes response
//! `content`, and `consumes`/`produces` become per-operation content keys.
//!
//! The output is a plain JSON value — the caller (the `suspect upgrade` CLI
//! command) serializes it and verifies it with the OpenAPI 3.1 validation
//! battery.

#![deny(missing_docs)]

use serde_json::{Map, Value, json};

/// Upgrades a materialized Swagger 2.0 document to OpenAPI 3.1.
///
/// # Errors
/// Returns a human-readable error when the document is not a Swagger 2.0
/// object (`swagger: "2.0"` missing) or a required field (`info`,
/// `paths`) is absent.
pub fn upgrade(doc: &Value) -> Result<Value, String> {
    let obj = doc
        .as_object()
        .ok_or_else(|| "document root must be an object".to_owned())?;
    if obj.get("swagger").and_then(Value::as_str) != Some("2.0") {
        return Err("document is not Swagger 2.0 (missing `swagger: \"2.0\"`)".to_owned());
    }
    let info = obj
        .get("info")
        .ok_or("`info` is required".to_owned())?
        .clone();
    let paths = obj
        .get("paths")
        .ok_or("`paths` is required".to_owned())?
        .clone();

    let mut out = Map::new();
    out.insert("openapi".into(), json!("3.1.0"));
    out.insert("info".into(), info);
    out.insert(
        "jsonSchemaDialect".into(),
        json!("https://spec.openapis.org/oas/3.1/dialect/base"),
    );

    let ctx = Context::new(obj);
    let servers = ctx.servers(None);
    out.insert("servers".into(), Value::Array(servers));

    if let Value::Object(path_map) = &paths {
        let mut upgraded_paths = Map::new();
        for (path_key, path_item) in path_map {
            if path_key.starts_with("x-") {
                upgraded_paths.insert(path_key.clone(), path_item.clone());
            } else if let Some(upgraded) = ctx.path_item(path_item) {
                upgraded_paths.insert(path_key.clone(), upgraded);
            }
        }
        out.insert("paths".into(), Value::Object(upgraded_paths));
    }

    let components = ctx.components();
    if !components.is_empty() {
        out.insert("components".into(), Value::Object(components));
    }

    // Keys with identical 3.x shapes, plus vendor extensions.
    for (key, value) in obj {
        if matches!(key.as_str(), "security" | "tags" | "externalDocs") || key.starts_with("x-") {
            out.insert(key.clone(), value.clone());
        }
    }

    let mut upgraded = Value::Object(out);
    rewrite_refs(&mut upgraded, &ctx.body_parameters);
    Ok(upgraded)
}

/// Document-wide facts the per-operation conversion needs.
struct Context<'a> {
    root: &'a Map<String, Value>,
    consumes: Option<Vec<String>>,
    produces: Option<Vec<String>>,
    /// Root `parameters` entries, by registry name.
    parameters: Map<String, Value>,
    /// Names of root parameters that are `in: body` (they become
    /// `components/requestBodies`, so their refs are rewritten there).
    body_parameters: std::collections::BTreeSet<String>,
}

/// One converted 2.0 parameter.
enum Converted {
    /// A 3.x parameter object (or a `$ref` to one).
    Parameter(Value),
    /// A body parameter: its 3.x request body.
    Body(Value),
    /// A formData field: name, property schema, required, is a file.
    Form(String, Value, bool, bool),
}

impl<'a> Context<'a> {
    fn new(root: &'a Map<String, Value>) -> Self {
        let parameters = root
            .get("parameters")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        let body_parameters = parameters
            .iter()
            .filter(|(_, p)| p.get("in").and_then(Value::as_str) == Some("body"))
            .map(|(name, _)| name.clone())
            .collect();
        Self {
            root,
            consumes: media_list(root.get("consumes")),
            produces: media_list(root.get("produces")),
            parameters,
            body_parameters,
        }
    }

    /// `schemes` × `host` × `basePath` → servers. With no `host` the
    /// server is relative to the document's own location, per 2.0.
    fn servers(&self, schemes: Option<&[Value]>) -> Vec<Value> {
        let base_path = self
            .root
            .get("basePath")
            .and_then(Value::as_str)
            .unwrap_or("");
        let Some(host) = self.root.get("host").and_then(Value::as_str) else {
            let url = if base_path.is_empty() { "/" } else { base_path };
            return vec![json!({ "url": url })];
        };
        let declared: Vec<&str> = schemes
            .or_else(|| {
                self.root
                    .get("schemes")
                    .and_then(Value::as_array)
                    .map(Vec::as_slice)
            })
            .unwrap_or_default()
            .iter()
            .filter_map(Value::as_str)
            .collect();
        let schemes = if declared.is_empty() {
            vec!["https"]
        } else {
            declared
        };
        schemes
            .iter()
            .map(|scheme| json!({ "url": format!("{scheme}://{host}{base_path}") }))
            .collect()
    }

    fn path_item(&self, path_item: &Value) -> Option<Value> {
        let item = path_item.as_object()?;
        let mut out = Map::new();

        // Path-level parameters: ordinary ones stay on the path item (3.x
        // cascades them the same way); body/formData ones must move into
        // each operation's request body.
        let mut shared_params = Vec::new();
        let mut shared_payload = Vec::new();
        for param in item
            .get("parameters")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or_default()
        {
            match self.parameter(param, &["application/json".to_owned()]) {
                Some(Converted::Parameter(p)) => shared_params.push(p),
                Some(_) => shared_payload.push(param.clone()),
                None => {}
            }
        }
        if !shared_params.is_empty() {
            out.insert("parameters".into(), Value::Array(shared_params));
        }

        for method in METHODS {
            let Some(op) = item.get(method).and_then(Value::as_object) else {
                continue;
            };
            out.insert(method.into(), self.operation(op, &shared_payload));
        }

        // `$ref` path items, `summary`/`description` and extensions pass
        // through.
        for (key, value) in item {
            if !METHODS.contains(&key.as_str()) && key != "parameters" {
                out.insert(key.clone(), value.clone());
            }
        }
        Some(Value::Object(out))
    }

    fn operation(&self, op: &Map<String, Value>, shared_payload: &[Value]) -> Value {
        let consumes = media_list(op.get("consumes"))
            .or_else(|| self.consumes.clone())
            .unwrap_or_else(default_media);
        let produces = media_list(op.get("produces"))
            .or_else(|| self.produces.clone())
            .unwrap_or_else(default_media);

        let mut upgraded = Map::new();
        for (key, value) in op {
            if matches!(
                key.as_str(),
                "tags"
                    | "summary"
                    | "description"
                    | "externalDocs"
                    | "operationId"
                    | "deprecated"
                    | "security"
            ) || key.starts_with("x-")
            {
                upgraded.insert(key.clone(), value.clone());
            }
        }

        let own: Vec<Value> = op
            .get("parameters")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        // Operation-level payload parameters override path-level ones with
        // the same name and location.
        let overridden = |p: &Value| {
            own.iter()
                .any(|o| o.get("name") == p.get("name") && o.get("in") == p.get("in"))
        };
        let all: Vec<&Value> = shared_payload
            .iter()
            .filter(|p| !overridden(p))
            .chain(own.iter())
            .collect();

        let mut parameters = Vec::new();
        let mut request_body = None;
        let mut form_properties = Map::new();
        let mut form_required = Vec::new();
        let mut has_file = false;
        for param in all {
            match self.parameter(param, &consumes) {
                Some(Converted::Parameter(p)) => parameters.push(p),
                Some(Converted::Body(body)) => request_body = Some(body),
                Some(Converted::Form(name, schema, required, file)) => {
                    has_file |= file;
                    if required {
                        form_required.push(json!(name));
                    }
                    form_properties.insert(name, schema);
                }
                None => {}
            }
        }
        if request_body.is_none() && !form_properties.is_empty() {
            let media = form_media(&consumes, has_file);
            let mut schema = json!({"type": "object", "properties": form_properties});
            if !form_required.is_empty() {
                schema["required"] = Value::Array(form_required);
            }
            request_body = Some(json!({"content": {media: {"schema": schema}}}));
        }
        if let Some(body) = request_body {
            upgraded.insert("requestBody".into(), body);
        }
        if !parameters.is_empty() {
            upgraded.insert("parameters".into(), Value::Array(parameters));
        }

        if let Some(responses) = op.get("responses").and_then(Value::as_object) {
            let converted = responses
                .iter()
                .map(|(status, response)| {
                    let value = if status.starts_with("x-") {
                        response.clone()
                    } else {
                        upgrade_response(response, &produces)
                    };
                    (status.clone(), value)
                })
                .collect();
            upgraded.insert("responses".into(), Value::Object(converted));
        }

        // Operation-level `schemes` that differ from the root's become
        // per-operation servers.
        if let Some(op_schemes) = op.get("schemes").and_then(Value::as_array) {
            let root_schemes = self
                .root
                .get("schemes")
                .and_then(Value::as_array)
                .map(Vec::as_slice)
                .unwrap_or_default();
            if !same_scheme_set(op_schemes, root_schemes) {
                upgraded.insert(
                    "servers".into(),
                    Value::Array(self.servers(Some(op_schemes))),
                );
            }
        }
        Value::Object(upgraded)
    }

    /// Converts one 2.0 parameter (or `$ref` to a root parameter).
    fn parameter(&self, param: &Value, consumes: &[String]) -> Option<Converted> {
        let obj = param.as_object()?;
        if let Some(reference) = obj.get("$ref").and_then(Value::as_str) {
            let name = reference.strip_prefix("#/parameters/");
            return match name.and_then(|n| self.parameters.get(n).map(|p| (n, p))) {
                // Body refs point at the converted request body component.
                Some((name, target)) if is_in(target, "body") => Some(Converted::Body(
                    json!({ "$ref": format!("#/components/requestBodies/{name}") }),
                )),
                // formData has no 3.x component form; inline it.
                Some((_, target)) if is_in(target, "formData") => self.parameter(target, consumes),
                // Ordinary parameters (and external refs) stay refs; the
                // final pass rewrites `#/parameters/` fragments.
                _ => Some(Converted::Parameter(param.clone())),
            };
        }
        let location = obj.get("in").and_then(Value::as_str).unwrap_or("");
        let required = obj
            .get("required")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        match location {
            "body" => {
                let schema = upgrade_schema(obj.get("schema").unwrap_or(&json!({})));
                let content: Map<String, Value> = consumes
                    .iter()
                    .map(|media| (media.clone(), json!({ "schema": schema })))
                    .collect();
                let mut body = Map::new();
                if let Some(description) = obj.get("description") {
                    body.insert("description".into(), description.clone());
                }
                body.insert("content".into(), Value::Object(content));
                if required {
                    body.insert("required".into(), json!(true));
                }
                Some(Converted::Body(Value::Object(body)))
            }
            "formData" => {
                let name = obj.get("name").and_then(Value::as_str)?.to_owned();
                let is_file = obj.get("type").and_then(Value::as_str) == Some("file");
                let mut schema = upgrade_schema(&Value::Object(simple_schema(obj)));
                if let Some(description) = obj.get("description") {
                    schema["description"] = description.clone();
                }
                Some(Converted::Form(name, schema, required, is_file))
            }
            _ => {
                let mut out = Map::new();
                for (key, value) in obj {
                    if matches!(
                        key.as_str(),
                        "name"
                            | "in"
                            | "description"
                            | "required"
                            | "deprecated"
                            | "allowEmptyValue"
                    ) || key.starts_with("x-")
                    {
                        out.insert(key.clone(), value.clone());
                    }
                }
                out.insert(
                    "schema".into(),
                    upgrade_schema(&Value::Object(simple_schema(obj))),
                );
                if let Some((style, explode)) = collection_style(obj, location) {
                    out.insert("style".into(), json!(style));
                    out.insert("explode".into(), json!(explode));
                }
                Some(Converted::Parameter(Value::Object(out)))
            }
        }
    }

    fn components(&self) -> Map<String, Value> {
        let mut components = Map::new();
        if let Some(definitions) = self.root.get("definitions").and_then(Value::as_object) {
            let schemas = definitions
                .iter()
                .map(|(k, v)| (k.clone(), upgrade_schema(v)))
                .collect();
            components.insert("schemas".into(), Value::Object(schemas));
        }
        let mut parameters = Map::new();
        let mut request_bodies = Map::new();
        let default = self.consumes.clone().unwrap_or_else(default_media);
        for (name, param) in &self.parameters {
            match self.parameter(param, &default) {
                Some(Converted::Parameter(p)) => {
                    parameters.insert(name.clone(), p);
                }
                Some(Converted::Body(body)) => {
                    request_bodies.insert(name.clone(), body);
                }
                // formData registry entries are inlined where referenced.
                Some(Converted::Form(..)) | None => {}
            }
        }
        if !parameters.is_empty() {
            components.insert("parameters".into(), Value::Object(parameters));
        }
        if !request_bodies.is_empty() {
            components.insert("requestBodies".into(), Value::Object(request_bodies));
        }
        if let Some(responses) = self.root.get("responses").and_then(Value::as_object) {
            let produces = self.produces.clone().unwrap_or_else(default_media);
            let converted = responses
                .iter()
                .map(|(k, v)| (k.clone(), upgrade_response(v, &produces)))
                .collect();
            components.insert("responses".into(), Value::Object(converted));
        }
        if let Some(security) = self.root.get("securityDefinitions") {
            components.insert(
                "securitySchemes".into(),
                upgrade_security_definitions(security),
            );
        }
        components
    }
}

const METHODS: [&str; 7] = ["get", "put", "post", "delete", "options", "head", "patch"];

fn is_in(param: &Value, location: &str) -> bool {
    param.get("in").and_then(Value::as_str) == Some(location)
}

fn default_media() -> Vec<String> {
    vec!["application/json".to_owned()]
}

/// A 2.0 media-type list; `None` when absent or empty, so the caller can
/// fall back to the enclosing level (operation → root → JSON).
fn media_list(value: Option<&Value>) -> Option<Vec<String>> {
    let list: Vec<String> = value?
        .as_array()?
        .iter()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect();
    (!list.is_empty()).then_some(list)
}

/// The request media type for formData fields: a declared form type
/// wins; otherwise multipart when a file is present, urlencoded if not.
fn form_media(consumes: &[String], has_file: bool) -> String {
    let declared = consumes.iter().find(|m| {
        m.starts_with("multipart/form-data") || m.starts_with("application/x-www-form-urlencoded")
    });
    match declared {
        Some(media) if !(has_file && media.starts_with("application/x-www-form-urlencoded")) => {
            media.clone()
        }
        _ if has_file => "multipart/form-data".to_owned(),
        _ => "application/x-www-form-urlencoded".to_owned(),
    }
}

/// The schema keywords a 2.0 non-body parameter or header carries
/// inline.
fn simple_schema(obj: &Map<String, Value>) -> Map<String, Value> {
    obj.iter()
        .filter(|(key, _)| {
            matches!(
                key.as_str(),
                "type"
                    | "format"
                    | "items"
                    | "enum"
                    | "default"
                    | "minimum"
                    | "exclusiveMinimum"
                    | "maximum"
                    | "exclusiveMaximum"
                    | "multipleOf"
                    | "minLength"
                    | "maxLength"
                    | "pattern"
                    | "minItems"
                    | "maxItems"
                    | "uniqueItems"
                    | "x-nullable"
            )
        })
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect()
}

/// 2.0 `collectionFormat` → 3.x `style`/`explode`. `None` for non-arrays
/// and the 3.x defaults that need no keywords.
fn collection_style(obj: &Map<String, Value>, location: &str) -> Option<(&'static str, bool)> {
    if obj.get("type").and_then(Value::as_str) != Some("array") {
        return None;
    }
    let format = obj
        .get("collectionFormat")
        .and_then(Value::as_str)
        .unwrap_or("csv");
    match (format, location) {
        ("multi", "query") => Some(("form", true)),
        ("csv", "query" | "cookie") => Some(("form", false)),
        ("csv", "path" | "header") => Some(("simple", false)),
        ("ssv", "query") => Some(("spaceDelimited", false)),
        ("pipes", "query") => Some(("pipeDelimited", false)),
        // `tsv` and other combinations have no 3.x serialization; leave
        // the defaults rather than claim a style that would misencode.
        _ => None,
    }
}

/// Upgrades a 2.0 response: `schema` and `examples` → per-media
/// `content`, header objects → header schemas.
fn upgrade_response(response: &Value, produces: &[String]) -> Value {
    let Some(obj) = response.as_object() else {
        return response.clone();
    };
    if obj.contains_key("$ref") {
        return response.clone();
    }
    let mut out = Map::new();
    out.insert(
        "description".into(),
        obj.get("description").cloned().unwrap_or_else(|| json!("")),
    );
    if let Some(headers) = obj.get("headers").and_then(Value::as_object) {
        let converted = headers
            .iter()
            .map(|(name, header)| (name.clone(), upgrade_header(header)))
            .collect();
        out.insert("headers".into(), Value::Object(converted));
    }
    let examples = obj.get("examples").and_then(Value::as_object);
    if let Some(schema) = obj.get("schema") {
        let schema = upgrade_schema(schema);
        let content: Map<String, Value> = produces
            .iter()
            .map(|media| {
                let mut entry = json!({ "schema": schema });
                // 2.0 `examples` is keyed by media type; each value is that
                // media type's single example.
                if let Some(example) = examples.and_then(|e| e.get(media)) {
                    entry["example"] = example.clone();
                }
                (media.clone(), entry)
            })
            .collect();
        out.insert("content".into(), Value::Object(content));
    }
    for (key, value) in obj {
        if key.starts_with("x-") {
            out.insert(key.clone(), value.clone());
        }
    }
    Value::Object(out)
}

/// A 2.0 header object (inline schema keywords) → a 3.x header.
fn upgrade_header(header: &Value) -> Value {
    let Some(obj) = header.as_object() else {
        return header.clone();
    };
    let mut out = Map::new();
    if let Some(description) = obj.get("description") {
        out.insert("description".into(), description.clone());
    }
    out.insert(
        "schema".into(),
        upgrade_schema(&Value::Object(simple_schema(obj))),
    );
    if let Some((style, explode)) = collection_style(obj, "header") {
        out.insert("style".into(), json!(style));
        out.insert("explode".into(), json!(explode));
    }
    Value::Object(out)
}

/// Rewrites 2.0 registry ref fragments across the tree:
/// `#/definitions/` → `#/components/schemas/`, `#/parameters/` →
/// `#/components/parameters/` (or `requestBodies` for body parameters),
/// `#/responses/` → `#/components/responses/`.
fn rewrite_refs(value: &mut Value, body_parameters: &std::collections::BTreeSet<String>) {
    match value {
        Value::Object(map) => {
            for (key, child) in map.iter_mut() {
                if key == "$ref"
                    && let Some(text) = child.as_str()
                    && let Some(rewritten) = rewrite_ref_text(text, body_parameters)
                {
                    *child = Value::String(rewritten);
                    continue;
                }
                rewrite_refs(child, body_parameters);
            }
        }
        Value::Array(items) => {
            for item in items {
                rewrite_refs(item, body_parameters);
            }
        }
        _ => {}
    }
}

/// Rewrites one ref string's 2.0 registry fragment.
fn rewrite_ref_text(
    raw: &str,
    body_parameters: &std::collections::BTreeSet<String>,
) -> Option<String> {
    let (prefix, fragment) = raw.split_once('#')?;
    let rest = |registry: &str| fragment.strip_prefix(registry);
    let target = if let Some(name) = rest("/definitions/") {
        format!("schemas/{name}")
    } else if let Some(name) = rest("/parameters/") {
        // Only same-document body parameters are known to have moved.
        if prefix.is_empty() && body_parameters.contains(name) {
            format!("requestBodies/{name}")
        } else {
            format!("parameters/{name}")
        }
    } else {
        format!("responses/{}", rest("/responses/")?)
    };
    Some(format!("{prefix}#/components/{target}"))
}

/// Whether two scheme lists name the same set (order-insensitive).
fn same_scheme_set(op_schemes: &[Value], root_schemes: &[Value]) -> bool {
    let op_set: std::collections::BTreeSet<&str> =
        op_schemes.iter().filter_map(Value::as_str).collect();
    let root_set: std::collections::BTreeSet<&str> =
        root_schemes.iter().filter_map(Value::as_str).collect();
    !op_set.is_empty() && op_set == root_set
}

/// Upgrades a 2.0 schema to JSON Schema 2020-12 (the OAS 3.1 dialect):
/// `discriminator: <name>` → object form; `type: file` → binary string;
/// Boolean `exclusiveMinimum`/`exclusiveMaximum` → numeric bounds;
/// `x-nullable: true` → `"null"` added to `type`. Nested schemas are
/// upgraded recursively.
fn upgrade_schema(schema: &Value) -> Value {
    let obj = match schema {
        Value::Object(obj) => obj,
        Value::Array(items) => return Value::Array(items.iter().map(upgrade_schema).collect()),
        other => return other.clone(),
    };
    let mut out = Map::new();
    for (key, value) in obj {
        match (key.as_str(), value) {
            ("discriminator", Value::String(property)) => {
                out.insert("discriminator".into(), json!({ "propertyName": property }));
            }
            ("type", Value::String(t)) if t == "file" => {
                out.insert("type".into(), json!("string"));
                out.insert("format".into(), json!("binary"));
            }
            // Handled below, together with their numeric bound.
            ("exclusiveMinimum" | "exclusiveMaximum", Value::Bool(_)) | ("x-nullable", _) => {}
            // Maps from names to schemas.
            ("properties" | "patternProperties", Value::Object(members)) => {
                let members = members
                    .iter()
                    .map(|(name, member)| (name.clone(), upgrade_schema(member)))
                    .collect();
                out.insert(key.clone(), Value::Object(members));
            }
            // Keywords whose value is a schema or a list of schemas.
            ("items" | "additionalProperties" | "not" | "allOf" | "anyOf" | "oneOf", value) => {
                out.insert(key.clone(), upgrade_schema(value));
            }
            // Everything else (enum, default, example, required, x-*) is a
            // literal and is never reinterpreted as a schema.
            (_, value) => {
                out.insert(key.clone(), value.clone());
            }
        }
    }
    for (flag, bound) in [
        ("exclusiveMinimum", "minimum"),
        ("exclusiveMaximum", "maximum"),
    ] {
        if obj.get(flag) == Some(&Value::Bool(true))
            && let Some(limit) = out.remove(bound)
        {
            out.insert(flag.into(), limit);
        }
    }
    if obj.get("x-nullable") == Some(&Value::Bool(true)) {
        match out.get("type").cloned() {
            Some(Value::String(t)) => {
                out.insert("type".into(), json!([t, "null"]));
            }
            Some(Value::Array(mut types)) => {
                if !types.contains(&json!("null")) {
                    types.push(json!("null"));
                }
                out.insert("type".into(), Value::Array(types));
            }
            // No `type` (e.g. a `$ref` or composition): allow null beside it.
            _ => {
                let inner = Value::Object(std::mem::take(&mut out));
                out.insert("anyOf".into(), json!([inner, { "type": "null" }]));
            }
        }
    }
    Value::Object(out)
}

/// Upgrades 2.0 security definitions to 3.x security schemes: `basic` →
/// `http`/`basic`, OAuth `flow` → the `flows` map with renamed flow keys.
fn upgrade_security_definitions(defs: &Value) -> Value {
    let mut out = Map::new();
    if let Some(map) = defs.as_object() {
        for (name, scheme) in map {
            let Some(scheme_obj) = scheme.as_object() else {
                continue;
            };
            let mut upgraded = Map::new();
            for key in [
                "description",
                "type",
                "name",
                "in",
                "scheme",
                "bearerFormat",
                "openIdConnectUrl",
            ] {
                if let Some(value) = scheme_obj.get(key) {
                    upgraded.insert(key.into(), value.clone());
                }
            }
            let scheme_type = scheme_obj.get("type").and_then(Value::as_str).unwrap_or("");
            match scheme_type {
                "basic" => {
                    upgraded.insert("type".into(), json!("http"));
                    upgraded.insert("scheme".into(), json!("basic"));
                }
                "oauth2" => {
                    if let Some(flow) = scheme_obj.get("flow").and_then(Value::as_str) {
                        let flow_31 = match flow {
                            "implicit" => "implicit",
                            "password" => "password",
                            "application" => "clientCredentials",
                            "accessCode" => "authorizationCode",
                            other => other,
                        };
                        let mut flow_obj = Map::new();
                        for key in ["authorizationUrl", "tokenUrl", "scopes"] {
                            if let Some(value) = scheme_obj.get(key) {
                                flow_obj.insert(key.into(), value.clone());
                            }
                        }
                        upgraded.insert("flows".into(), json!({flow_31: Value::Object(flow_obj)}));
                    }
                }
                _ => {}
            }
            out.insert(name.clone(), Value::Object(upgraded));
        }
    }
    Value::Object(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use suspect_low::LowDoc;

    use suspect_source::{Source, Uri};

    const SWAGGER: &str = r#"
swagger: "2.0"
info: {title: Pets, version: "1"}
host: api.example.com
basePath: /v1
schemes: [https]
consumes: [application/json]
produces: [application/json]
securityDefinitions:
  Basic: {type: basic}
  OAuth:
    type: oauth2
    flow: application
    tokenUrl: https://auth.example.com/token
    scopes: {read: read access}
paths:
  /pets:
    get:
      operationId: listPets
      responses:
        '200':
          description: page of pets
          schema:
            $ref: '#/definitions/Pet'
    post:
      operationId: createPet
      parameters:
        - name: body
          in: body
          required: true
          schema:
            $ref: '#/definitions/Pet'
      responses:
        '201': {description: created}
  /pets/{petId}:
    delete:
      operationId: deletePet
      parameters:
        - name: petId
          in: path
          required: true
          type: string
      responses:
        '204': {description: removed}
definitions:
  Pet:
    type: object
    required: [name]
    discriminator: kind
    properties:
      name: {type: string}
      photo: {type: file}
"#;

    #[test]
    fn upgrades_the_full_document() {
        let uri = Uri::parse("mem://s.yaml").unwrap();
        let low = suspect_low::LowDoc::parse(
            uri,
            suspect_source::Source::from_vec(SWAGGER.as_bytes().to_vec()),
        );
        let doc = suspect_overlay::Value::from_node(low.root()).to_json();
        let doc = serde_json::from_str::<Value>(&doc).unwrap();
        let upgraded = upgrade(&doc).expect("upgrades");
        let obj = upgraded.as_object().unwrap();

        assert_eq!(obj["openapi"], "3.1.0");
        assert!(
            obj["servers"][0]["url"]
                .as_str()
                .is_some_and(|u| u.starts_with("https://api.example.com/v1"))
        );
        assert!(obj["components"]["schemas"]["Pet"].is_object());
        // discriminator: string → object form.
        assert_eq!(
            obj["components"]["schemas"]["Pet"]["discriminator"]["propertyName"],
            "kind"
        );
        // file → string + binary.
        assert_eq!(
            obj["components"]["schemas"]["Pet"]["properties"]["photo"],
            json!({"type": "string", "format": "binary"})
        );
        // basic → http/basic.
        assert_eq!(
            obj["components"]["securitySchemes"]["Basic"],
            json!({"type": "http", "scheme": "basic"})
        );
        // Operation-level schemes narrower than the root become
        // per-operation servers (not a marker extension).
        assert!(upgraded["paths"]["/pets"]["post"].get("servers").is_none());
        let op = upgrade(&doc_with_op_scheme()).unwrap();
        assert_eq!(
            op["paths"]["/pets/{petId}"]["delete"]["servers"][0]["url"],
            "http://api.example.com/v1"
        );
        // OAuth flow rename application → clientCredentials.
        assert!(
            obj["components"]["securitySchemes"]["OAuth"]["flows"]["clientCredentials"].is_object()
        );
        // #/definitions refs rewritten to #/components/schemas.
        let get = &obj["paths"]["/pets"]["get"]["responses"]["200"];
        assert_eq!(
            get["content"]["application/json"]["schema"]["$ref"],
            "#/components/schemas/Pet"
        );
        // body parameter → requestBody.
        let post = &obj["paths"]["/pets"]["post"];
        assert!(
            post["requestBody"]["content"]["application/json"]["schema"]["$ref"]
                .as_str()
                .is_some_and(|r| r.ends_with("Pet"))
        );
        // response schema → content.
        let get = &obj["paths"]["/pets"]["get"]["responses"]["200"];
        assert!(
            get["content"]["application/json"]["schema"]["$ref"]
                .as_str()
                .is_some_and(|r| r.ends_with("Pet"))
        );
    }

    /// A document whose delete operation constrains to http while the
    /// root allows https too.
    fn doc_with_op_scheme() -> Value {
        let uri = Uri::parse("mem://s.yaml").unwrap();
        let low = LowDoc::parse(
            uri,
            Source::from_vec(
                br#"swagger: "2.0"
info: {title: Pets, version: "1"}
host: api.example.com
basePath: /v1
schemes: [https, http]
paths:
  /pets/{petId}:
    delete:
      operationId: deletePet
      schemes: [http]
      responses:
        '204': {description: removed}
"#
                .to_vec(),
            ),
        );
        let json = suspect_overlay::Value::from_node(low.root()).to_json();
        serde_json::from_str(&json).unwrap()
    }

    fn upgraded(doc: Value) -> Value {
        upgrade(&doc).expect("upgrades")
    }

    #[test]
    fn root_media_types_apply_when_an_operation_declares_none() {
        let out = upgraded(json!({
            "swagger": "2.0", "info": {"title": "t", "version": "1"},
            "consumes": ["application/xml"], "produces": ["text/csv"],
            "paths": {"/r": {"post": {
                "parameters": [{"name": "b", "in": "body", "schema": {"type": "object"}}],
                "responses": {"200": {"description": "ok", "schema": {"type": "string"}}}
            }}}
        }));
        let op = &out["paths"]["/r"]["post"];
        assert!(op["requestBody"]["content"]["application/xml"].is_object());
        assert!(op["responses"]["200"]["content"]["text/csv"].is_object());
        assert!(
            op["responses"]["200"]["content"]
                .get("application/json")
                .is_none()
        );
    }

    #[test]
    fn registries_become_components_and_refs_follow() {
        let out = upgraded(json!({
            "swagger": "2.0", "info": {"title": "t", "version": "1"},
            "parameters": {
                "limit": {"name": "limit", "in": "query", "type": "integer"},
                "pet": {"name": "pet", "in": "body", "required": true, "schema": {"$ref": "#/definitions/Pet"}}
            },
            "responses": {"NotFound": {"description": "missing", "schema": {"$ref": "#/definitions/Err"}}},
            "definitions": {"Pet": {"type": "object"}, "Err": {"type": "object"}},
            "paths": {"/pets": {"post": {
                "parameters": [{"$ref": "#/parameters/limit"}, {"$ref": "#/parameters/pet"}],
                "responses": {"404": {"$ref": "#/responses/NotFound"}}
            }}}
        }));
        let op = &out["paths"]["/pets"]["post"];
        assert_eq!(op["parameters"][0]["$ref"], "#/components/parameters/limit");
        assert_eq!(op["requestBody"]["$ref"], "#/components/requestBodies/pet");
        assert_eq!(
            op["responses"]["404"]["$ref"],
            "#/components/responses/NotFound"
        );
        let components = &out["components"];
        assert_eq!(
            components["parameters"]["limit"]["schema"],
            json!({"type": "integer"})
        );
        assert_eq!(components["requestBodies"]["pet"]["required"], true);
        assert_eq!(
            components["requestBodies"]["pet"]["content"]["application/json"]["schema"]["$ref"],
            "#/components/schemas/Pet"
        );
        assert_eq!(
            components["responses"]["NotFound"]["content"]["application/json"]["schema"]["$ref"],
            "#/components/schemas/Err"
        );
    }

    #[test]
    fn path_level_parameters_are_converted_and_stay_on_the_path_item() {
        let out = upgraded(json!({
            "swagger": "2.0", "info": {"title": "t", "version": "1"},
            "paths": {"/pets/{id}": {
                "parameters": [{"name": "id", "in": "path", "required": true, "type": "string"}],
                "get": {"responses": {"200": {"description": "ok"}}}
            }}
        }));
        let item = &out["paths"]["/pets/{id}"];
        assert_eq!(item["parameters"][0]["schema"], json!({"type": "string"}));
        assert!(item["parameters"][0].get("type").is_none());
        assert!(
            item["get"].get("parameters").is_none(),
            "not duplicated per operation"
        );
    }

    #[test]
    fn schema_keywords_are_upgraded_to_2020_12() {
        let out = upgraded(json!({
            "swagger": "2.0", "info": {"title": "t", "version": "1"},
            "paths": {"/r": {"post": {
                "parameters": [{"name": "b", "in": "body", "schema": {
                    "type": "object",
                    "properties": {
                        "n": {"type": "number", "minimum": 0, "exclusiveMinimum": true, "maximum": 9, "exclusiveMaximum": false},
                        "maybe": {"type": "string", "x-nullable": true},
                        "enum": {"type": "string", "discriminator": "kind"},
                        "f": {"type": "file"}
                    }
                }}],
                "responses": {"200": {"description": "ok"}}
            }}}
        }));
        let props = &out["paths"]["/r"]["post"]["requestBody"]["content"]["application/json"]["schema"]
            ["properties"];
        assert_eq!(
            props["n"],
            json!({"type": "number", "exclusiveMinimum": 0, "maximum": 9})
        );
        assert_eq!(props["maybe"]["type"], json!(["string", "null"]));
        // A property named like a keyword is still a schema.
        assert_eq!(
            props["enum"]["discriminator"],
            json!({"propertyName": "kind"})
        );
        assert_eq!(props["f"], json!({"type": "string", "format": "binary"}));
    }

    #[test]
    fn collection_formats_headers_examples_and_extensions() {
        let out = upgraded(json!({
            "swagger": "2.0", "info": {"title": "t", "version": "1"}, "x-root": 1,
            "produces": ["application/json"],
            "paths": {"/r": {"get": {
                "x-op": true,
                "parameters": [
                    {"name": "ids", "in": "query", "type": "array", "items": {"type": "string"}, "collectionFormat": "multi"},
                    {"name": "tags", "in": "query", "type": "array", "items": {"type": "string"}, "collectionFormat": "pipes"}
                ],
                "responses": {"200": {
                    "description": "ok",
                    "headers": {"X-Rate": {"type": "integer", "description": "left"}},
                    "schema": {"type": "object"},
                    "examples": {"application/json": {"a": 1}}
                }}
            }}}
        }));
        let op = &out["paths"]["/r"]["get"];
        assert_eq!(
            (
                op["parameters"][0]["style"].clone(),
                op["parameters"][0]["explode"].clone()
            ),
            (json!("form"), json!(true))
        );
        assert_eq!(op["parameters"][1]["style"], "pipeDelimited");
        let ok = &op["responses"]["200"];
        assert_eq!(
            ok["headers"]["X-Rate"],
            json!({"description": "left", "schema": {"type": "integer"}})
        );
        assert_eq!(
            ok["content"]["application/json"]["example"],
            json!({"a": 1})
        );
        assert_eq!(op["x-op"], true);
        assert_eq!(out["x-root"], 1);
    }

    #[test]
    fn a_missing_host_gives_a_relative_server() {
        let out = upgraded(json!({
            "swagger": "2.0", "info": {"title": "t", "version": "1"}, "basePath": "/v2", "paths": {}
        }));
        assert_eq!(out["servers"], json!([{"url": "/v2"}]));
    }

    #[test]
    fn non_swagger_documents_are_rejected() {
        let doc = json!({"openapi": "3.1.0"});
        assert!(upgrade(&doc).is_err());
    }
}
