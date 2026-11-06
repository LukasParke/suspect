//! Python interpreter and tool-venv lookup shared by the Python suites.
//!
//! Interpreters come from [`suspect_codegen::toolchain::pinned_python`], so
//! a developer machine whose `python3` is an older system Python (macOS
//! ships 3.9 with Xcode) still finds the supported 3.11/3.14 installs.
//! Missing interpreters or tool venvs skip, or panic under
//! `SUSPECT_REQUIRE_NATIVE=1`.
#![allow(dead_code)]

use std::{
    ffi::OsString,
    path::{Path, PathBuf},
    process::Command,
};

use suspect_codegen::toolchain;

/// The interpreter the single-interpreter suites run: `SUSPECT_PYTHON_BIN`
/// when set, else the pinned Python 3.11 (the oldest admitted minor).
pub fn python() -> Option<PathBuf> {
    match std::env::var_os("SUSPECT_PYTHON_BIN").filter(|value| !value.is_empty()) {
        Some(path) => Some(path.into()),
        None => toolchain::pinned_python("3.11"),
    }
}

/// An interpreter of one exact admitted minor version (`"3.11"`, `"3.14"`).
pub fn pinned(minor: &str) -> Option<PathBuf> {
    toolchain::pinned_python(minor)
}

/// Every interpreter in `minors`, or `None` (after the skip message) when
/// any of them is missing.
pub fn pinned_all<const N: usize>(minors: [&str; N]) -> Option<[PathBuf; N]> {
    let found: Vec<PathBuf> = minors
        .iter()
        .map(|minor| pinned(minor))
        .collect::<Option<_>>()?;
    found.try_into().ok()
}

/// The build/type-check/docs tools venv interpreter provisioned by
/// `tools/native-setup.sh python` (`SUSPECT_PYTHON_TOOLS` overrides it).
///
/// # Panics
/// When the venv is missing and `SUSPECT_REQUIRE_NATIVE` is set.
pub fn tools() -> Option<PathBuf> {
    let path = std::env::var_os("SUSPECT_PYTHON_TOOLS")
        .filter(|value| !value.is_empty())
        .map_or_else(
            || {
                Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("../../target/sdk-native-python-tools/bin/python")
            },
            PathBuf::from,
        );
    if path.is_file() {
        return Some(path);
    }
    skip(&format!(
        "Python tools venv {} is missing; run `tools/native-setup.sh python` (or set SUSPECT_PYTHON_TOOLS)",
        path.display()
    ));
    None
}

/// The `uv` executable the installed-wheel suites create venvs with
/// (`SUSPECT_UV_BIN` overrides `uv` on PATH). The suites install offline,
/// so `tools/native-setup.sh python` warms uv's cache.
///
/// # Panics
/// When uv is missing and `SUSPECT_REQUIRE_NATIVE` is set.
pub fn uv() -> Option<PathBuf> {
    let uv: PathBuf = std::env::var_os("SUSPECT_UV_BIN")
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "uv".into())
        .into();
    if Command::new(&uv)
        .arg("--version")
        .output()
        .is_ok_and(|output| output.status.success())
    {
        return Some(uv);
    }
    skip(&format!(
        "`{}` not found; install uv (https://docs.astral.sh/uv/) or set SUSPECT_UV_BIN",
        uv.display()
    ));
    None
}

fn skip(message: &str) {
    let required = std::env::var(toolchain::REQUIRE_NATIVE_ENV)
        .is_ok_and(|value| !value.is_empty() && value != "0");
    assert!(
        !required,
        "{} is set: {message}",
        toolchain::REQUIRE_NATIVE_ENV
    );
    eprintln!("skipping: {message}");
}

/// Returns the interpreter named by `env_var` (default `python3`) when it is
/// at least `min`, else the pinned Python 3.11 when `min` allows it,
/// otherwise logs why the check is skipped and returns `None`.
///
/// # Panics
/// Under CI, when no interpreter of at least `min` is found.
pub fn python_at_least(env_var: &str, min: (u32, u32)) -> Option<OsString> {
    let bin = std::env::var_os(env_var).unwrap_or_else(|| "python3".into());
    let found = Command::new(&bin)
        .args([
            "-c",
            "import sys; print(sys.version_info[0], sys.version_info[1])",
        ])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| {
            let text = String::from_utf8_lossy(&output.stdout).into_owned();
            let mut parts = text.split_whitespace().map(str::parse::<u32>);
            Some((parts.next()?.ok()?, parts.next()?.ok()?))
        });
    let problem = match found {
        Some(version) if version >= min => return Some(bin),
        Some((major, minor)) => format!("is Python {major}.{minor}"),
        None => "could not be run".to_owned(),
    };
    if std::env::var_os(env_var).is_none() && min <= (3, 11) {
        // A too-old default `python3`: fall back to the pinned supported
        // interpreter instead of skipping.
        if let Some(pinned) = toolchain::pinned_python("3.11") {
            return Some(pinned.into());
        }
    }
    let message = format!(
        "{} {problem}; this check needs Python {}.{}+ (set {env_var} to override)",
        bin.to_string_lossy(),
        min.0,
        min.1
    );
    assert!(std::env::var_os("CI").is_none(), "{message}");
    eprintln!("skipping: {message}");
    None
}

/// What the installed-wheel suites need: the tools venv, `uv`, and both
/// admitted interpreters (3.11 and 3.14).
pub struct Installed {
    pub tools: PathBuf,
    pub uv: PathBuf,
    pub interpreters: [(&'static str, PathBuf); 2],
}

/// [`Installed`], or `None` after the skip message for whatever is missing.
pub fn installed() -> Option<Installed> {
    let tools = tools();
    let uv = uv();
    let interpreters = pinned_all(["3.11", "3.14"]);
    let [old, new] = interpreters?;
    Some(Installed {
        tools: tools?,
        uv: uv?,
        interpreters: [("3.11", old), ("3.14", new)],
    })
}
