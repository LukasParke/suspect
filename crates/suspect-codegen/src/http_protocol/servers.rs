use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;
use suspect_ir::contract::SourceId;
use url::Url;

use super::planner::Planner;
use super::*;

pub(super) fn plan(
    p: &mut Planner<'_>,
    source: Option<&SourceId>,
    operation: &SourceId,
) -> ServersPlan {
    let anchor = source.unwrap_or(operation);
    let candidates = match source.and_then(|s| p.contract.source(s)) {
        None => vec![default_server(p, anchor, false)],
        Some(Value::Array(entries)) if entries.is_empty() => vec![default_server(p, anchor, true)],
        Some(Value::Array(entries)) => {
            if entries.len() > 1 {
                p.require(Capability::MultipleServers, anchor);
            }
            let mut names = BTreeSet::new();
            let mut result = Vec::new();
            for (index, _) in entries.iter().enumerate() {
                if let Some(server) = server(p, &anchor.child(&index.to_string())) {
                    if let Some(name) = &server.name
                        && !names.insert(name.value.clone())
                    {
                        p.error(
                            &name.source.source,
                            "http-server-name-duplicate",
                            "server names must be unique in the effective server array",
                        );
                    }
                    result.push(server);
                }
            }
            result
        }
        Some(_) => {
            p.error(
                anchor,
                "http-servers-invalid",
                "effective servers must be an array",
            );
            Vec::new()
        }
    };
    ServersPlan {
        source: p.location(anchor),
        candidates,
    }
}

fn default_server(p: &mut Planner<'_>, source: &SourceId, explicit: bool) -> ServerPlan {
    p.require(Capability::RelativeServers, source);
    p.require(Capability::DocumentRelativeServers, source);
    let document = if explicit {
        source.document()
    } else {
        p.contract.entry()
    };
    let document_base = p.location(&SourceId::new(document.clone(), Default::default()));
    ServerPlan {
        source: None,
        default_from: Some(p.location(source)),
        template: "/".to_owned(),
        description: None,
        name: None,
        variables: Vec::new(),
        document_base,
    }
}

pub(super) fn server(p: &mut Planner<'_>, source: &SourceId) -> Option<ServerPlan> {
    let raw = p.object(source, "Server")?;
    p.known(source, raw, &["url", "description", "variables", "name"]);
    let url = p.string(source, "url", true)?;
    let description = p.string(source, "description", false);
    let name = p.string(source, "name", false);
    if name.is_some() && !p.is_32(source) {
        p.unsupported(
            &source.child("name"),
            "http-version-field",
            "Server.name requires OAS 3.2",
        );
    }
    let names = match placeholders(&url.value) {
        Ok(names) => names,
        Err(reason) => {
            p.error(&source.child("url"), "http-server-template", reason);
            return None;
        }
    };
    let mut variables = Vec::new();
    if let Some(value) = raw.get("variables") {
        let at = source.child("variables");
        if let Some(map) = value.as_object() {
            if !map.is_empty() {
                p.require(Capability::ServerVariables, &at);
            }
            for (key, _) in map {
                let source = at.child(key);
                let Some(raw) = p.object(&source, "Server Variable") else {
                    continue;
                };
                p.known(&source, raw, &["default", "enum", "description"]);
                if key.is_empty() || key.contains(['{', '}']) {
                    p.error(
                        &source,
                        "http-server-variable-name",
                        "server variable names must be nonempty and cannot contain braces",
                    );
                }
                let Some(default) = p.string(&source, "default", true) else {
                    continue;
                };
                let values = p.strings(&source, "enum", false);
                if let Some(values) = &values {
                    if values.is_empty() {
                        p.error(
                            &source.child("enum"),
                            "http-server-variable-enum",
                            "server variable enum must not be empty",
                        );
                    }
                    if !values.iter().any(|v| v.value == default.value) {
                        p.error(
                            &source.child("default"),
                            "http-server-variable-default",
                            "server variable default must be one of its enum values",
                        );
                    }
                }
                let description = p.string(&source, "description", false);
                variables.push(ServerVariable {
                    source: p.inline(&source),
                    name: key.clone(),
                    default,
                    values,
                    description,
                });
            }
        } else {
            p.error(
                &at,
                "http-server-variables-invalid",
                "server variables must be a map of Server Variable Objects",
            );
        }
    }
    for name in names {
        if !variables.iter().any(|v| v.name == name) {
            p.error(
                &source.child("url"),
                "http-server-variable-missing",
                format!("server template variable {name:?} has no declared default"),
            );
        }
    }
    let result = ServerPlan {
        source: Some(p.inline(source)),
        default_from: None,
        template: url.value,
        description,
        name,
        variables,
        document_base: p.location(&SourceId::new(
            source.document().clone(),
            Default::default(),
        )),
    };
    // A whole/scheme-variable URL can produce a relative override even when its
    // default is absolute. Do not widen a native adapter through that default.
    if !result.variables.is_empty()
        && !result.template.to_ascii_lowercase().starts_with("https://")
        && !result.template.to_ascii_lowercase().starts_with("http://")
    {
        p.require(Capability::RelativeServers, source);
        p.require(Capability::DocumentRelativeServers, source);
    }
    match result.expand(&BTreeMap::new()) {
        Ok(value) => match Url::parse(&value) {
            Ok(url) if url.scheme() == "https" => {}
            Ok(url) if url.scheme() == "http" => {
                p.require(Capability::HttpServers, source);
            }
            Ok(_) => p.unsupported(
                &source.child("url"),
                "http-server-scheme",
                "HTTP SDK server URLs require http or https",
            ),
            Err(url::ParseError::RelativeUrlWithoutBase) => {
                p.require(Capability::RelativeServers, source);
                p.require(Capability::DocumentRelativeServers, source);
            }
            Err(_) => p.error(
                &source.child("url"),
                "http-server-url",
                "server URL is invalid",
            ),
        },
        Err(error) => p.error(error.source().source(), error.code(), error.message()),
    }
    Some(result)
}

impl ServerPlan {
    /// OAS 3.2 §4.5: the default relative-server base is the effective physical
    /// retrieval URL serving its document. `$self`/`$id` do not relocate an API.
    /// Local-file sources require an explicit HTTP base via `resolve_url`.
    pub fn resolve_document_url(
        &self,
        overrides: &BTreeMap<String, String>,
    ) -> Result<String, WireError> {
        self.resolve_url(self.document_base.source.document().as_str(), overrides)
    }
    /// Literal OAS variable substitution with default/enum/unknown-key checks.
    /// Values are not URI-template expanded or silently percent-encoded.
    pub fn expand(&self, overrides: &BTreeMap<String, String>) -> Result<String, WireError> {
        let anchor = self
            .source
            .as_ref()
            .map(|v| &v.terminal)
            .or(self.default_from.as_ref())
            .expect("server has source or default provenance");
        for name in overrides.keys() {
            if !self.variables.iter().any(|v| &v.name == name) {
                return Err(WireError::new(
                    anchor,
                    "http-server-override-unknown",
                    format!("undeclared server variable {name:?}"),
                ));
            }
        }
        let mut result = String::new();
        let mut rest = self.template.as_str();
        while let Some(open) = rest.find('{') {
            result.push_str(&rest[..open]);
            let tail = &rest[open + 1..];
            let close = tail.find('}').ok_or_else(|| {
                WireError::new(
                    anchor,
                    "http-server-template",
                    "unclosed template expression",
                )
            })?;
            let name = &tail[..close];
            let variable = self
                .variables
                .iter()
                .find(|v| v.name == name)
                .ok_or_else(|| {
                    WireError::new(
                        anchor,
                        "http-server-variable-missing",
                        format!("undeclared template variable {name:?}"),
                    )
                })?;
            let value = overrides.get(name).unwrap_or(&variable.default.value);
            if variable
                .values
                .as_ref()
                .is_some_and(|values| !values.iter().any(|v| &v.value == value))
            {
                return Err(WireError::new(
                    &variable.source.terminal,
                    "http-server-override-enum",
                    "server variable override is not a declared enum value",
                ));
            }
            result.push_str(value);
            rest = &tail[close + 1..];
        }
        result.push_str(rest);
        validate_url(&result)
            .map_err(|reason| WireError::new(anchor, "http-server-url", reason))?;
        Ok(result)
    }

    /// Resolve a relative server against the URL serving the document, not its
    /// `$self` or a schema `$id`. Local-file specs require a caller-supplied HTTP
    /// document URL; the planner never invents a network host for them.
    pub fn resolve_url(
        &self,
        document_url: &str,
        overrides: &BTreeMap<String, String>,
    ) -> Result<String, WireError> {
        let anchor = self
            .source
            .as_ref()
            .map(|v| &v.terminal)
            .or(self.default_from.as_ref())
            .expect("server provenance");
        let expanded = self.expand(overrides)?;
        let base = match Url::parse(&expanded) {
            Ok(_) => "https://unused.invalid/", // Absolute server ignores the caller base.
            Err(url::ParseError::RelativeUrlWithoutBase) => {
                let base = Url::parse(document_url).map_err(|_| {
                    WireError::new(
                        anchor,
                        "http-server-base-url",
                        "relative server needs an absolute HTTP document URL",
                    )
                })?;
                if !matches!(base.scheme(), "http" | "https") {
                    return Err(WireError::new(
                        anchor,
                        "http-server-base-url",
                        "relative server needs an HTTP document URL",
                    ));
                }
                document_url
            }
            Err(_) => {
                return Err(WireError::new(
                    anchor,
                    "http-server-url",
                    "invalid expanded server URL",
                ));
            }
        };
        // Use generic RFC 3986 resolution: encoded dot segments, path case and
        // encoded slashes are data. A browser URL parser may repair/normalize
        // those values, which would change the source-backed server identity.
        let resolved = suspect_ir::contract::resource_uri::resolve_document(base, &expanded)
            .map_err(|error| WireError::new(anchor, "http-server-base-url", error.to_string()))?;
        let parsed = Url::parse(&resolved).map_err(|_| {
            WireError::new(
                anchor,
                "http-server-url",
                "resolved server is not an HTTP URL",
            )
        })?;
        if !matches!(parsed.scheme(), "http" | "https") || !parsed.has_host() {
            return Err(WireError::new(
                anchor,
                "http-server-scheme",
                "server requires http or https",
            ));
        }
        Ok(resolved)
    }
}

pub(super) fn placeholders(value: &str) -> Result<Vec<&str>, &'static str> {
    let mut result = Vec::new();
    let mut open = None;
    for (i, c) in value.char_indices() {
        match c {
            '{' if open.is_some() => return Err("nested template braces are invalid"),
            '{' => open = Some(i + 1),
            '}' => {
                let start = open.take().ok_or("unmatched closing template brace")?;
                if start == i {
                    return Err("template variable name must not be empty");
                }
                result.push(&value[start..i]);
            }
            _ => {}
        }
    }
    if open.is_some() {
        return Err("unclosed template expression");
    }
    Ok(result)
}

fn validate_url(value: &str) -> Result<(), &'static str> {
    if value.is_empty()
        || value.contains(['?', '#', '\\', '{', '}'])
        || value.chars().any(|c| c.is_whitespace() || c.is_control())
    {
        return Err(
            "server URL must not contain whitespace, controls, query, fragment or unsubstituted braces",
        );
    }
    if !percent_triples(value) {
        return Err("server URL contains an invalid percent escape");
    }
    suspect_ir::contract::resource_uri::split_reference(value)
        .map_err(|_| "server URL is not a valid RFC 3986 URL reference")?;
    let url = Url::parse(value)
        .or_else(|_| Url::parse("https://relative.invalid/").and_then(|base| base.join(value)))
        .map_err(|_| "server URL is not a valid URL reference")?;
    if matches!(url.scheme(), "http" | "https") && !url.has_host() {
        return Err("HTTP server URL requires a host");
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err("server URL userinfo is not a credential hook");
    }
    Ok(())
}

pub(super) fn percent_triples(value: &str) -> bool {
    let bytes = value.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            if bytes.get(i + 1).is_none_or(|b| !b.is_ascii_hexdigit())
                || bytes.get(i + 2).is_none_or(|b| !b.is_ascii_hexdigit())
            {
                return false;
            }
            i += 3;
        } else {
            i += 1;
        }
    }
    true
}
