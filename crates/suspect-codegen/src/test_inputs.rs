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
/// The openrouter-web checkout root. Panics with the exact export line when
/// unset.
#[must_use]
pub fn openrouter_root() -> PathBuf {
    resolve_root(|name| std::env::var_os(name)).unwrap_or_else(|message| panic!("{message}"))
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

/// The root from `get` (an environment lookup).
///
/// # Errors
/// The `missing input` message naming the variable to export.
pub fn resolve_root(get: impl Fn(&str) -> Option<OsString>) -> Result<PathBuf, String> {
    get(ROOT)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .ok_or_else(|| format!("missing input: set {ROOT}=/path/to/openrouter-web (needs {SPEC})"))
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
    fn the_root_variable_names_the_checkout() {
        assert_eq!(
            resolve_root(env(&[("OPENROUTER_WEB_ROOT", "/w")])).unwrap(),
            PathBuf::from("/w")
        );
        // The old file variables are not read.
        assert!(
            resolve_root(env(&[(
                "SUSPECT_OPENROUTER_YAML",
                "/x/projects/docs/openapi/openapi.yaml"
            )]))
            .is_err()
        );
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
