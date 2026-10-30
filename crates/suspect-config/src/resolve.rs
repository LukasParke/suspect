//! The precedence rule, stated once and enforced everywhere.
//!
//! ```text
//! default < manifest policy (deprecated) < extends file < .suspect.yaml
//!         < env < editor (initializationOptions < workspace/configuration) < flags
//! ```
//!
//! [`resolve`] and [`resolve_with`] are the only way to read settings: the
//! CLI, CI and the language server all call them, so they agree on every
//! value and `suspect config --explain` can say where each one came from.
//! A flag is `Some` or it is not, so an unset flag never shadows config.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::Settings;
use crate::discover::{self, Found};
use crate::layer::{Layer, Notice, Origin, Raw};
use crate::schema::{self, Kind, Setting};

/// Where environment variables come from.
#[derive(Debug, Clone, Default)]
pub enum Env {
    /// The process environment.
    #[default]
    Process,
    /// A fixed map (tests, and callers that must not see the environment).
    Map(BTreeMap<String, String>),
}

impl Env {
    /// No variables at all.
    #[must_use]
    pub fn empty() -> Self {
        Self::Map(BTreeMap::new())
    }

    /// The variable's value, trimmed; blank counts as unset.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<String> {
        let value = match self {
            Self::Process => std::env::var(name).ok(),
            Self::Map(map) => map.get(name).cloned(),
        }?;
        let value = value.trim();
        (!value.is_empty()).then(|| value.to_owned())
    }
}

/// The layers above the files.
#[derive(Debug, Default)]
pub struct Inputs<'a> {
    /// CLI flags: the top layer.
    pub flags: Layer,
    /// Editor sections, lowest first: initializationOptions, then the
    /// pulled `suspect` section (which wins).
    pub editor: &'a [&'a serde_json::Value],
    /// Process environment by default; tests pass a map.
    pub env: Env,
}

/// Every problem that kept a value from applying, with the settings that
/// did apply (a bad value drops that key; a broken file drops that file).
#[derive(Debug)]
pub struct ConfigError {
    /// Each problem with its origin.
    pub problems: Vec<Notice>,
    /// The settings resolved without the rejected values.
    pub resolved: Box<Resolved>,
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let lines: Vec<String> = self.problems.iter().map(ToString::to_string).collect();
        write!(f, "{}", lines.join("\n"))
    }
}

impl std::error::Error for ConfigError {}

/// The effective configuration for one start path.
#[derive(Debug, Clone, PartialEq)]
pub struct Resolved {
    /// Effective, validated settings.
    pub settings: Settings,
    /// Directory of the settings file in effect, else of the manifest,
    /// else the start directory. Editor paths resolve against it.
    pub root: PathBuf,
    /// The settings file in effect.
    pub config: Option<PathBuf>,
    /// The nearest project manifest at or below the settings file.
    pub manifest: Option<PathBuf>,
    /// Every file read, lowest layer first (for watchers and status).
    pub files: Vec<PathBuf>,
    /// Deprecations and unknown keys, each with an origin.
    pub warnings: Vec<Notice>,
    /// Values that were rejected, each with an origin.
    pub errors: Vec<Notice>,
    /// The directory discovery started from.
    pub start: PathBuf,
    values: BTreeMap<String, (Raw, Origin)>,
    trace: BTreeMap<String, Vec<(Origin, Raw)>>,
}

/// One row of `suspect config --explain`.
#[derive(Debug, Clone)]
pub struct ExplainRow {
    /// Setting path (`lint.rules.<id>` per rule).
    pub key: String,
    /// The effective value as text; `None` when unset with no default.
    pub value: Option<String>,
    /// The effective value as JSON (`null` when unset).
    pub json: serde_json::Value,
    /// Where it came from.
    pub origin: Origin,
    /// Every layer's value, lowest first.
    pub trace: Vec<(Origin, Raw)>,
    /// The table row.
    pub setting: &'static Setting,
}

static DEFAULT_ORIGIN: Origin = Origin::Default;

impl Resolved {
    /// Where the value at `path` came from.
    #[must_use]
    pub fn origin(&self, path: &str) -> &Origin {
        self.values.get(path).map_or(&DEFAULT_ORIGIN, |(_, o)| o)
    }

    /// The raw value at `path`, when some layer set it.
    #[must_use]
    pub fn value(&self, path: &str) -> Option<&Raw> {
        self.values.get(path).map(|(raw, _)| raw)
    }

    /// Every layer's value at `path`, lowest first.
    #[must_use]
    pub fn trace(&self, path: &str) -> &[(Origin, Raw)] {
        self.trace.get(path).map_or(&[], Vec::as_slice)
    }

    /// One row per value setting, and one per `lint.rules` key.
    #[must_use]
    pub fn explain(&self) -> Vec<ExplainRow> {
        let mut rows = Vec::new();
        for setting in schema::values() {
            if setting.path == "extends" {
                continue;
            }
            if let Kind::Map(_) = setting.kind {
                let prefix = format!("{}.", setting.path);
                for (key, (raw, origin)) in self.values.range(prefix.clone()..) {
                    if !key.starts_with(&prefix) {
                        break;
                    }
                    rows.push(ExplainRow {
                        key: key.clone(),
                        value: Some(raw.to_string()),
                        json: raw.to_json(),
                        origin: origin.clone(),
                        trace: self.trace(key).to_vec(),
                        setting,
                    });
                }
                continue;
            }
            let (value, json) = match self.values.get(setting.path) {
                Some((raw, _)) => (Some(raw.to_string()), raw.to_json()),
                None => self.default_of(setting),
            };
            rows.push(ExplainRow {
                key: setting.path.to_owned(),
                value,
                json,
                origin: self.origin(setting.path).clone(),
                trace: self.trace(setting.path).to_vec(),
                setting,
            });
        }
        rows
    }

    fn default_of(&self, setting: &Setting) -> (Option<String>, serde_json::Value) {
        match setting.path {
            // The one default that depends on the files.
            "format.sort_keys" => {
                let on = self.settings.format.sort_keys;
                (Some(on.to_string()), serde_json::Value::Bool(on))
            }
            "rules.roots" => {
                let roots: Vec<String> = self
                    .settings
                    .rules
                    .roots
                    .iter()
                    .map(|p| p.display().to_string())
                    .collect();
                (
                    Some(format!("[{}]", roots.join(", "))),
                    serde_json::Value::from(roots),
                )
            }
            _ => match setting.default {
                Some(text) => (
                    Some(text.to_owned()),
                    serde_json::from_str(text)
                        .unwrap_or_else(|_| serde_json::Value::String(text.to_owned())),
                ),
                None => (None, serde_json::Value::Null),
            },
        }
    }

    /// `{root, config, files, values: [{key, value, origin, layer, trace}],
    /// warnings, errors}`.
    #[must_use]
    pub fn explain_json(&self) -> serde_json::Value {
        let values: Vec<serde_json::Value> = self
            .explain()
            .into_iter()
            .map(|row| {
                let trace: Vec<serde_json::Value> = row
                    .trace
                    .iter()
                    .map(|(origin, raw)| serde_json::json!([origin.to_string(), raw.to_json()]))
                    .collect();
                serde_json::json!({
                    "key": row.key,
                    "value": row.json,
                    "origin": row.origin.to_string(),
                    "layer": row.origin.layer(),
                    "trace": trace,
                })
            })
            .collect();
        let notices =
            |list: &[Notice]| -> Vec<String> { list.iter().map(ToString::to_string).collect() };
        serde_json::json!({
            "root": self.root.display().to_string(),
            "config": self.config.as_ref().map(|p| p.display().to_string()),
            "files": self.files.iter().map(|p| p.display().to_string()).collect::<Vec<_>>(),
            "values": values,
            "warnings": notices(&self.warnings),
            "errors": notices(&self.errors),
        })
    }

    /// A short, human-readable summary for `suspect config`: the file in
    /// effect and every value a file, the environment or a flag set.
    #[must_use]
    pub fn describe(&self) -> String {
        if self.files.is_empty() && self.values.is_empty() {
            return "no configuration file found (using built-in defaults)".to_owned();
        }
        let mut lines = vec![match &self.config {
            Some(path) => format!("config: {}", crate::layer::shown(path).display()),
            None => "no configuration file found".to_owned(),
        }];
        for row in self.explain() {
            if row.origin != Origin::Default
                && let Some(value) = &row.value
            {
                lines.push(format!("  {}: {value}", row.key));
            }
        }
        lines.join("\n")
    }
}

/// Files + environment + defaults, discovered from `start`.
///
/// # Errors
/// Every rejected value or unreadable file; the error carries the rest.
pub fn resolve(start: &Path) -> Result<Resolved, ConfigError> {
    resolve_with(start, Inputs::default())
}

/// The same with flag or editor layers on top.
///
/// # Errors
/// Every rejected value or unreadable file; the error carries the rest.
pub fn resolve_with(start: &Path, inputs: Inputs<'_>) -> Result<Resolved, ConfigError> {
    let start = discover::start_dir(start);
    let found = discover::discover(&start);
    resolve_found(&start, found, inputs)
}

/// [`resolve_with`] for a discovery the caller already ran (the language
/// server keys its cache on [`Found`]).
///
/// # Errors
/// Every rejected value or unreadable file; the error carries the rest.
pub fn resolve_found(
    start: &Path,
    found: Found,
    inputs: Inputs<'_>,
) -> Result<Resolved, ConfigError> {
    let mut layers: Vec<Layer> = Vec::new();
    let mut files = Vec::new();
    let mut errors = Vec::new();

    if let Some(manifest) = &found.manifest
        && let Some(layer) = crate::file::manifest_layer(manifest)
    {
        files.push(manifest.clone());
        layers.push(layer);
    }
    if let Some(config) = &found.config {
        match crate::file::settings_layer(config) {
            Ok((layer, extends)) => {
                if let Some(parent) = extends {
                    match crate::file::extended_layer(config, &parent) {
                        Ok(parent_layer) => {
                            files.push(parent.clone());
                            layers.push(parent_layer);
                        }
                        Err(notice) => errors.push(notice),
                    }
                }
                files.push(config.clone());
                layers.push(layer);
            }
            Err(notice) => errors.push(notice),
        }
    }
    let root = found
        .config
        .as_deref()
        .or(found.manifest.as_deref())
        .and_then(Path::parent)
        .map_or_else(|| start.to_path_buf(), Path::to_path_buf);
    layers.push(Layer::from_env(&inputs.env));
    for section in inputs.editor {
        layers.push(Layer::from_editor(section, &root));
    }
    layers.push(inputs.flags);

    let mut warnings = Vec::new();
    let mut values: BTreeMap<String, (Raw, Origin)> = BTreeMap::new();
    let mut trace: BTreeMap<String, Vec<(Origin, Raw)>> = BTreeMap::new();
    let mut sections: Vec<&'static str> = Vec::new();
    for layer in layers {
        errors.extend(layer.errors);
        warnings.extend(layer.warnings);
        sections.extend(layer.sections);
        for (key, (raw, origin)) in layer.entries {
            trace
                .entry(key.clone())
                .or_default()
                .push((origin.clone(), raw.clone()));
            values.insert(key, (raw, origin));
        }
    }
    let settings = crate::typed::build(&values, &sections, &root);
    let resolved = Resolved {
        settings,
        root,
        config: found.config,
        manifest: found.manifest,
        files,
        warnings,
        errors: errors.clone(),
        start: start.to_path_buf(),
        values,
        trace,
    };
    if errors.is_empty() {
        Ok(resolved)
    } else {
        Err(ConfigError {
            problems: errors,
            resolved: Box::new(resolved),
        })
    }
}

/// One setting's resolution across the layers, remembering which layer
/// supplied the value.
#[derive(Debug, Clone, PartialEq)]
pub struct Resolution<T> {
    /// The effective value.
    pub value: T,
    /// Which layer supplied it.
    pub source: LayerKind,
    /// Every layer, for `suspect config` style reporting.
    pub trace: Vec<(LayerKind, Option<T>)>,
}

/// Where a value came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayerKind {
    /// An explicit command-line flag.
    Flag,
    /// An environment variable.
    Env,
    /// A configuration file.
    Config,
    /// The built-in default.
    Default,
}

impl LayerKind {
    /// A human label.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Flag => "flag",
            Self::Env => "environment",
            Self::Config => "config",
            Self::Default => "default",
        }
    }
}

impl<T: Clone> Resolution<T> {
    /// Resolves a value from the four layers, highest precedence first.
    ///
    /// `flag` is `None` when the user did not pass the flag, which is the
    /// case that previously could not be distinguished from a default.
    #[must_use]
    pub fn new(flag: Option<T>, env: Option<T>, config: Option<T>, default: T) -> Self {
        let mut trace: Vec<(LayerKind, Option<T>)> = Vec::new();
        for (layer, value) in [
            (LayerKind::Flag, flag),
            (LayerKind::Env, env),
            (LayerKind::Config, config),
        ] {
            trace.push((layer, value.clone()));
        }
        let resolved = trace
            .iter()
            .find_map(|(layer, value)| value.clone().map(|value| (*layer, value)));
        let (source, value) = resolved.unwrap_or((LayerKind::Default, default));
        Self {
            value,
            source,
            trace,
        }
    }

    /// Whether the value came from an explicit flag or environment
    /// variable rather than a file or default.
    #[must_use]
    pub fn is_explicit(&self) -> bool {
        matches!(self.source, LayerKind::Flag | LayerKind::Env)
    }
}

/// Resolves a string-valued setting from an environment variable name.
#[must_use]
pub fn resolve_str(
    flag: Option<String>,
    key: &str,
    config: Option<String>,
    default: &str,
) -> String {
    let env = std::env::var(key).ok().filter(|v| !v.is_empty());
    Resolution::new(flag, env, config, default.to_owned()).value
}

/// Resolves a boolean setting from an environment variable name.
///
/// A clap `bool` flag cannot say "explicitly false", so only `true` counts
/// as a flag value; an unset flag defers to the environment, then config.
#[must_use]
pub fn resolve_bool(flag: bool, key: &str, config: bool) -> bool {
    let env = std::env::var(key).ok().map(|v| {
        matches!(
            v.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        )
    });
    Resolution::new(flag.then_some(true), env, Some(config), false).value
}

#[cfg(test)]
mod legacy_tests {
    use super::*;

    #[test]
    fn a_flag_beats_config_and_default() {
        let resolved = Resolution::new(
            Some("warning".to_owned()),
            None,
            Some("info".to_owned()),
            "hint".to_owned(),
        );
        assert_eq!(resolved.value, "warning");
        assert_eq!(resolved.source, LayerKind::Flag);
        assert!(resolved.is_explicit());
    }

    #[test]
    fn config_beats_default_when_no_flag_is_given() {
        let resolved = Resolution::new(
            None::<String>,
            None,
            Some("info".to_owned()),
            "hint".to_owned(),
        );
        assert_eq!(resolved.value, "info");
        assert_eq!(resolved.source, LayerKind::Config);
        assert!(!resolved.is_explicit());
    }

    #[test]
    fn the_default_applies_with_nothing_set() {
        let resolved = Resolution::new(None::<String>, None, None, "hint".to_owned());
        assert_eq!(resolved.value, "hint");
        assert_eq!(resolved.source, LayerKind::Default);
    }

    #[test]
    fn env_sits_between_flag_and_config() {
        let resolved = Resolution::new(
            Some("error".to_owned()),
            Some("warning".to_owned()),
            Some("info".to_owned()),
            "hint".to_owned(),
        );
        assert_eq!(resolved.value, "error", "the flag still wins over env");
        let resolved = Resolution::new(
            None::<String>,
            Some("warning".to_owned()),
            Some("info".to_owned()),
            "hint".to_owned(),
        );
        assert_eq!(resolved.value, "warning");
        assert_eq!(resolved.source, LayerKind::Env);
    }

    #[test]
    fn every_layer_is_traceable() {
        let resolved = Resolution::new(
            Some("error".to_owned()),
            Some("warning".to_owned()),
            Some("info".to_owned()),
            "hint".to_owned(),
        );
        let layers: Vec<&str> = resolved.trace.iter().map(|(l, _)| l.label()).collect();
        assert_eq!(layers.first(), Some(&"flag"));
        assert!(layers.contains(&"environment"));
        assert!(layers.contains(&"config"));
    }

    #[test]
    fn boolean_env_values_are_honoured() {
        // A flag (even false, as clap supplies a default bool) is explicit
        // only when set; the helper's contract is documented in the module.
        assert!(resolve_bool(true, "SUSPECT_TEST_UNSET_BOOL", false));
        assert!(!resolve_bool(false, "SUSPECT_TEST_UNSET_BOOL", false));
        // An unset flag must not shadow the config value.
        assert!(resolve_bool(false, "SUSPECT_TEST_UNSET_BOOL", true));
    }
}
