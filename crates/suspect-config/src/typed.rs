//! Merged layer entries -> the typed [`Settings`].
//!
//! Every value was checked against its [`crate::schema::Kind`] when it
//! entered its layer, so this conversion cannot fail.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::Settings;
use crate::layer::{Origin, Raw};

pub(crate) fn build(
    values: &BTreeMap<String, (Raw, Origin)>,
    sections: &[&str],
    root: &Path,
) -> Settings {
    let get = |path: &str| values.get(path).map(|(raw, _)| raw);
    let string = |path: &str| match get(path) {
        Some(Raw::Str(s)) => Some(s.clone()),
        _ => None,
    };
    let path = |path: &str| match get(path) {
        Some(Raw::Path(p)) => Some(p.clone()),
        _ => None,
    };
    let boolean = |path: &str| match get(path) {
        Some(Raw::Bool(b)) => Some(*b),
        _ => None,
    };
    let int = |path: &str| match get(path) {
        Some(Raw::Int(n)) => Some(*n),
        _ => None,
    };
    let list = |path: &str| match get(path) {
        Some(Raw::List(items)) => Some(items.clone()),
        _ => None,
    };
    // `path.<key>` entries of a map row.
    let map = |path: &str| -> BTreeMap<String, String> {
        let prefix = format!("{path}.");
        values
            .range(prefix.clone()..)
            .take_while(|(k, _)| k.starts_with(&prefix))
            .filter_map(|(k, (raw, _))| match raw {
                Raw::Str(name) => Some((k[prefix.len()..].to_owned(), name.clone())),
                _ => None,
            })
            .collect()
    };

    let mut s = Settings::default();
    s.lint.ruleset = path("lint.ruleset");
    s.lint.min_severity = string("lint.min_severity");
    s.lint.design = string("lint.design");
    s.lint.rules = map("lint.rules");
    s.lint.recommended = boolean("lint.recommended").unwrap_or(true);

    s.validate.strict_format = boolean("validate.strict_format").unwrap_or(false);
    s.validate.reference_allowlist = path("validate.reference_allowlist");

    s.format.json = boolean("format.json").unwrap_or(false);
    s.format.yaml = boolean("format.yaml").unwrap_or(false);
    // A `format` section is an opt-in to the canonical formatter, so key
    // reordering defaults on with it and off without it: format-on-save
    // must not reorder a whole file in an unconfigured workspace.
    s.format.sort_keys =
        boolean("format.sort_keys").unwrap_or_else(|| sections.contains(&"format"));

    s.docs.style = string("docs.style");
    s.docs.output = path("docs.output");

    s.codegen.out = path("codegen.out");
    s.codegen.profile = string("codegen.profile");
    s.codegen.package_name = string("codegen.package_name");
    s.codegen.package_version = string("codegen.package_version");
    s.codegen.import_names = map("codegen.import_names");
    s.codegen.operation_ids = list("codegen.operation_ids").unwrap_or_default();
    s.codegen.compatibility_profiles = list("codegen.compatibility_profiles").unwrap_or_default();
    s.codegen.session.config = path("codegen.session.config");
    if let Some(out) = string("codegen.session.out") {
        s.codegen.session.out = out;
    }

    if let Some(n) = int("ref.max_docs") {
        s.refs.max_docs = usize::try_from(n).unwrap_or(usize::MAX);
    }
    if let Some(on) = boolean("editor.inlay_hints.refs") {
        s.editor.inlay_hints.refs = on;
    }
    if let Some(on) = boolean("editor.inlay_hints.properties") {
        s.editor.inlay_hints.properties = on;
    }

    if let Some(url) = string("test.base_url") {
        s.test.base_url = url;
    }
    s.test.credentials = path("test.credentials");
    if let Some(report) = string("test.report") {
        s.test.report = report;
    }

    if let Some(port) = int("gateway.port") {
        s.gateway.port = u16::try_from(port).unwrap_or(s.gateway.port);
    }
    if let Some(mode) = string("gateway.mode") {
        s.gateway.mode = mode;
    }
    s.gateway.upstream = string("gateway.upstream");

    s.diff.baseline = string("diff.baseline");

    s.rules.enabled = boolean("rules.enabled").unwrap_or(false);
    s.rules.roots = match (list("rules.roots"), values.get("rules.roots")) {
        (Some(items), Some((_, origin))) => {
            // Relative roots resolve against the file that set them.
            let base = origin.path().and_then(Path::parent).unwrap_or(root);
            items.into_iter().map(|r| base.join(r)).collect()
        }
        _ => s
            .rules
            .roots
            .iter()
            .map(|r| root.join(r))
            .collect::<Vec<PathBuf>>(),
    };

    s.extensions = match get("extensions") {
        Some(Raw::Json(value)) => Some(value.clone()),
        _ => None,
    };
    s
}
