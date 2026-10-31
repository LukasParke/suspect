//! `resolve` across every layer: precedence, per-key map merging, origins
//! with line numbers, aliases, `extends`, and the table vs struct
//! round trip.

use std::collections::BTreeMap;
use std::path::Path;

use suspect_config::schema::{self, Kind};
use suspect_config::{Env, Inputs, Layer, Origin, Raw, Settings, resolve_with};

fn env(pairs: &[(&str, &str)]) -> Env {
    Env::Map(
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect::<BTreeMap<_, _>>(),
    )
}

fn write(dir: &Path, name: &str, text: &str) {
    let path = dir.join(name);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

/// The six layers, each setting a scalar, a boolean and map keys.
fn fixture(dir: &Path) {
    write(
        dir,
        "suspect.project.json",
        r#"{
  "version": 1,
  "lint": {
    "min_severity": "info",
    "rules": {"from-manifest": "off", "shared": "hint"}
  },
  "validate": {"strict_format": true}
}"#,
    );
    write(
        dir,
        ".suspect.yaml",
        "lint:\n  min_severity: warning\n  rules:\n    shared: error\n    from-file: info\nvalidate:\n  strict_format: false\n",
    );
}

#[test]
fn every_layer_wins_over_the_ones_below() {
    let dir = tempfile::tempdir().unwrap();
    fixture(dir.path());
    let spec = dir.path().join("openapi.yaml");

    // Files only: the settings file beats the manifest per key.
    let r = resolve_with(
        &spec,
        Inputs {
            env: Env::empty(),
            ..Inputs::default()
        },
    )
    .unwrap();
    assert_eq!(r.settings.lint.min_severity.as_deref(), Some("warning"));
    assert!(
        !r.settings.validate.strict_format,
        "file false beats manifest true"
    );
    assert_eq!(r.settings.lint.rules["shared"], "error");
    assert_eq!(
        r.settings.lint.rules["from-manifest"], "off",
        "manifest keys survive"
    );
    assert_eq!(r.settings.lint.rules["from-file"], "info");
    assert_eq!(
        r.origin("lint.min_severity"),
        &Origin::File {
            path: dir.path().join(".suspect.yaml"),
            line: 2
        }
    );
    assert!(matches!(
        r.origin("lint.rules.from-manifest"),
        Origin::Manifest { line: 5, .. }
    ));

    // Env beats the files.
    let r = resolve_with(
        &spec,
        Inputs {
            env: env(&[
                ("SUSPECT_LINT_MIN_SEVERITY", "error"),
                ("SUSPECT_STRICT_FORMAT", "1"),
            ]),
            ..Inputs::default()
        },
    )
    .unwrap();
    assert_eq!(r.settings.lint.min_severity.as_deref(), Some("error"));
    assert!(r.settings.validate.strict_format);
    assert_eq!(
        r.origin("lint.min_severity"),
        &Origin::Env {
            var: "SUSPECT_LINT_MIN_SEVERITY"
        }
    );

    // Editor beats env; the pulled section beats initializationOptions.
    let init = serde_json::json!({"lint": {"minSeverity": "info", "rules": {"from-init": "off"}}});
    let pulled = serde_json::json!({"suspect": {"lint": {"minSeverity": "hint", "rules": {"shared": "warn"}},
        "validate": {"strictFormat": false}}});
    let editor = [&init, &pulled];
    let r = resolve_with(
        &spec,
        Inputs {
            env: env(&[
                ("SUSPECT_LINT_MIN_SEVERITY", "error"),
                ("SUSPECT_STRICT_FORMAT", "1"),
            ]),
            editor: &editor,
            ..Inputs::default()
        },
    )
    .unwrap();
    assert_eq!(r.settings.lint.min_severity.as_deref(), Some("hint"));
    assert!(
        !r.settings.validate.strict_format,
        "an editor false beats an env true"
    );
    assert_eq!(r.settings.lint.rules["shared"], "warning");
    assert_eq!(r.settings.lint.rules["from-init"], "off");
    assert_eq!(r.settings.lint.rules["from-manifest"], "off");
    assert_eq!(
        r.origin("lint.min_severity"),
        &Origin::Editor {
            key: "suspect.lint.minSeverity".into()
        }
    );
    // Every layer's value is in the trace, lowest first.
    let layers: Vec<&str> = r
        .trace("lint.min_severity")
        .iter()
        .map(|(o, _)| o.layer())
        .collect();
    assert_eq!(layers, ["manifest", "file", "env", "editor", "editor"]);

    // A flag beats everything.
    let flags = Layer::flags()
        .set(
            "lint.min_severity",
            Some(Raw::Str("error".into())),
            "--min-severity",
        )
        .set("lint.ruleset", None, "--ruleset");
    let r = resolve_with(
        &spec,
        Inputs {
            flags,
            env: env(&[("SUSPECT_LINT_MIN_SEVERITY", "info")]),
            ..Inputs::default()
        },
    )
    .unwrap();
    assert_eq!(r.settings.lint.min_severity.as_deref(), Some("error"));
    assert_eq!(
        r.origin("lint.min_severity"),
        &Origin::Flag {
            name: "--min-severity"
        }
    );
    assert!(
        r.settings.lint.ruleset.is_none(),
        "an unpassed flag adds nothing"
    );
}

#[test]
fn bad_values_are_errors_that_keep_the_rest() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        ".suspect.yaml",
        "lint:\n  min_severity: fatal\n  rules:\n    info-contact: loud\n    info-license: warn\ngateway:\n  port: 70000\n",
    );
    let error = resolve_with(
        dir.path(),
        Inputs {
            env: Env::empty(),
            ..Inputs::default()
        },
    )
    .unwrap_err();
    let text = error.to_string();
    assert_eq!(error.problems.len(), 3, "{text}");
    assert!(
        text.contains(".suspect.yaml:2: `lint.min_severity`: unrecognized severity `fatal`"),
        "{text}"
    );
    assert!(
        text.contains(".suspect.yaml:4: `lint.rules.info-contact`: unrecognized severity `loud`"),
        "{text}"
    );
    assert_eq!(
        error.resolved.settings.lint.rules["info-license"],
        "warning"
    );
    assert!(error.resolved.settings.lint.min_severity.is_none());
}

#[test]
fn a_broken_file_is_an_error_with_its_line() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        ".suspect.yaml",
        "lint:\n  min_severity: warning\n  rules: [unclosed\n",
    );
    let error = resolve_with(
        dir.path(),
        Inputs {
            env: Env::empty(),
            ..Inputs::default()
        },
    )
    .unwrap_err();
    assert_eq!(error.problems.len(), 1);
    assert!(
        matches!(error.problems[0].origin, Origin::File { .. }),
        "{error}"
    );
    assert!(
        error.resolved.settings.lint.min_severity.is_none(),
        "the file is dropped"
    );
}

#[test]
fn aliases_apply_with_a_warning() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        ".suspect.yaml",
        "formatting:\n  sort_keys: false\ninlay_hints:\n  ref_targets: false\nrefs:\n  maxDocs: 9\ndocs:\n  out: site\nlint:\n  min_severty: error\n",
    );
    let r = resolve_with(
        dir.path(),
        Inputs {
            env: Env::empty(),
            ..Inputs::default()
        },
    )
    .unwrap();
    assert!(!r.settings.format.sort_keys);
    assert!(!r.settings.editor.inlay_hints.refs);
    assert_eq!(r.settings.refs.max_docs, 9);
    assert_eq!(r.settings.docs.output, Some(dir.path().join("site")));
    let warnings: Vec<String> = r.warnings.iter().map(ToString::to_string).collect();
    for expected in [
        "`formatting` is deprecated; use `format`",
        "`formatting.sort_keys` is deprecated; use `format.sort_keys`",
        "`inlay_hints.ref_targets` is deprecated; use `editor.inlay_hints.refs`",
        "`refs.maxDocs` is deprecated; use `ref.max_docs`",
        "`docs.out` is deprecated; use `docs.output`",
        "`lint.min_severty` is not a .suspect.yaml key",
    ] {
        assert!(
            warnings.iter().any(|w| w.contains(expected)),
            "{expected}: {warnings:#?}"
        );
    }
}

#[test]
fn a_format_section_turns_key_sorting_on() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), ".suspect.yaml", "format:\n  json: false\n");
    let r = resolve_with(
        dir.path(),
        Inputs {
            env: Env::empty(),
            ..Inputs::default()
        },
    )
    .unwrap();
    assert!(r.settings.format.sort_keys);
    write(dir.path(), ".suspect.yaml", "lint: {}\n");
    let r = resolve_with(
        dir.path(),
        Inputs {
            env: Env::empty(),
            ..Inputs::default()
        },
    )
    .unwrap();
    assert!(!r.settings.format.sort_keys);
}

#[test]
fn extends_layers_one_parent_under_the_file() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        ".suspect.yaml",
        "lint:\n  min_severity: error\n  ruleset: rules.yaml\n  rules:\n    shared: off\n",
    );
    write(
        dir.path(),
        "api/.suspect.yaml",
        "extends: ../.suspect.yaml\nlint:\n  min_severity: info\n",
    );
    let r = resolve_with(
        &dir.path().join("api/openapi.yaml"),
        Inputs {
            env: Env::empty(),
            ..Inputs::default()
        },
    )
    .unwrap();
    assert_eq!(r.settings.lint.min_severity.as_deref(), Some("info"));
    assert_eq!(r.settings.lint.rules["shared"], "off");
    assert_eq!(
        r.settings.lint.ruleset,
        Some(dir.path().join("rules.yaml")),
        "the parent's paths resolve against the parent"
    );
    assert_eq!(
        r.files,
        [
            dir.path().join(".suspect.yaml"),
            dir.path().join("api/.suspect.yaml")
        ]
    );

    // A second level is rejected, the extending file still applies.
    write(dir.path(), ".suspect.yaml", "extends: other.yaml\n");
    let error = resolve_with(
        &dir.path().join("api"),
        Inputs {
            env: Env::empty(),
            ..Inputs::default()
        },
    )
    .unwrap_err();
    assert!(error.to_string().contains("one level"), "{error}");
    assert_eq!(
        error.resolved.settings.lint.min_severity.as_deref(),
        Some("info")
    );

    // A file extending itself is a cycle.
    write(
        dir.path(),
        "api/.suspect.yaml",
        "extends: ./.suspect.yaml\n",
    );
    let error = resolve_with(
        &dir.path().join("api"),
        Inputs {
            env: Env::empty(),
            ..Inputs::default()
        },
    )
    .unwrap_err();
    assert!(error.to_string().contains("itself"), "{error}");
}

#[test]
fn nothing_found_is_the_defaults() {
    let dir = tempfile::tempdir().unwrap();
    let r = resolve_with(
        dir.path(),
        Inputs {
            env: Env::empty(),
            ..Inputs::default()
        },
    )
    .unwrap();
    let mut expected = Settings::default();
    expected.rules.roots = vec![dir.path().join(".suspect/rules")];
    assert_eq!(r.settings, expected);
    assert!(r.files.is_empty());
    assert!(r.describe().starts_with("no configuration file found"));
}

/// A non-default value for a row, as YAML.
fn sample(setting: &schema::Setting) -> Option<String> {
    Some(match setting.kind {
        Kind::Section | Kind::Targets => return None,
        Kind::Bool => match setting.default {
            Some("true") => "false".into(),
            _ => "true".into(),
        },
        Kind::Str => "sample".into(),
        Kind::Path => "some/where".into(),
        Kind::Url => "http://example.test:1".into(),
        Kind::Port => "9999".into(),
        Kind::Count => "7".into(),
        Kind::Enum(values) => values
            .iter()
            .find(|v| Some(**v) != setting.default)
            .map(|v| (*v).to_owned())?,
        Kind::Severity => "error".into(),
        Kind::PolicySeverity => "off".into(),
        Kind::Map(_) => "{some-rule: off}".into(),
        Kind::List => "[a]".into(),
        Kind::EnumList(values) => format!("[{}]", values[0]),
        Kind::Object => "{x-thing: {after: info}}".into(),
    })
}

/// Nested YAML for a dotted path.
fn yaml_at(path: &str, value: &str) -> String {
    let parts: Vec<&str> = path.split('.').collect();
    let mut out = String::new();
    for (depth, part) in parts.iter().enumerate() {
        out.push_str(&"  ".repeat(depth));
        out.push_str(part);
        out.push(':');
        if depth + 1 == parts.len() {
            out.push(' ');
            out.push_str(value);
        }
        out.push('\n');
    }
    out
}

#[test]
fn every_table_row_reaches_the_typed_settings() {
    let dir = tempfile::tempdir().unwrap();
    let inputs = || Inputs {
        env: Env::empty(),
        ..Inputs::default()
    };
    write(dir.path(), ".suspect.yaml", "{}\n");
    let baseline = resolve_with(dir.path(), inputs()).unwrap().settings;
    for setting in schema::values() {
        if setting.path == "extends" {
            continue;
        }
        let Some(value) = sample(setting) else {
            continue;
        };
        write(dir.path(), ".suspect.yaml", &yaml_at(setting.path, &value));
        let r =
            resolve_with(dir.path(), inputs()).unwrap_or_else(|e| panic!("{}: {e}", setting.path));
        let key = match setting.kind {
            Kind::Map(_) => format!("{}.some-rule", setting.path),
            _ => setting.path.to_owned(),
        };
        assert!(
            matches!(r.origin(&key), Origin::File { .. }),
            "{key}: --explain does not report the file"
        );
        assert!(
            r.explain()
                .iter()
                .any(|row| row.key == key && matches!(row.origin, Origin::File { .. })),
            "{key}: no explain row"
        );
        assert_ne!(
            r.settings, baseline,
            "{}: the typed settings ignore this row",
            setting.path
        );
    }
}

#[test]
fn explain_lists_every_row_and_json_has_the_shape() {
    let dir = tempfile::tempdir().unwrap();
    fixture(dir.path());
    let r = resolve_with(
        dir.path(),
        Inputs {
            env: Env::empty(),
            ..Inputs::default()
        },
    )
    .unwrap();
    let rows = r.explain();
    for setting in
        schema::values().filter(|s| !matches!(s.kind, Kind::Map(_)) && s.path != "extends")
    {
        assert!(
            rows.iter().any(|row| row.key == setting.path),
            "{}",
            setting.path
        );
    }
    let json = r.explain_json();
    let value = json["values"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["key"] == "lint.rules.shared")
        .unwrap();
    assert_eq!(value["value"], "error");
    assert_eq!(value["layer"], "file");
    assert_eq!(value["trace"].as_array().unwrap().len(), 2);
    assert_eq!(json["files"].as_array().unwrap().len(), 2);
}

// Ported from the language server's former `editor_config` tests.

#[test]
fn blank_or_null_editor_strings_do_not_override_the_file() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        ".suspect.yaml",
        "lint:\n  min_severity: warning\n  ruleset: rules.yaml\n  design: warn\n",
    );
    // VS Code's defaults for untouched string settings, and a cleared one.
    for blank in [
        serde_json::json!(""),
        serde_json::json!("   "),
        serde_json::Value::Null,
    ] {
        let client = serde_json::json!({
            "suspect": {"lint": {"minSeverity": blank, "ruleset": blank, "design": blank},
                        "inlayHints": {"refs": false}}
        });
        let editor = [&client];
        let r = resolve_with(
            dir.path(),
            Inputs {
                editor: &editor,
                env: Env::empty(),
                ..Inputs::default()
            },
        )
        .unwrap();
        assert_eq!(
            r.settings.lint.min_severity.as_deref(),
            Some("warning"),
            "{blank}"
        );
        assert_eq!(
            r.settings.lint.ruleset,
            Some(dir.path().join("rules.yaml")),
            "{blank}"
        );
        assert_eq!(
            r.settings.lint.design.as_deref(),
            Some("warning"),
            "{blank}"
        );
        assert!(!r.settings.editor.inlay_hints.refs);
    }
}

#[test]
fn a_relative_editor_ruleset_resolves_against_the_config_root() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "api/.suspect.yaml", "lint: {}\n");
    let client = serde_json::json!({"lint": {"ruleset": "policy/rules.yaml"}});
    let editor = [&client];
    let inputs = || Inputs {
        editor: &editor,
        env: Env::empty(),
        ..Inputs::default()
    };
    let r = resolve_with(&dir.path().join("api/v1/openapi.yaml"), inputs()).unwrap();
    assert_eq!(
        r.settings.lint.ruleset,
        Some(dir.path().join("api/policy/rules.yaml")),
        "the editor's relative path resolves where the file's would, not the cwd"
    );
    // No configuration file: the start directory is the base.
    let bare = tempfile::tempdir().unwrap();
    let r = resolve_with(bare.path(), inputs()).unwrap();
    assert_eq!(
        r.settings.lint.ruleset,
        Some(bare.path().join("policy/rules.yaml"))
    );
    // An absolute path is taken as given.
    let absolute = dir.path().join("elsewhere.yaml");
    let client = serde_json::json!({"lint": {"ruleset": absolute}});
    let editor = [&client];
    let r = resolve_with(
        dir.path(),
        Inputs {
            editor: &editor,
            env: Env::empty(),
            ..Inputs::default()
        },
    )
    .unwrap();
    assert_eq!(r.settings.lint.ruleset, Some(absolute));
}
