//! Layers: one source of settings each (a file, the manifest, the
//! environment, an editor, the command line), every value carrying where
//! it came from.
//!
//! Values are checked against the setting table as they enter a layer, so
//! a bad value is reported once with its origin and dropped, and the merge
//! in [`crate::resolve()`] is plain insertion. Map settings are flattened to
//! `lint.rules.<id>`, so every merge is per key by construction.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use suspect_low::{NodeRef, ValueKind};

use crate::schema::{self, Kind, Setting};

/// Where a value came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Origin {
    /// The built-in default.
    Default,
    /// A settings file (`.suspect.yaml:12`).
    File {
        /// The file.
        path: PathBuf,
        /// One-based line.
        line: u32,
    },
    /// A project manifest's deprecated policy sections.
    Manifest {
        /// The manifest.
        path: PathBuf,
        /// One-based line.
        line: u32,
    },
    /// An environment variable.
    Env {
        /// Variable name.
        var: &'static str,
    },
    /// An editor (client) setting.
    Editor {
        /// The key as the client spelled it: `suspect.lint.minSeverity`.
        key: String,
    },
    /// A command-line flag.
    Flag {
        /// The flag: `--min-severity`.
        name: &'static str,
    },
}

impl Origin {
    /// The layer name: `default`, `file`, `manifest`, `env`, `editor`, `flag`.
    #[must_use]
    pub fn layer(&self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::File { .. } => "file",
            Self::Manifest { .. } => "manifest",
            Self::Env { .. } => "env",
            Self::Editor { .. } => "editor",
            Self::Flag { .. } => "flag",
        }
    }

    /// The file this origin points into, if any.
    #[must_use]
    pub fn path(&self) -> Option<&Path> {
        match self {
            Self::File { path, .. } | Self::Manifest { path, .. } => Some(path),
            _ => None,
        }
    }
}

impl std::fmt::Display for Origin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Default => write!(f, "default"),
            Self::File { path, line } | Self::Manifest { path, line } => {
                write!(f, "{}:{line}", shown(path).display())
            }
            Self::Env { var } => write!(f, "env {var}"),
            Self::Editor { key } => write!(f, "editor {key}"),
            Self::Flag { name } => write!(f, "flag {name}"),
        }
    }
}

/// `path` relative to the working directory when it sits under it.
#[must_use]
pub fn shown(path: &Path) -> PathBuf {
    std::env::current_dir()
        .ok()
        .and_then(|cwd| path.strip_prefix(cwd).ok().map(Path::to_path_buf))
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| path.to_path_buf())
}

/// A raw setting value, already checked against its [`Kind`].
#[derive(Debug, Clone, PartialEq)]
pub enum Raw {
    /// A boolean.
    Bool(bool),
    /// Text (strings, enumeration and severity names, URLs).
    Str(String),
    /// A whole number (counts, ports).
    Int(u64),
    /// A path, already resolved against the declaring file's directory.
    Path(PathBuf),
    /// A list of strings.
    List(Vec<String>),
    /// An opaque section, as JSON.
    Json(serde_json::Value),
}

impl std::fmt::Display for Raw {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Bool(b) => write!(f, "{b}"),
            Self::Str(s) => write!(f, "{s}"),
            Self::Int(n) => write!(f, "{n}"),
            Self::Path(p) => write!(f, "{}", p.display()),
            Self::List(items) => write!(f, "[{}]", items.join(", ")),
            Self::Json(value) => write!(f, "{value}"),
        }
    }
}

impl Raw {
    /// The value as JSON, for `--explain --format json`.
    #[must_use]
    pub fn to_json(&self) -> serde_json::Value {
        match self {
            Self::Bool(b) => serde_json::Value::Bool(*b),
            Self::Str(s) => serde_json::Value::String(s.clone()),
            Self::Int(n) => serde_json::Value::from(*n),
            Self::Path(p) => serde_json::Value::String(p.display().to_string()),
            Self::List(items) => serde_json::Value::from(items.clone()),
            Self::Json(value) => value.clone(),
        }
    }
}

/// A problem found while reading configuration, with its origin.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notice {
    /// Where the problem is.
    pub origin: Origin,
    /// What is wrong.
    pub message: String,
}

impl std::fmt::Display for Notice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.origin, self.message)
    }
}

/// One layer: setting path -> checked value with its origin.
#[derive(Debug, Clone, Default)]
pub struct Layer {
    pub(crate) entries: BTreeMap<String, (Raw, Origin)>,
    /// Values rejected while building the layer.
    pub(crate) errors: Vec<Notice>,
    /// Deprecated spellings and unknown keys.
    pub(crate) warnings: Vec<Notice>,
    /// Sections the layer's file wrote (`format`), for defaults that
    /// depend on a section being present.
    pub(crate) sections: Vec<&'static str>,
}

impl Layer {
    /// An empty flag layer, filled with [`Layer::set`].
    #[must_use]
    pub fn flags() -> Self {
        Self::default()
    }

    /// Sets `path` from a flag. `None` means "not passed" and adds nothing.
    ///
    /// # Panics
    /// When `path` is not a row in the setting table: a programming error.
    #[must_use]
    pub fn set(mut self, path: &'static str, value: Option<Raw>, flag: &'static str) -> Self {
        let setting = schema::lookup(path)
            .unwrap_or_else(|| panic!("`{path}` is not a setting"))
            .setting;
        if let Some(value) = value {
            let origin = Origin::Flag { name: flag };
            match check_raw(setting, value, None) {
                Ok(raw) => {
                    self.entries.insert(path.to_owned(), (raw, origin));
                }
                Err(message) => self.errors.push(Notice { origin, message }),
            }
        }
        self
    }

    /// The entries, for tests and tools.
    pub fn entries(&self) -> impl Iterator<Item = (&str, &Raw, &Origin)> {
        self.entries
            .iter()
            .map(|(k, (raw, origin))| (k.as_str(), raw, origin))
    }

    /// The value at `path`, if the layer sets it.
    #[must_use]
    pub fn get(&self, path: &str) -> Option<&Raw> {
        self.entries.get(path).map(|(raw, _)| raw)
    }

    /// A client `suspect` section (initializationOptions or
    /// workspace/configuration), camelCase or snake_case keys. Blank
    /// strings and `null` are "not set": VS Code reports an untouched
    /// string setting as `""`. Relative paths resolve against `base`.
    #[must_use]
    pub fn from_editor(section: &serde_json::Value, base: &Path) -> Self {
        let section = section.get("suspect").unwrap_or(section);
        let mut layer = Self::default();
        if let Some(object) = section.as_object() {
            layer.editor_object(object, "", base);
        }
        layer
    }

    fn editor_object(
        &mut self,
        object: &serde_json::Map<String, serde_json::Value>,
        prefix: &str,
        base: &Path,
    ) {
        // An unknown client key is skipped silently: clients send their own
        // settings (`basePath`, `server.path`) in the same section.
        for (key, value) in object {
            let raw_key = join(prefix, key);
            let origin = Origin::Editor {
                key: format!("suspect.{raw_key}"),
            };
            let found = schema::lookup_editor(&raw_key);
            match (found, value) {
                (Some(found), serde_json::Value::Object(map))
                    if matches!(found.setting.kind, Kind::Map(_)) =>
                {
                    for (id, value) in map {
                        let inner = match found.setting.kind {
                            Kind::Map(inner) => *inner,
                            _ => unreachable!(),
                        };
                        if let Some(raw) = json_scalar(value) {
                            let path = format!("{}.{id}", found.setting.path);
                            self.insert_kind(&path, inner, raw, origin.clone(), Some(base));
                        }
                    }
                }
                (Some(found), value) if found.setting.kind == Kind::Object => {
                    if !value.is_null() {
                        self.entries.insert(
                            found.setting.path.to_owned(),
                            (Raw::Json(value.clone()), origin),
                        );
                    }
                }
                (found, serde_json::Value::Object(map))
                    if found.is_none_or(|f| f.setting.kind == Kind::Section) =>
                {
                    self.editor_object(map, &raw_key, base);
                }
                (Some(found), value) => {
                    if let Some(raw) = json_scalar(value) {
                        self.insert(found.setting, raw, origin, Some(base));
                    }
                }
                (None, _) => {}
            }
        }
    }

    /// Reads a settings file or manifest subtree. `scope` is the settings
    /// path the node sits at; `rename` maps a manifest spelling onto the
    /// settings table (`editor.ref` -> `ref`).
    pub(crate) fn read_node(
        &mut self,
        node: &NodeRef<'_>,
        scope: &str,
        base: &Path,
        origin: &dyn Fn(u32) -> Origin,
    ) {
        if node.kind() != ValueKind::Object {
            return;
        }
        for entry in node.entries() {
            let path = join(scope, entry.key);
            let at = origin(entry.key_node.line_col().0 + 1);
            let Some(found) = schema::lookup(&path) else {
                self.warnings.push(Notice {
                    origin: at,
                    message: format!("`{path}` is not a .suspect.yaml key; ignored"),
                });
                continue;
            };
            if let Some(alias) = found.alias {
                self.warnings.push(Notice {
                    origin: at.clone(),
                    message: format!("`{alias}` is deprecated; use `{}`", found.setting.path),
                });
            }
            let Some(value) = entry.value.filter(|v| v.kind() != ValueKind::Null) else {
                if found.setting.kind == Kind::Section {
                    self.sections.push(found.setting.path);
                }
                continue;
            };
            match found.setting.kind {
                Kind::Section => {
                    self.sections.push(found.setting.path);
                    if value.kind() == ValueKind::Object {
                        self.read_node(&value, &path, base, origin);
                    } else {
                        self.errors.push(Notice {
                            origin: at,
                            message: format!("`{path}` must be a section"),
                        });
                    }
                }
                Kind::Map(inner) => {
                    if value.kind() != ValueKind::Object {
                        self.errors.push(Notice {
                            origin: at,
                            message: format!("`{}` must be a map", found.setting.path),
                        });
                        continue;
                    }
                    for item in value.entries() {
                        let Some(raw) = item.value.and_then(|v| node_scalar(&v)) else {
                            continue;
                        };
                        let key = format!("{}.{}", found.setting.path, item.key);
                        let at = origin(item.key_node.line_col().0 + 1);
                        self.insert_kind(&key, *inner, raw, at, Some(base));
                    }
                }
                Kind::Object | Kind::Targets => {
                    self.entries.insert(
                        found.setting.path.to_owned(),
                        (Raw::Json(node_json(&value)), at),
                    );
                }
                Kind::List => match value.kind() {
                    ValueKind::Array => {
                        let items = value
                            .items()
                            .iter()
                            .filter_map(|i| i.as_str().map(str::to_owned))
                            .collect();
                        self.insert(found.setting, Raw::List(items), at, Some(base));
                    }
                    _ => self.errors.push(Notice {
                        origin: at,
                        message: format!("`{}` must be a list", found.setting.path),
                    }),
                },
                _ => match node_scalar(&value) {
                    Some(raw) => self.insert(found.setting, raw, at, Some(base)),
                    None => self.errors.push(Notice {
                        origin: at,
                        message: format!(
                            "`{}` expects {}",
                            found.setting.path,
                            found.setting.kind.type_name()
                        ),
                    }),
                },
            }
        }
    }

    /// Reads the environment variables the table declares.
    pub(crate) fn from_env(env: &crate::Env) -> Self {
        let mut layer = Self::default();
        for (var, setting) in schema::env_vars() {
            if let Some(value) = env.get(var) {
                layer.insert(setting, Raw::Str(value), Origin::Env { var }, None);
            }
        }
        layer
    }

    fn insert(&mut self, setting: &'static Setting, raw: Raw, origin: Origin, base: Option<&Path>) {
        match check_raw(setting, raw, base) {
            Ok(checked) => {
                self.entries
                    .insert(setting.path.to_owned(), (checked, origin));
            }
            Err(message) if message.is_empty() => {}
            Err(message) => self.errors.push(Notice { origin, message }),
        }
    }

    fn insert_kind(
        &mut self,
        path: &str,
        kind: Kind,
        raw: Raw,
        origin: Origin,
        base: Option<&Path>,
    ) {
        match check_kind(path, kind, raw, base) {
            Ok(raw) => {
                self.entries.insert(path.to_owned(), (raw, origin));
            }
            Err(message) if message.is_empty() => {}
            Err(message) => self.errors.push(Notice { origin, message }),
        }
    }
}

fn join(scope: &str, key: &str) -> String {
    if scope.is_empty() {
        key.to_owned()
    } else {
        format!("{scope}.{key}")
    }
}

fn check_raw(setting: &Setting, raw: Raw, base: Option<&Path>) -> Result<Raw, String> {
    check_kind(setting.path, setting.kind, raw, base)
}

/// Checks and normalizes one value. `Err("")` means "blank: not set".
fn check_kind(path: &str, kind: Kind, raw: Raw, base: Option<&Path>) -> Result<Raw, String> {
    if let Raw::Str(s) = &raw
        && s.trim().is_empty()
    {
        return Err(String::new());
    }
    let text = |raw: &Raw| match raw {
        Raw::Str(s) => Some(s.trim().to_owned()),
        Raw::Int(n) => Some(n.to_string()),
        Raw::Bool(b) => Some(b.to_string()),
        Raw::Path(p) => Some(p.display().to_string()),
        Raw::List(_) | Raw::Json(_) => None,
    };
    let expected = || format!("`{path}` expects {}", kind.type_name());
    match kind {
        Kind::Bool => match &raw {
            Raw::Bool(_) => Ok(raw),
            Raw::Str(s) => match s.trim().to_ascii_lowercase().as_str() {
                "true" | "1" | "yes" | "on" => Ok(Raw::Bool(true)),
                "false" | "0" | "no" | "off" => Ok(Raw::Bool(false)),
                other => Err(format!("`{path}` expects a boolean, not `{other}`")),
            },
            _ => Err(expected()),
        },
        Kind::Count | Kind::Port => {
            let n = match &raw {
                Raw::Int(n) => Some(*n),
                Raw::Str(s) => s.trim().parse::<u64>().ok(),
                _ => None,
            };
            match n {
                Some(n) if kind == Kind::Port && n > 65535 => {
                    Err(format!("`{path}`: {n} is not a port (0-65535)"))
                }
                Some(n) => Ok(Raw::Int(n)),
                None => Err(expected()),
            }
        }
        Kind::Severity | Kind::PolicySeverity => {
            let name = text(&raw).ok_or_else(expected)?;
            match crate::parse_severity(&name) {
                Some(crate::SeverityName::Off) if kind == Kind::Severity => Err(format!(
                    "`{path}`: severity `off` is not a valid minimum severity"
                )),
                Some(parsed) => Ok(Raw::Str(parsed.label().to_owned())),
                None if kind == Kind::Severity => Err(format!(
                    "`{path}`: unrecognized severity `{name}` (expected error, warning, info, or hint)"
                )),
                None => Err(format!(
                    "`{path}`: unrecognized severity `{name}` (expected error, warning, info, hint, or off)"
                )),
            }
        }
        Kind::Enum(values) => {
            let name = text(&raw).ok_or_else(expected)?;
            if values.contains(&name.as_str()) {
                Ok(Raw::Str(name))
            } else {
                Err(format!(
                    "`{path}`: `{name}` is not one of: {}",
                    values.join(", ")
                ))
            }
        }
        Kind::Path => {
            let value = match raw {
                Raw::Path(p) => p,
                other => PathBuf::from(text(&other).ok_or_else(expected)?),
            };
            Ok(Raw::Path(match base {
                Some(base) if value.is_relative() => base.join(value),
                _ => value,
            }))
        }
        Kind::List => match raw {
            Raw::List(items) => Ok(Raw::List(items)),
            other => Ok(Raw::List(
                text(&other)
                    .ok_or_else(expected)?
                    .split(',')
                    .map(|s| s.trim().to_owned())
                    .filter(|s| !s.is_empty())
                    .collect(),
            )),
        },
        Kind::Str | Kind::Url => Ok(Raw::Str(text(&raw).ok_or_else(expected)?)),
        Kind::Object | Kind::Targets => match raw {
            Raw::Json(_) => Ok(raw),
            _ => Err(format!("`{path}` must be a section")),
        },
        Kind::Section | Kind::Map(_) => Err(format!("`{path}` must be a section")),
    }
}

/// A YAML/JSON scalar as a raw value; `None` for null and containers.
fn node_scalar(node: &NodeRef<'_>) -> Option<Raw> {
    match node.kind() {
        ValueKind::Bool => node.as_bool().map(Raw::Bool),
        ValueKind::Int => node
            .as_u64()
            .map(Raw::Int)
            .or_else(|| node.as_i64().map(|n| Raw::Str(n.to_string()))),
        ValueKind::Float => node.as_f64().map(|n| Raw::Str(n.to_string())),
        ValueKind::Str => node.as_str().map(|s| Raw::Str(s.to_owned())),
        _ => None,
    }
}

fn json_scalar(value: &serde_json::Value) -> Option<Raw> {
    match value {
        serde_json::Value::Bool(b) => Some(Raw::Bool(*b)),
        serde_json::Value::Number(n) => {
            Some(n.as_u64().map_or_else(|| Raw::Str(n.to_string()), Raw::Int))
        }
        serde_json::Value::String(s) => Some(Raw::Str(s.clone())),
        serde_json::Value::Array(items) => Some(Raw::List(
            items
                .iter()
                .filter_map(|i| i.as_str().map(str::to_owned))
                .collect(),
        )),
        serde_json::Value::Null | serde_json::Value::Object(_) => None,
    }
}

/// A node as JSON, for opaque sections.
pub(crate) fn node_json(node: &NodeRef<'_>) -> serde_json::Value {
    match node.kind() {
        ValueKind::Null => serde_json::Value::Null,
        ValueKind::Bool => node
            .as_bool()
            .map_or(serde_json::Value::Null, serde_json::Value::Bool),
        ValueKind::Int => node
            .as_i64()
            .map(serde_json::Value::from)
            .or_else(|| node.as_u64().map(serde_json::Value::from))
            .unwrap_or(serde_json::Value::Null),
        ValueKind::Float => node
            .as_f64()
            .map_or(serde_json::Value::Null, serde_json::Value::from),
        ValueKind::Str => node.as_str().map_or(serde_json::Value::Null, |s| {
            serde_json::Value::String(s.to_owned())
        }),
        ValueKind::Array => serde_json::Value::Array(node.items().iter().map(node_json).collect()),
        ValueKind::Object => serde_json::Value::Object(
            node.entries()
                .into_iter()
                .map(|e| {
                    (
                        e.key.to_owned(),
                        e.value.map_or(serde_json::Value::Null, |v| node_json(&v)),
                    )
                })
                .collect(),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flags_add_only_what_was_passed() {
        let layer = Layer::flags()
            .set(
                "lint.min_severity",
                Some(Raw::Str("warn".into())),
                "--min-severity",
            )
            .set("lint.ruleset", None, "--ruleset");
        assert_eq!(
            layer.get("lint.min_severity"),
            Some(&Raw::Str("warning".into()))
        );
        assert!(layer.get("lint.ruleset").is_none());
    }

    #[test]
    fn editor_sections_accept_both_spellings_and_skip_blanks() {
        let base = Path::new("/w");
        let layer = Layer::from_editor(
            &serde_json::json!({"suspect": {
                "lint": {"minSeverity": "error", "ruleset": "r.yaml", "design": "",
                         "rules": {"info-contact": "off"}},
                "validate": {"strict_format": true},
                "inlayHints": {"refTargets": false},
                "basePath": "suspect"
            }}),
            base,
        );
        assert_eq!(
            layer.get("lint.min_severity"),
            Some(&Raw::Str("error".into()))
        );
        assert_eq!(
            layer.get("lint.ruleset"),
            Some(&Raw::Path("/w/r.yaml".into()))
        );
        assert!(layer.get("lint.design").is_none(), "blank is unset");
        assert_eq!(
            layer.get("lint.rules.info-contact"),
            Some(&Raw::Str("off".into()))
        );
        assert_eq!(layer.get("validate.strict_format"), Some(&Raw::Bool(true)));
        assert_eq!(
            layer.get("editor.inlay_hints.refs"),
            Some(&Raw::Bool(false))
        );
        assert!(layer.warnings.is_empty(), "{:?}", layer.warnings);
    }

    #[test]
    fn bad_values_are_errors_with_their_origin() {
        let layer = Layer::from_editor(
            &serde_json::json!({"lint": {"minSeverity": "fatal"}, "gatewayPort": 70000}),
            Path::new("/"),
        );
        assert_eq!(layer.errors.len(), 2, "{:?}", layer.errors);
        assert!(layer.errors.iter().any(|e| e.message.contains("`fatal`")));
        assert!(layer.entries.is_empty());
    }
}
