//! JSON Schemas (draft 2020-12) for `.suspect.yaml` and
//! `suspect.project.json`, generated from the setting table.
//!
//! The checked-in copies under `editors/vscode/schemas/` are kept current
//! by `tests/json_schema.rs` (`SUSPECT_BLESS=1` rewrites them). Each
//! property carries `description` and `default` from the table, `enum`
//! from its kind, `x-suspect-editor` (the client setting and its scope)
//! and `x-suspect-env` where the table names them. Deprecated spellings
//! are listed with `deprecated: true` so existing files still validate.

use serde_json::{Value, json};

use crate::schema::{Kind, MANIFEST, SETTINGS, Setting, TARGET_FIELDS};

const DRAFT: &str = "https://json-schema.org/draft/2020-12/schema";

/// The schema for `.suspect.yaml`.
#[must_use]
pub fn settings() -> Value {
    let mut root = object_schema(&SETTINGS[0]);
    root["$schema"] = json!(DRAFT);
    root["title"] = json!(".suspect.yaml");
    fill(&mut root, SETTINGS, "");
    aliases(&mut root, SETTINGS);
    root
}

/// The schema for `suspect.project.json`.
#[must_use]
pub fn manifest() -> Value {
    let mut root = object_schema(&MANIFEST[0]);
    root["$schema"] = json!(DRAFT);
    root["title"] = json!("suspect.project.json");
    root["required"] = json!(["version", "entry"]);
    fill(&mut root, MANIFEST, "");
    for section in ["lint", "validate", "editor"] {
        root["properties"][section]["deprecated"] = json!(true);
    }
    root
}

/// Adds every row directly under `scope` (and, recursively, their
/// children) to `node`'s `properties`.
fn fill(node: &mut Value, table: &'static [Setting], scope: &str) {
    let prefix = if scope.is_empty() {
        String::new()
    } else {
        format!("{scope}.")
    };
    for setting in table {
        let Some(name) = setting.path.strip_prefix(&prefix) else {
            continue;
        };
        if name.is_empty() || name.contains('.') {
            continue;
        }
        let mut property = property(setting);
        if setting.kind == Kind::Section {
            fill(&mut property, table, setting.path);
        }
        node["properties"][name] = property;
    }
}

/// Places each deprecated spelling where it is written: a section alias
/// as a copy of the section, a key alias as a copy of its row. Shorter
/// spellings first, so `inlay_hints` exists before `inlay_hints.refs`.
fn aliases(root: &mut Value, table: &'static [Setting]) {
    let mut all: Vec<(&'static str, &'static Setting)> = table
        .iter()
        .flat_map(|s| s.aliases.iter().map(move |a| (*a, s)))
        .collect();
    all.sort_by_key(|(alias, _)| alias.matches('.').count());
    for (alias, setting) in all {
        let mut property = if setting.kind == Kind::Section {
            at(root, setting.path)
                .cloned()
                .unwrap_or_else(|| property(setting))
        } else {
            property(setting)
        };
        property["deprecated"] = json!(true);
        property["description"] = json!(format!(
            "Deprecated spelling of `{}`. {}",
            setting.path, setting.doc
        ));
        let (parent, name) = alias.rsplit_once('.').unwrap_or(("", alias));
        let Some(parent) = at_mut(root, parent) else {
            continue;
        };
        parent["properties"][name] = property;
    }
}

fn at<'v>(root: &'v Value, path: &str) -> Option<&'v Value> {
    let mut node = root;
    for part in path.split('.').filter(|p| !p.is_empty()) {
        node = node.get("properties")?.get(part)?;
    }
    Some(node)
}

fn at_mut<'v>(root: &'v mut Value, path: &str) -> Option<&'v mut Value> {
    let mut node = root;
    for part in path.split('.').filter(|p| !p.is_empty()) {
        node = node.get_mut("properties")?.get_mut(part)?;
    }
    Some(node)
}

fn object_schema(setting: &Setting) -> Value {
    json!({
        "type": "object",
        "description": setting.doc,
        "additionalProperties": false,
        "properties": {},
    })
}

fn property(setting: &Setting) -> Value {
    let mut out = match setting.kind {
        Kind::Section => object_schema(setting),
        Kind::Targets => {
            let mut target = json!({
                "type": "object",
                "additionalProperties": false,
                "required": ["profile", "package_name", "package_version"],
                "properties": {},
            });
            for field in TARGET_FIELDS {
                target["properties"][field.path] = property(field);
            }
            json!({"type": "array", "items": target})
        }
        kind => kind_schema(kind),
    };
    out["description"] = json!(setting.doc);
    if let Some(default) = default_value(setting) {
        out["default"] = default;
    }
    if let Some(key) = setting.editor {
        out["x-suspect-editor"] = json!({"key": format!("suspect.{key}"), "scope": "resource"});
    }
    if let Some(var) = setting.env {
        out["x-suspect-env"] = json!(var);
    }
    out
}

fn kind_schema(kind: Kind) -> Value {
    match kind {
        Kind::Bool => json!({"type": "boolean"}),
        Kind::Str | Kind::Path => json!({"type": "string"}),
        Kind::Url => json!({"type": "string", "format": "uri"}),
        Kind::Port => json!({"type": "integer", "minimum": 0, "maximum": 65535}),
        Kind::Count => json!({"type": "integer", "minimum": 0}),
        Kind::Enum(_) | Kind::Severity | Kind::PolicySeverity => {
            json!({"type": "string", "enum": kind.values()})
        }
        Kind::Map(inner) => json!({"type": "object", "additionalProperties": kind_schema(*inner)}),
        Kind::List => json!({"type": "array", "items": {"type": "string"}}),
        Kind::Object | Kind::Section | Kind::Targets => json!({"type": "object"}),
    }
}

/// The table's default as JSON, when it is a literal (not prose such as
/// "the directory name").
fn default_value(setting: &Setting) -> Option<Value> {
    let text = setting.default?;
    if let Ok(value) = serde_json::from_str::<Value>(text) {
        return Some(value);
    }
    let literal = !text.contains(' ');
    let stringy = matches!(
        setting.kind,
        Kind::Str | Kind::Path | Kind::Url | Kind::Enum(_) | Kind::Severity | Kind::PolicySeverity
    );
    (literal && stringy).then(|| Value::String(text.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_carry_kinds_defaults_and_annotations() {
        let schema = settings();
        let lint = &schema["properties"]["lint"]["properties"];
        let floor = &lint["min_severity"];
        assert_eq!(floor["enum"], json!(["error", "warning", "info", "hint"]));
        assert_eq!(floor["default"], "hint");
        assert_eq!(floor["x-suspect-env"], "SUSPECT_LINT_MIN_SEVERITY");
        assert_eq!(floor["x-suspect-editor"]["key"], "suspect.lint.minSeverity");
        assert_eq!(
            lint["rules"]["additionalProperties"]["enum"],
            json!(["error", "warning", "info", "hint", "off"])
        );
        assert_eq!(
            schema["properties"]["ref"]["properties"]["max_docs"]["default"],
            500
        );
        assert_eq!(schema["additionalProperties"], false);
    }

    #[test]
    fn deprecated_spellings_still_validate() {
        let schema = settings();
        let p = &schema["properties"];
        assert_eq!(p["formatting"]["deprecated"], true);
        assert_eq!(
            p["formatting"]["properties"]["sort_keys"]["deprecated"],
            true
        );
        assert_eq!(
            p["inlay_hints"]["properties"]["ref_targets"]["deprecated"],
            true
        );
        assert_eq!(p["inlay_hints"]["properties"]["refs"]["deprecated"], true);
        assert_eq!(p["refs"]["properties"]["maxDocs"]["deprecated"], true);
        assert_eq!(p["ref"]["properties"]["maxDocs"]["deprecated"], true);
        assert_eq!(p["docs"]["properties"]["out"]["deprecated"], true);
        assert!(
            p["run"].is_null(),
            "an alias without its section is not invented"
        );
    }

    #[test]
    fn the_manifest_schema_closes_codegen_targets() {
        let schema = manifest();
        let target = &schema["properties"]["codegen"]["items"];
        assert_eq!(target["additionalProperties"], false);
        assert!(target["properties"]["sdk_defaults"].is_object());
        assert!(
            schema["properties"]["name"].get("default").is_none(),
            "prose is not a default"
        );
        assert_eq!(schema["required"], json!(["version", "entry"]));
        assert_eq!(schema["properties"]["lint"]["deprecated"], true);
    }
}
