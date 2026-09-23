//! Swagger 2.0 → OpenAPI 3.1 upgrader.
//!
//! Translates a materialized Swagger 2.0 document into an OpenAPI 3.1
//! document: `host`/`basePath`/`schemes` fold into `servers`,
//! `definitions` moves to `components/schemas`, `securityDefinitions` to
//! `components/securitySchemes` (with OAuth flow renames), body/formData
//! parameters become request bodies, `response.schema` becomes response
//! `content`, and `consumes`/`produces` become per-operation content keys.
//!
//! The output is a plain JSON value — the caller serializes it as YAML or
//! JSON and verifies it with the OpenAPI 3.1 validation battery.

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

    // Servers: schemes x host x basePath fold into one server entry per
    // scheme (or a single entry when schemes is absent).
    let schemes: Vec<&str> = obj
        .get("schemes")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_str).collect())
        .unwrap_or_else(|| vec!["https"]);
    let host = obj.get("host").and_then(Value::as_str).unwrap_or("");
    let base_path = obj.get("basePath").and_then(Value::as_str).unwrap_or("");
    let servers: Vec<Value> = schemes
        .iter()
        .map(|scheme| {
            json!({"url": format!("{scheme}://{host}{base_path}"), "description": format!("{scheme} server")})
        })
        .collect();
    if !servers.is_empty() {
        out.insert("servers".into(), Value::Array(servers));
    }

    let root_consumes = media_types(obj.get("consumes"));
    let root_produces = media_types(obj.get("produces"));

    // Paths: upgrade every operation, threading consumes/produces.
    if let Value::Object(path_map) = &paths {
        let mut upgraded_paths = Map::new();
        for (path_key, path_item) in path_map {
            if let Some(upgraded) = upgrade_path_item(path_item, &root_consumes, &root_produces) {
                upgraded_paths.insert(path_key.clone(), upgraded);
            }
        }
        out.insert("paths".into(), Value::Object(upgraded_paths));
    }

    // Components: definitions, securityDefinitions, parameter/response
    // registries.
    let mut components = Map::new();
    if let Some(definitions) = obj.get("definitions") {
        let schemas: Map<String, Value> = definitions
            .as_object()
            .map(|m| {
                m.iter()
                    .map(|(k, v)| (k.clone(), upgrade_schema(v)))
                    .collect()
            })
            .unwrap_or_default();
        components.insert("schemas".into(), Value::Object(schemas));
    }
    if let Some(security_definitions) = obj.get("securityDefinitions") {
        components.insert(
            "securitySchemes".into(),
            upgrade_security_definitions(security_definitions),
        );
    }
    if !components.is_empty() {
        out.insert("components".into(), Value::Object(components));
    }

    // Pass-through keys with identical 3.x shapes.
    for key in ["security", "tags", "externalDocs"] {
        if let Some(value) = obj.get(key) {
            out.insert(key.into(), value.clone());
        }
    }

    let mut upgraded = Value::Object(out);
    rewrite_definition_refs(&mut upgraded);
    Ok(upgraded)
}

/// Rewrites `#/definitions/<name>` and `...yaml#/definitions/<name>` ref
/// fragments to `...#/components/schemas/<name>` across the whole tree.
fn rewrite_definition_refs(value: &mut Value) {
    match value {
        Value::Object(map) => {
            for (key, child) in map.iter_mut() {
                if key == "$ref"
                    && let Some(text) = child.as_str()
                    && let Some(rewritten) = rewrite_ref_text(text)
                {
                    *child = Value::String(rewritten);
                    continue;
                }
                rewrite_definition_refs(child);
            }
        }
        Value::Array(items) => {
            for item in items {
                rewrite_definition_refs(item);
            }
        }
        _ => {}
    }
}

/// Rewrites one ref string's `#/definitions/` fragment.
fn rewrite_ref_text(raw: &str) -> Option<String> {
    let (prefix, fragment) = raw.split_once('#')?;
    if !fragment.starts_with("/definitions/") {
        return None;
    }
    Some(format!(
        "{prefix}#/components/schemas/{}",
        &fragment["/definitions/".len()..]
    ))
}

/// Normalizes a 2.0 media-type array to a list of strings; defaults to
/// `application/json` per the Swagger 2.0 spec when absent.
fn media_types(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .filter(|v: &Vec<String>| !v.is_empty())
        .unwrap_or_else(|| vec!["application/json".to_owned()])
}

/// Upgrades one path item: methods plus path-level parameters.
fn upgrade_path_item(
    path_item: &Value,
    root_consumes: &[String],
    root_produces: &[String],
) -> Option<Value> {
    let item = path_item.as_object()?;
    let methods = ["get", "put", "post", "delete", "options", "head", "patch"];
    let mut out = Map::new();

    // Path-level parameters cascade to operations in 2.0 and 3.x alike.
    let path_params: Vec<Value> = item
        .get("parameters")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    for method in methods {
        let Some(op) = item.get(method) else {
            continue;
        };
        let Some(op_obj) = op.as_object() else {
            continue;
        };
        let consumes = media_types(op_obj.get("consumes"));
        let consumes = if consumes.is_empty() {
            root_consumes.to_vec()
        } else {
            consumes
        };
        let produces = media_types(op_obj.get("produces"));
        let produces = if produces.is_empty() {
            root_produces.to_vec()
        } else {
            produces
        };
        let params: Vec<Value> = op_obj
            .get("parameters")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();

        let mut upgraded = Map::new();
        // Identity keys.
        for key in [
            "tags",
            "summary",
            "description",
            "externalDocs",
            "operationId",
            "deprecated",
            "security",
        ] {
            if let Some(value) = op_obj.get(key) {
                upgraded.insert(key.into(), value.clone());
            }
        }

        // Split parameters: body → requestBody, formData → aggregated
        // form body, others stay parameters with schema keywords.
        let mut parameters: Vec<Value> = path_params.clone();
        let mut form_properties = Map::new();
        let mut form_required: Vec<Value> = Vec::new();
        let mut request_body: Option<Value> = None;

        for param in &params {
            let Some(param_obj) = param.as_object() else {
                continue;
            };
            let location = param_obj.get("in").and_then(Value::as_str).unwrap_or("");
            match location {
                "body" => {
                    let schema = param_obj.get("schema").cloned().unwrap_or(Value::Null);
                    request_body = Some(json!({
                        "required": param_obj.get("required").and_then(Value::as_bool).unwrap_or(false),
                        "content": {consumes[0].clone(): {"schema": schema}},
                    }));
                }
                "formData" => {
                    let name = param_obj.get("name").and_then(Value::as_str).unwrap_or("");
                    let mut prop = serde_json::Map::new();
                    for key in [
                        "type",
                        "format",
                        "items",
                        "enum",
                        "minimum",
                        "maximum",
                        "minLength",
                        "maxLength",
                        "pattern",
                    ] {
                        if let Some(value) = param_obj.get(key) {
                            prop.insert(key.into(), value.clone());
                        }
                    }
                    if param_obj.get("type").and_then(Value::as_str) == Some("file") {
                        prop.insert("type".into(), json!("string"));
                        prop.insert("format".into(), json!("binary"));
                    }
                    form_properties.insert(name.to_owned(), Value::Object(prop));
                    if param_obj.get("required").and_then(Value::as_bool) == Some(true) {
                        form_required.push(json!(name));
                    }
                }
                _ => {
                    let mut upgraded_param = Map::new();
                    for key in [
                        "name",
                        "in",
                        "description",
                        "required",
                        "deprecated",
                        "allowEmptyValue",
                    ] {
                        if let Some(value) = param_obj.get(key) {
                            upgraded_param.insert(key.into(), value.clone());
                        }
                    }
                    // Schema keywords carried directly on the 2.0 parameter.
                    let schema = upgrade_schema(&Value::Object(param_obj.clone()));
                    if let Some(schema_obj) = schema.as_object() {
                        let mut extracted = Map::new();
                        for schema_key in [
                            "type",
                            "format",
                            "items",
                            "enum",
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
                            "default",
                        ] {
                            if let Some(value) = schema_obj.get(schema_key) {
                                extracted.insert(schema_key.into(), value.clone());
                            }
                        }
                        upgraded_param.insert("schema".into(), Value::Object(extracted));
                    }
                    parameters.push(Value::Object(upgraded_param));
                }
            }
        }

        // A formData aggregate or a body parameter becomes requestBody.
        if request_body.is_none() && !form_properties.is_empty() {
            let is_multipart = params
                .iter()
                .any(|p| p.get("type").and_then(Value::as_str) == Some("file"));
            let media = if is_multipart {
                "multipart/form-data"
            } else {
                "application/x-www-form-urlencoded"
            };
            let mut form_schema = json!({"type": "object", "properties": form_properties});
            if !form_required.is_empty() {
                form_schema["required"] = Value::Array(form_required);
            }
            request_body = Some(json!({"content": {media: {"schema": form_schema}}}));
        }
        if let Some(body) = request_body {
            upgraded.insert("requestBody".into(), body);
        }
        if !parameters.is_empty() {
            upgraded.insert("parameters".into(), Value::Array(parameters));
        }

        // Responses: schema → content.<produces>.schema.
        if let Some(responses) = op_obj.get("responses") {
            let mut upgraded_responses = Map::new();
            if let Some(resp_map) = responses.as_object() {
                for (status, response) in resp_map {
                    let Some(resp_obj) = response.as_object() else {
                        upgraded_responses.insert(status.clone(), response.clone());
                        continue;
                    };
                    let mut upgraded_response = Map::new();
                    if let Some(description) = resp_obj.get("description") {
                        upgraded_response.insert("description".into(), description.clone());
                    }
                    if let Some(headers) = resp_obj.get("headers") {
                        upgraded_response.insert("headers".into(), headers.clone());
                    }
                    let mut content = Map::new();
                    for produce in &produces {
                        let mut media_entry = Map::new();
                        if let Some(schema) = resp_obj.get("schema") {
                            media_entry.insert("schema".into(), upgrade_schema(schema));
                        }
                        if let Some(examples) = resp_obj.get("examples") {
                            media_entry.insert("examples".into(), examples.clone());
                        }
                        content.insert(produce.clone(), Value::Object(media_entry));
                    }
                    if !content.is_empty() {
                        upgraded_response.insert("content".into(), Value::Object(content));
                    }
                    upgraded_responses.insert(status.clone(), Value::Object(upgraded_response));
                }
            }
            upgraded.insert("responses".into(), Value::Object(upgraded_responses));
        }

        // Schemes at operation level fold into a servers override.
        if let Some(op_schemes) = op_obj.get("schemes").and_then(Value::as_array) {
            if let Some(host) = host_of(&upgraded) {
                let _ = host;
            }
            // 2.0 operation schemes constrain the protocol; the 3.1 form is
            // per-operation servers. We cannot reconstruct the host here
            // without the root — leave a marker extension.
            let schemes_list: Vec<Value> = op_schemes.clone();
            upgraded.insert("x-schemes".into(), Value::Array(schemes_list));
        }

        out.insert(method.into(), Value::Object(upgraded));
    }

    // $ref path items and other non-method keys pass through.
    for (key, value) in item {
        if !methods.contains(&key.as_str()) && key != "parameters" {
            out.insert(key.clone(), value.clone());
        }
    }

    Some(Value::Object(out))
}

/// Extracts the 2.0 root host (helper for future server-per-operation
/// overrides).
fn host_of(_upgraded: &Map<String, Value>) -> Option<&str> {
    None
}

/// Upgrades a 2.0 schema object: `discriminator: string` → object form,
/// `type: file` → binary string; other keywords carry over.
fn upgrade_schema(schema: &Value) -> Value {
    match schema {
        Value::Object(obj) => {
            let mut out = Map::new();
            for (key, value) in obj {
                match (key.as_str(), value) {
                    ("discriminator", Value::String(property)) => {
                        out.insert("discriminator".into(), json!({"propertyName": property}));
                    }
                    ("type", Value::String(t)) if t == "file" => {
                        out.insert("type".into(), json!("string"));
                        out.insert("format".into(), json!("binary"));
                    }
                    (_, value) => {
                        out.insert(key.clone(), upgrade_schema(value));
                    }
                }
            }
            Value::Object(out)
        }
        Value::Array(items) => Value::Array(items.iter().map(upgrade_schema).collect()),
        other => other.clone(),
    }
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
    use suspect_source::Uri;

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

    #[test]
    fn non_swagger_documents_are_rejected() {
        let doc = json!({"openapi": "3.1.0"});
        assert!(upgrade(&doc).is_err());
    }
}
