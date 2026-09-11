//! Source-linked HTTP validation. Invalid declarations stay in raw storage;
//! diagnostics prevent consumers from treating missing typed values as defaults.

use std::collections::HashSet;

use serde_json::Value;

use super::http::Object;
use super::{
    Contract, ContractDiagnostic, ContractSeverity, HttpMethod, Parameter, ParameterLocation,
    ParameterStyle, ResponseStatus, SourceId,
};

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
enum Kind {
    PathItem,
    Operation,
    Parameter,
    Header,
    Body,
    Response,
    Media,
    Encoding,
    Server,
    Variable,
    SecurityScheme,
    Example,
    Link,
    Callback,
}

pub(super) fn validate(contract: &Contract) -> Vec<ContractDiagnostic> {
    let mut validator = Validator {
        contract,
        seen: HashSet::new(),
        diagnostics: Vec::new(),
    };
    let source = SourceId::new(contract.entry().clone(), suspect_low::Pointer::root());
    let root = Object::new(contract, source);
    validator.array(&root, "servers", Kind::Server);
    validator.security(&root);
    for key in ["paths", "webhooks"] {
        if key == "webhooks"
            && contract.openapi_version().starts_with("3.0.")
            && root.field(key).is_some()
        {
            validator.error(
                root.source.child(key),
                "UNSUPPORTED_HTTP_FIELD",
                "webhooks requires OpenAPI 3.1 or later",
            );
        }
        if key != "webhooks" || !contract.openapi_version().starts_with("3.0.") {
            validator.map(&root, key, Kind::PathItem, key == "paths");
        }
    }
    if let Some((source, value)) = root.field("components") {
        if value.is_object() {
            let components = Object::new(contract, source);
            for (key, kind) in [
                ("parameters", Kind::Parameter),
                ("headers", Kind::Header),
                ("requestBodies", Kind::Body),
                ("responses", Kind::Response),
                ("securitySchemes", Kind::SecurityScheme),
                ("examples", Kind::Example),
                ("links", Kind::Link),
                ("callbacks", Kind::Callback),
                ("pathItems", Kind::PathItem),
                ("mediaTypes", Kind::Media),
            ] {
                let version = contract.openapi_version_at(&components.source);
                if key == "mediaTypes" && !version.starts_with("3.2.")
                    || key == "pathItems" && version.starts_with("3.0.")
                {
                    if components.field(key).is_some() {
                        validator.error(
                            components.source.child(key),
                            "UNSUPPORTED_HTTP_FIELD",
                            format!("components.{key} is not defined in OpenAPI {version}"),
                        );
                    }
                    continue;
                }
                validator.map(&components, key, kind, false);
                if key == "mediaTypes"
                    && let Some((source, value)) = components.field(key)
                    && let Some(map) = value.as_object()
                {
                    for name in map.keys() {
                        if name.is_empty()
                            || !name
                                .bytes()
                                .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
                        {
                            validator.error(
                                source.child(name),
                                "INVALID_COMPONENT_NAME",
                                "component names must match ^[a-zA-Z0-9._-]+$",
                            );
                        }
                    }
                }
            }
        } else {
            validator.error(source, "INVALID_HTTP_SHAPE", "components must be an object");
        }
    }
    validator.diagnostics
}

pub(super) fn parameter_set(contract: &Contract, sources: &[SourceId]) -> Vec<ContractDiagnostic> {
    let mut validator = Validator {
        contract,
        seen: HashSet::new(),
        diagnostics: Vec::new(),
    };
    validator.parameter_set(sources);
    validator.diagnostics
}

struct Validator<'a> {
    contract: &'a Contract,
    seen: HashSet<(SourceId, Kind)>,
    diagnostics: Vec<ContractDiagnostic>,
}

impl<'a> Validator<'a> {
    fn error(&mut self, source: SourceId, code: &'static str, message: impl Into<String>) {
        let at = self.contract.source_span(&source).unwrap_or_default();
        self.diagnostics.push(ContractDiagnostic {
            source,
            at,
            code,
            severity: ContractSeverity::Error,
            message: message.into(),
        });
    }

    fn visit(&mut self, source: SourceId, kind: Kind) {
        if !self.seen.insert((source.clone(), kind)) {
            return;
        }
        let mut object = Object::new(self.contract, source);
        object.path_item = kind == Kind::PathItem;
        let Some(raw) = object.raw().as_object() else {
            self.error(
                object.source,
                "INVALID_HTTP_SHAPE",
                format!("{kind:?} must be an object"),
            );
            return;
        };
        let reference_allowed = matches!(
            kind,
            Kind::PathItem
                | Kind::Parameter
                | Kind::Header
                | Kind::Body
                | Kind::Response
                | Kind::SecurityScheme
                | Kind::Example
                | Kind::Link
                | Kind::Callback
        ) || kind == Kind::Media && self.is_32(&object);
        if reference_allowed && raw.contains_key("$ref") {
            if raw.get("$ref").is_some_and(Value::is_string)
                && !self.contract.reference_targets.contains_key(&object.source)
            {
                self.error(
                    object.source.child("$ref"),
                    "HTTP_REFERENCE_UNINDEXED",
                    "HTTP reference is not present in the semantic reference graph",
                );
            }
            let mut chain = HashSet::new();
            let mut next = Some(object.source.clone());
            while let Some(source) = next {
                if !chain.insert(source.clone()) {
                    self.error(
                        object.source.child("$ref"),
                        "HTTP_REFERENCE_CYCLE",
                        "HTTP reference does not resolve to a finite object",
                    );
                    break;
                }
                next = self
                    .contract
                    .reference_targets
                    .get(&source)
                    .cloned()
                    .flatten();
            }
            if let Some(Some(target)) = self.contract.reference_targets.get(&object.source) {
                self.visit(target.clone(), kind);
                if kind == Kind::PathItem {
                    let referenced = Object {
                        contract: self.contract,
                        source: target.clone(),
                        path_item: true,
                    };
                    for name in raw
                        .keys()
                        .filter(|name| name.as_str() != "$ref" && !name.starts_with("x-"))
                    {
                        if referenced.field(name).is_some() {
                            self.error(object.source.child(name), "AMBIGUOUS_PATH_ITEM_REFERENCE", format!("Path Item field {name:?} is present in both the declaration and its reference; OpenAPI leaves this conflict undefined"));
                        }
                    }
                }
            }
            if kind != Kind::PathItem {
                if !self
                    .contract
                    .openapi_version_at(&object.source)
                    .starts_with("3.0.")
                {
                    for name in ["summary", "description"] {
                        if let Some(value) = raw.get(name)
                            && !value.is_string()
                        {
                            self.error(
                                object.source.child(name),
                                "INVALID_HTTP_SHAPE",
                                format!("Reference Object {name} must be a string"),
                            );
                        }
                    }
                }
                return;
            }
        }
        self.known_fields(&object, kind);
        match kind {
            Kind::PathItem => {
                self.strings(&object, &["summary", "description"]);
                self.parameters(&object);
                self.array(&object, "servers", Kind::Server);
                for (name, _) in super::method::FIXED_METHODS {
                    let field = if name == "query" {
                        object.field_32(name)
                    } else {
                        object.field(name)
                    };
                    if let Some((source, _)) = field {
                        self.visit(source, Kind::Operation);
                    }
                }
                if let Some((source, value)) = object.field_32("additionalOperations") {
                    if let Some(map) = value.as_object() {
                        for token in map.keys() {
                            if HttpMethod::parse(token).is_none() {
                                self.error(
                                    source.child(token),
                                    "INVALID_HTTP_METHOD",
                                    "additionalOperations keys must be nonempty ASCII HTTP tokens",
                                );
                            } else if super::method::additional_method(token).is_none() {
                                self.error(source.child(token), "DUPLICATE_HTTP_METHOD",
                                    "a method defined by a fixed Path Item field must not appear in additionalOperations");
                            }
                            self.visit(source.child(token), Kind::Operation);
                        }
                    } else {
                        self.error(
                            source,
                            "INVALID_HTTP_SHAPE",
                            "additionalOperations must be an object",
                        );
                    }
                }
            }
            Kind::Operation => {
                self.strings(&object, &["operationId", "summary", "description"]);
                self.booleans(&object, &["deprecated"]);
                self.string_array(&object, "tags");
                self.parameters(&object);
                self.array(&object, "servers", Kind::Server);
                self.security(&object);
                if let Some((source, _)) = object.field("requestBody") {
                    self.visit(source, Kind::Body);
                }
                if let Some((source, value)) = object.field("responses") {
                    if let Some(map) = value.as_object() {
                        if map.keys().all(|k| k.starts_with("x-")) {
                            self.error(
                                source.clone(),
                                "INVALID_HTTP_SHAPE",
                                "responses must contain at least one response",
                            );
                        }
                        for key in map.keys().filter(|k| !k.starts_with("x-")) {
                            if ResponseStatus::parse(key).is_none() {
                                self.error(source.child(key), "INVALID_RESPONSE_STATUS", "response key must be an exact 100–599 status, an uppercase 1XX–5XX range, or default");
                            }
                        }
                    }
                } else if self
                    .contract
                    .openapi_version_at(&object.source)
                    .starts_with("3.0.")
                {
                    self.error(
                        object.source.clone(),
                        "INVALID_HTTP_SHAPE",
                        "operation requires responses",
                    );
                }
                self.map(&object, "responses", Kind::Response, true);
                self.map(&object, "callbacks", Kind::Callback, false);
                self.external_docs(&object);
            }
            Kind::Parameter | Kind::Header => self.parameter(&object, kind),
            Kind::Body => {
                self.strings(&object, &["description"]);
                self.booleans(&object, &["required"]);
                self.required_field(&object, "content");
                self.map(&object, "content", Kind::Media, false);
            }
            Kind::Response => {
                if self.is_32(&object) {
                    self.strings(&object, &["summary", "description"]);
                } else {
                    self.required_string(&object, "description");
                }
                self.map(&object, "content", Kind::Media, false);
                self.map(&object, "headers", Kind::Header, false);
                self.map(&object, "links", Kind::Link, false);
            }
            Kind::Media => {
                self.examples(&object);
                self.encoding_fields(&object, true);
            }
            Kind::Encoding => {
                self.strings(&object, &["contentType"]);
                self.booleans(&object, &["explode", "allowReserved"]);
                self.style(&object, Some(ParameterLocation::Query));
                self.map(&object, "headers", Kind::Header, false);
                if self.is_32(&object) {
                    self.encoding_fields(&object, false);
                }
            }
            Kind::Server => self.server(&object),
            Kind::Variable => {
                self.required_string(&object, "default");
                self.strings(&object, &["description"]);
                self.string_array(&object, "enum");
                if let Some((source, value)) = object.field("enum")
                    && let Some(values) = value.as_array()
                {
                    if values.is_empty() {
                        self.error(
                            source,
                            "INVALID_SERVER_VARIABLE",
                            "server variable enum must not be empty",
                        );
                    }
                    if let Some((source, default)) = object.field("default")
                        && !values.contains(default)
                    {
                        self.error(
                            source,
                            "INVALID_SERVER_VARIABLE",
                            "server variable default must be a member of enum",
                        );
                    }
                }
            }
            Kind::SecurityScheme => self.security_scheme(&object),
            Kind::Example => {
                self.strings(&object, &["summary", "description", "externalValue"]);
                self.exclusive(&object, "value", "externalValue");
                if self.is_32(&object) {
                    self.strings(&object, &["serializedValue"]);
                    self.exclusive(&object, "value", "dataValue");
                    self.exclusive(&object, "value", "serializedValue");
                    self.exclusive(&object, "serializedValue", "externalValue");
                }
            }
            Kind::Link => {
                self.strings(&object, &["operationId", "operationRef", "description"]);
                self.exclusive(&object, "operationId", "operationRef");
                if object.field("operationId").is_none() && object.field("operationRef").is_none() {
                    self.error(
                        object.source.clone(),
                        "INVALID_HTTP_SHAPE",
                        "link requires operationId or operationRef",
                    );
                }
                self.object_field(&object, "parameters");
                if let Some((source, _)) = object.field("server") {
                    self.visit(source, Kind::Server);
                }
            }
            Kind::Callback => {
                for name in raw.keys().filter(|key| !key.starts_with("x-")) {
                    self.visit(object.source.child(name), Kind::PathItem);
                }
            }
        }
    }

    fn known_fields(&mut self, object: &Object<'_>, kind: Kind) {
        let fields: &[&str] = match kind {
            Kind::PathItem => &[
                "$ref",
                "summary",
                "description",
                "get",
                "put",
                "post",
                "delete",
                "options",
                "head",
                "patch",
                "trace",
                "servers",
                "parameters",
            ],
            Kind::Operation => &[
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
            ],
            Kind::Parameter => &[
                "name",
                "in",
                "description",
                "required",
                "deprecated",
                "allowEmptyValue",
                "style",
                "explode",
                "allowReserved",
                "schema",
                "example",
                "examples",
                "content",
            ],
            Kind::Header => &[
                "description",
                "required",
                "deprecated",
                "style",
                "explode",
                "schema",
                "example",
                "examples",
                "content",
            ],
            Kind::Body => &["description", "content", "required"],
            Kind::Response => &["description", "headers", "content", "links"],
            Kind::Media => &["schema", "example", "examples", "encoding"],
            Kind::Encoding => &[
                "contentType",
                "headers",
                "style",
                "explode",
                "allowReserved",
            ],
            Kind::Server => &["url", "description", "variables"],
            Kind::Variable => &["enum", "default", "description"],
            Kind::SecurityScheme => &[
                "type",
                "description",
                "name",
                "in",
                "scheme",
                "bearerFormat",
                "flows",
                "openIdConnectUrl",
            ],
            Kind::Example => &["summary", "description", "value", "externalValue"],
            Kind::Link => &[
                "operationRef",
                "operationId",
                "parameters",
                "requestBody",
                "description",
                "server",
            ],
            Kind::Callback => return,
        };
        let mut fields = fields.to_vec();
        if self.is_32(object) {
            fields.extend_from_slice(match kind {
                Kind::PathItem => &["query", "additionalOperations"],
                Kind::Response => &["summary"],
                Kind::Media => &["itemSchema", "prefixEncoding", "itemEncoding"],
                Kind::Encoding => &["encoding", "prefixEncoding", "itemEncoding"],
                Kind::Server => &["name"],
                Kind::SecurityScheme => &["oauth2MetadataUrl", "deprecated"],
                Kind::Example => &["dataValue", "serializedValue"],
                _ => &[],
            });
        }
        self.fixed_fields(object, &format!("{kind:?}"), &fields);
    }

    fn is_32(&self, object: &Object<'_>) -> bool {
        self.contract
            .openapi_version_at(&object.source)
            .starts_with("3.2.")
    }

    fn fixed_fields(&mut self, object: &Object<'_>, kind: &str, fields: &[&str]) {
        for key in object.raw().as_object().into_iter().flat_map(|m| m.keys()) {
            if !key.starts_with("x-") && !fields.contains(&key.as_str()) {
                self.error(
                    object.source.child(key),
                    "UNSUPPORTED_HTTP_FIELD",
                    format!(
                        "unsupported {kind} field {key:?}; its value is retained in raw source"
                    ),
                );
            }
        }
    }

    fn required_field(&mut self, object: &Object<'_>, key: &str) {
        if object.field(key).is_none() {
            self.error(
                object.source.clone(),
                "INVALID_HTTP_SHAPE",
                format!("missing required {key}"),
            );
        }
    }
    fn required_string(&mut self, object: &Object<'_>, key: &str) {
        self.required_field(object, key);
        self.strings(object, &[key]);
    }
    fn strings(&mut self, object: &Object<'_>, keys: &[&str]) {
        for key in keys {
            if let Some((source, value)) = object.field(key)
                && !value.is_string()
            {
                self.error(
                    source,
                    "INVALID_HTTP_SHAPE",
                    format!("{key} must be a string"),
                );
            }
        }
    }
    fn booleans(&mut self, object: &Object<'_>, keys: &[&str]) {
        for key in keys {
            if let Some((source, value)) = object.field(key)
                && !value.is_boolean()
            {
                self.error(
                    source,
                    "INVALID_HTTP_SHAPE",
                    format!("{key} must be a boolean"),
                );
            }
        }
    }
    fn string_array(&mut self, object: &Object<'_>, key: &str) {
        if let Some((source, value)) = object.field(key) {
            if let Some(values) = value.as_array() {
                for (i, value) in values.iter().enumerate() {
                    if !value.is_string() {
                        self.error(
                            source.child(&i.to_string()),
                            "INVALID_HTTP_SHAPE",
                            format!("{key} entries must be strings"),
                        );
                    }
                }
            } else {
                self.error(
                    source,
                    "INVALID_HTTP_SHAPE",
                    format!("{key} must be an array"),
                );
            }
        }
    }
    fn object_field(&mut self, object: &Object<'_>, key: &str) {
        if let Some((source, value)) = object.field(key)
            && !value.is_object()
        {
            self.error(
                source,
                "INVALID_HTTP_SHAPE",
                format!("{key} must be an object"),
            );
        }
    }
    fn array(&mut self, object: &Object<'_>, key: &str, kind: Kind) {
        if let Some((source, value)) = object.field(key) {
            if let Some(values) = value.as_array() {
                for i in 0..values.len() {
                    self.visit(source.child(&i.to_string()), kind);
                }
            } else {
                self.error(
                    source,
                    "INVALID_HTTP_SHAPE",
                    format!("{key} must be an array"),
                );
            }
        }
    }
    fn map(&mut self, object: &Object<'_>, key: &str, kind: Kind, extensions: bool) {
        if let Some((source, value)) = object.field(key) {
            if let Some(values) = value.as_object() {
                for name in values
                    .keys()
                    .filter(|name| !(extensions && name.starts_with("x-")))
                {
                    self.visit(source.child(name), kind);
                }
            } else {
                self.error(
                    source,
                    "INVALID_HTTP_SHAPE",
                    format!("{key} must be an object"),
                );
            }
        }
    }
    fn exclusive(&mut self, object: &Object<'_>, first: &str, second: &str) {
        if object.field(first).is_some() && object.field(second).is_some() {
            self.error(
                object.source.clone(),
                "INVALID_HTTP_COMBINATION",
                format!("{first} and {second} are mutually exclusive"),
            );
        }
    }
    fn examples(&mut self, object: &Object<'_>) {
        self.exclusive(object, "example", "examples");
        self.map(object, "examples", Kind::Example, false);
    }
    fn encoding_fields(&mut self, object: &Object<'_>, media: bool) {
        self.map(object, "encoding", Kind::Encoding, false);
        if !self.is_32(object) {
            return;
        }
        self.exclusive(object, "encoding", "prefixEncoding");
        self.exclusive(object, "encoding", "itemEncoding");
        self.array(object, "prefixEncoding", Kind::Encoding);
        if let Some((source, _)) = object.field("itemEncoding") {
            self.visit(source, Kind::Encoding);
        }
        if media
            && object.field("itemSchema").is_none()
            && let Some((source, _)) = object
                .field("prefixEncoding")
                .or_else(|| object.field("itemEncoding"))
        {
            let schema = object.field("schema");
            let explicit_non_array = schema
                .as_ref()
                .and_then(|(_, schema)| schema.get("type"))
                .is_some_and(|value| match value {
                    Value::String(name) => name != "array",
                    Value::Array(names) => !names.iter().any(|name| name == "array"),
                    _ => false,
                });
            if schema.is_none() || explicit_non_array {
                self.error(
                    source,
                    "INVALID_ENCODING_SCHEMA",
                    "positional encodings require itemSchema or an array schema",
                );
            }
        }
    }
    fn external_docs(&mut self, object: &Object<'_>) {
        if let Some((source, value)) = object.field("externalDocs") {
            if value.is_object() {
                let docs = Object::new(self.contract, source);
                self.fixed_fields(&docs, "ExternalDocumentation", &["url", "description"]);
                self.required_string(&docs, "url");
                self.strings(&docs, &["description"]);
            } else {
                self.error(
                    source,
                    "INVALID_HTTP_SHAPE",
                    "externalDocs must be an object",
                );
            }
        }
    }
    fn parameters(&mut self, object: &Object<'_>) {
        self.array(object, "parameters", Kind::Parameter);
        if let Some((source, value)) = object.field("parameters")
            && let Some(values) = value.as_array()
        {
            let mut names = HashSet::new();
            for i in 0..values.len() {
                let parameter = Object::new(self.contract, source.child(&i.to_string()));
                if let Some(identity) = parameter.string("name").zip(parameter.string("in"))
                    && !names.insert(identity)
                {
                    self.error(
                        parameter.source,
                        "DUPLICATE_PARAMETER",
                        "parameters at the same level must be unique by (name, in)",
                    );
                }
            }
            self.parameter_set(
                &(0..values.len())
                    .map(|i| source.child(&i.to_string()))
                    .collect::<Vec<_>>(),
            );
        }
    }
    fn parameter(&mut self, object: &Object<'_>, kind: Kind) {
        self.strings(object, &["description"]);
        self.booleans(
            object,
            &[
                "required",
                "deprecated",
                "allowEmptyValue",
                "explode",
                "allowReserved",
            ],
        );
        let location = if kind == Kind::Header {
            Some(ParameterLocation::Header)
        } else {
            self.required_string(object, "name");
            self.required_string(object, "in");
            let location = Parameter {
                object: object.clone(),
            }
            .location();
            if object.field("in").is_some() && location.is_none() {
                self.error(
                    object.field("in").unwrap().0,
                    "UNSUPPORTED_PARAMETER_LOCATION",
                    "parameter location must be query, header, path, cookie, or (in OpenAPI 3.2) querystring",
                );
            }
            if location == Some(ParameterLocation::Path) && object.boolean("required") != Some(true)
            {
                self.error(
                    object
                        .field("required")
                        .map_or_else(|| object.source.clone(), |(s, _)| s),
                    "INVALID_PATH_PARAMETER",
                    "path parameters must declare required: true",
                );
            }
            location
        };
        if location == Some(ParameterLocation::Querystring) {
            self.required_field(object, "content");
            for key in ["schema", "style", "explode", "allowReserved"] {
                if let Some((source, _)) = object.field(key) {
                    self.error(
                        source,
                        "INVALID_QUERYSTRING_PARAMETER",
                        format!("querystring parameters use content; {key} must not be present"),
                    );
                }
            }
        }
        if object.field("allowEmptyValue").is_some() && location != Some(ParameterLocation::Query) {
            self.error(
                object.field("allowEmptyValue").unwrap().0,
                "INVALID_PARAMETER_FIELD",
                "allowEmptyValue is valid only for in: query parameters",
            );
        }
        self.style(object, location);
        self.exclusive(object, "schema", "content");
        if object.field("schema").is_none() && object.field("content").is_none() {
            self.error(
                object.source.clone(),
                "INVALID_HTTP_SHAPE",
                "parameter/header requires schema or content",
            );
        }
        if let Some((source, value)) = object.field("content")
            && let Some(map) = value.as_object()
            && map.len() != 1
        {
            self.error(
                source,
                "INVALID_PARAMETER_CONTENT",
                "parameter/header content must contain exactly one media type",
            );
        }
        self.map(object, "content", Kind::Media, false);
        self.examples(object);
    }
    fn parameter_set(&mut self, sources: &[SourceId]) {
        let parameters: Vec<_> = sources
            .iter()
            .map(|source| Parameter {
                object: Object::new(self.contract, source.clone()),
            })
            .collect();
        let query = parameters
            .iter()
            .any(|parameter| parameter.location() == Some(ParameterLocation::Query));
        let querystrings: Vec<_> = parameters
            .iter()
            .filter(|parameter| parameter.location() == Some(ParameterLocation::Querystring))
            .collect();
        for parameter in querystrings.iter().skip(1) {
            self.error(parameter.source().clone(), "DUPLICATE_QUERYSTRING_PARAMETER",
                "an operation/path item can have only one effective querystring parameter, regardless of name");
        }
        if query {
            for parameter in querystrings {
                self.error(parameter.source().clone(), "INVALID_QUERYSTRING_COMBINATION",
                    "querystring and query parameters must not coexist, including inherited path-item parameters");
            }
        }
    }
    fn style(&mut self, object: &Object<'_>, location: Option<ParameterLocation>) {
        let Some((source, _)) = object.field("style") else {
            return;
        };
        let style = object.style();
        let valid = match location {
            Some(ParameterLocation::Path) => matches!(
                style,
                Some(ParameterStyle::Matrix | ParameterStyle::Label | ParameterStyle::Simple)
            ),
            Some(ParameterLocation::Header) => style == Some(ParameterStyle::Simple),
            Some(ParameterLocation::Cookie) => {
                matches!(style, Some(ParameterStyle::Form | ParameterStyle::Cookie))
            }
            Some(ParameterLocation::Query) => matches!(
                style,
                Some(
                    ParameterStyle::Form
                        | ParameterStyle::SpaceDelimited
                        | ParameterStyle::PipeDelimited
                        | ParameterStyle::DeepObject
                )
            ),
            Some(ParameterLocation::Querystring) => false,
            None => style.is_some(),
        };
        if !valid {
            self.error(
                source,
                "INVALID_PARAMETER_STYLE",
                "style is unknown or invalid for this parameter location",
            );
        }
    }
    fn server(&mut self, object: &Object<'_>) {
        self.required_string(object, "url");
        self.strings(object, &["description"]);
        if self.is_32(object) {
            self.strings(object, &["name"]);
        }
        self.map(object, "variables", Kind::Variable, false);
        if let Some(url) = object.string("url") {
            let variables = object.field("variables").and_then(|(_, v)| v.as_object());
            for part in url.split('{').skip(1) {
                let Some((name, _)) = part.split_once('}') else {
                    self.error(
                        object.field("url").unwrap().0,
                        "INVALID_SERVER_URL",
                        "server URL has an unterminated variable",
                    );
                    continue;
                };
                if !variables.is_some_and(|map| map.contains_key(name)) {
                    self.error(
                        object.field("url").unwrap().0,
                        "INVALID_SERVER_VARIABLE",
                        format!("server URL variable {name:?} has no declaration"),
                    );
                }
            }
        }
    }
    fn security(&mut self, object: &Object<'_>) {
        let Some((source, value)) = object.field("security") else {
            return;
        };
        let Some(values) = value.as_array() else {
            self.error(source, "INVALID_HTTP_SHAPE", "security must be an array");
            return;
        };
        for (i, value) in values.iter().enumerate() {
            let requirement = source.child(&i.to_string());
            let Some(map) = value.as_object() else {
                self.error(
                    requirement,
                    "INVALID_HTTP_SHAPE",
                    "security requirement must be an object",
                );
                continue;
            };
            for (name, scopes) in map {
                let source = requirement.child(name);
                let root = SourceId::new(source.document().clone(), suspect_low::Pointer::root());
                let scheme_source = root
                    .child("components")
                    .child("securitySchemes")
                    .child(name);
                if self.contract.source(&scheme_source).is_none() {
                    self.error(
                        source.clone(),
                        "UNKNOWN_SECURITY_SCHEME",
                        format!("security scheme {name:?} is not declared in this document"),
                    );
                } else {
                    self.visit(scheme_source, Kind::SecurityScheme);
                }
                if !scopes
                    .as_array()
                    .is_some_and(|array| array.iter().all(Value::is_string))
                {
                    self.error(
                        source,
                        "INVALID_HTTP_SHAPE",
                        "security scopes must be an array of strings",
                    );
                }
            }
        }
    }
    fn security_scheme(&mut self, object: &Object<'_>) {
        self.required_string(object, "type");
        self.strings(
            object,
            &[
                "description",
                "name",
                "in",
                "scheme",
                "bearerFormat",
                "openIdConnectUrl",
            ],
        );
        if self.is_32(object) {
            self.strings(object, &["oauth2MetadataUrl"]);
            self.booleans(object, &["deprecated"]);
        }
        match object.string("type") {
            Some("apiKey") => {
                self.required_string(object, "name");
                self.required_string(object, "in");
                if !matches!(object.string("in"), Some("query" | "header" | "cookie")) {
                    self.error(
                        object
                            .field("in")
                            .map_or_else(|| object.source.clone(), |(s, _)| s),
                        "INVALID_SECURITY_SCHEME",
                        "API key location must be query, header, or cookie",
                    );
                }
            }
            Some("http") => self.required_string(object, "scheme"),
            Some("openIdConnect") => self.required_string(object, "openIdConnectUrl"),
            Some("mutualTLS")
                if !self
                    .contract
                    .openapi_version_at(&object.source)
                    .starts_with("3.0.") => {}
            Some("oauth2") => {
                self.required_field(object, "flows");
                self.object_field(object, "flows");
                if let Some((source, value)) = object.field("flows")
                    && let Some(flows) = value.as_object()
                {
                    for (name, value) in flows.iter().filter(|(k, _)| !k.starts_with("x-")) {
                        let flow = Object::new(self.contract, source.child(name));
                        if !matches!(
                            name.as_str(),
                            "implicit" | "password" | "clientCredentials" | "authorizationCode"
                        ) && !(name == "deviceAuthorization" && self.is_32(object))
                        {
                            self.error(
                                flow.source,
                                "UNSUPPORTED_OAUTH_FLOW",
                                format!("unsupported OAuth flow {name:?}"),
                            );
                            continue;
                        }
                        if !value.is_object() {
                            self.error(
                                flow.source,
                                "INVALID_HTTP_SHAPE",
                                "OAuth flow must be an object",
                            );
                            continue;
                        }
                        let mut fields =
                            vec!["authorizationUrl", "tokenUrl", "refreshUrl", "scopes"];
                        if self.is_32(object) {
                            fields.push("deviceAuthorizationUrl");
                        }
                        self.fixed_fields(&flow, "OAuthFlow", &fields);
                        if matches!(name.as_str(), "implicit" | "authorizationCode") {
                            self.required_string(&flow, "authorizationUrl");
                        }
                        if name != "implicit" {
                            self.required_string(&flow, "tokenUrl");
                        }
                        if name == "deviceAuthorization" {
                            self.required_string(&flow, "deviceAuthorizationUrl");
                        }
                        self.strings(&flow, &["authorizationUrl", "tokenUrl", "refreshUrl"]);
                        if self.is_32(object) {
                            self.strings(&flow, &["deviceAuthorizationUrl"]);
                        }
                        self.required_field(&flow, "scopes");
                        self.object_field(&flow, "scopes");
                        if let Some((source, value)) = flow.field("scopes")
                            && let Some(scopes) = value.as_object()
                        {
                            for (name, description) in scopes {
                                if !description.is_string() {
                                    self.error(
                                        source.child(name),
                                        "INVALID_HTTP_SHAPE",
                                        "OAuth scope descriptions must be strings",
                                    );
                                }
                            }
                        }
                    }
                }
            }
            Some(_) => self.error(
                object.field("type").unwrap().0,
                "UNSUPPORTED_SECURITY_SCHEME",
                "unsupported security scheme type",
            ),
            None => {}
        }
    }
}
