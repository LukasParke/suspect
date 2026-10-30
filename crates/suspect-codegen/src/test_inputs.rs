//! Named external inputs for opt-in gates. A missing input fails the gate.
//!
//! The OpenRouter gates read one variable, `OPENROUTER_WEB_ROOT`, the
//! openrouter-web checkout root. Crates that do not depend on
//! `suspect-codegen` include this `std`-only file with
//! `#[path = "../../suspect-codegen/src/test_inputs.rs"] mod test_inputs;`.

use std::ffi::OsString;
use std::path::PathBuf;

/// The checkout root variable.
pub const ROOT: &str = "OPENROUTER_WEB_ROOT";
/// The public specification, relative to the root.
pub const SPEC: &str = "projects/docs/openapi/openapi.yaml";
/// Accepted for one release; each names the spec file, not the root.
pub const DEPRECATED_FILE_VARS: &[&str] = &[
    "SUSPECT_OPENROUTER_YAML",
    "SUSPECT_OPENROUTER_OPENAPI",
    "SUSPECT_OPENROUTER_SPEC",
    "OPENROUTER_PUBLIC_SCHEMA",
];

/// The openrouter-web checkout root. Panics with the exact export line when
/// unset.
#[must_use]
pub fn openrouter_root() -> PathBuf {
    match resolve_root(|name| std::env::var_os(name)) {
        Ok((root, warning)) => {
            if let Some(warning) = warning {
                eprintln!("{warning}");
            }
            root
        }
        Err(message) => panic!("{message}"),
    }
}

/// `<root>/projects/docs/openapi/openapi.yaml`, checked to exist.
#[must_use]
pub fn openrouter_spec() -> PathBuf {
    let spec = openrouter_root().join(SPEC);
    assert!(
        spec.is_file(),
        "missing input: {} does not exist",
        spec.display()
    );
    spec
}

/// The root from `get` (an environment lookup), with the deprecation note
/// to print when a file variable supplied it.
///
/// # Errors
/// The `missing input` message naming the variable to export.
pub fn resolve_root(
    get: impl Fn(&str) -> Option<OsString>,
) -> Result<(PathBuf, Option<String>), String> {
    if let Some(root) = get(ROOT).filter(|v| !v.is_empty()) {
        return Ok((PathBuf::from(root), None));
    }
    for var in DEPRECATED_FILE_VARS {
        if let Some(file) = get(var).filter(|v| !v.is_empty()) {
            let file = PathBuf::from(file);
            // <root>/projects/docs/openapi/openapi.yaml -> <root>
            let root = file
                .ancestors()
                .nth(4)
                .map_or_else(|| file.clone(), PathBuf::from);
            return Ok((
                root,
                Some(format!(
                    "{var} is deprecated; set {ROOT} to the openrouter-web checkout"
                )),
            ));
        }
    }
    Err(format!(
        "missing input: set {ROOT}=/path/to/openrouter-web (needs {SPEC})"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<OsString> {
        let pairs = pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), OsString::from(v)))
            .collect::<Vec<_>>();
        move |name| {
            pairs
                .iter()
                .find(|(k, _)| k == name)
                .map(|(_, v)| v.clone())
        }
    }

    #[test]
    fn root_wins_over_deprecated_file_variables() {
        let (root, warning) = resolve_root(env(&[
            ("OPENROUTER_WEB_ROOT", "/w"),
            (
                "SUSPECT_OPENROUTER_YAML",
                "/x/projects/docs/openapi/openapi.yaml",
            ),
        ]))
        .unwrap();
        assert_eq!(root, PathBuf::from("/w"));
        assert_eq!(warning, None);
    }

    #[test]
    fn deprecated_file_variable_maps_to_the_root_with_a_warning() {
        let dir = std::env::temp_dir().join("suspect-test-inputs");
        let spec = dir.join(SPEC);
        for var in DEPRECATED_FILE_VARS {
            let (root, warning) = resolve_root(env(&[
                ("OPENROUTER_WEB_ROOT", ""),
                (var, spec.to_str().unwrap()),
            ]))
            .unwrap();
            assert_eq!(root, dir);
            assert_eq!(
                warning.as_deref(),
                Some(
                    format!("{var} is deprecated; set OPENROUTER_WEB_ROOT to the openrouter-web checkout")
                        .as_str()
                )
            );
        }
    }

    #[test]
    fn unset_is_an_error_with_the_export_line() {
        let error = resolve_root(env(&[])).unwrap_err();
        assert_eq!(
            error,
            "missing input: set OPENROUTER_WEB_ROOT=/path/to/openrouter-web (needs projects/docs/openapi/openapi.yaml)"
        );
    }
}
