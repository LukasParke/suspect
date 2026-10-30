//! Native-toolchain manifest for the opt-in acceptance gates.
//!
//! Every generated-SDK behavioral gate needs a real toolchain, and those
//! gates are opt-in (`#[ignore]`) so CI stays hermetic. This manifest makes
//! the requirements declarative: per tool, the command that reports its
//! version, the regex that extracts it, the minimum acceptable version, and
//! the installation guidance shown when the probe fails. Gates call
//! [`probe_status`] instead of hand-rolled `Option` dances, so the skip
//! message tells a contributor exactly what is missing, what was found,
//! what is required, and how to install it.

/// One native tool the acceptance gates may require.
pub struct Tool {
    /// Stable identifier (`"go"`, `"jdk"`, …).
    pub id: &'static str,
    /// What the tool gates, in one line.
    pub description: &'static str,
    /// Command that prints the version, as argv.
    pub version_command: &'static [&'static str],
    /// Regex whose first capture group is the version string.
    pub version_regex: &'static str,
    /// Minimum acceptable version (numeric dot-separated comparison).
    pub min_version: &'static str,
    /// Installation guidance shown when the probe fails.
    pub install_documentation: &'static str,
    /// Environment variable overriding the executable location
    /// (`SUSPECT_DOTNET_BIN`, …). Empty when the tool has no override.
    pub env_override: &'static str,
    /// Environment variable naming a tool home directory whose
    /// `bin/<command>` is the executable (`JAVA_HOME`,
    /// `SUSPECT_RUBY_HOME`). Empty when the tool has none.
    pub home_env: &'static str,
    /// Known version-manager install path used when the tool is not on
    /// `PATH` (mise layouts). A leading `~/` expands to `$HOME`. A file is
    /// the executable itself; a directory is searched for `<command>`,
    /// `bin/<command>`, then `*/bin/<command>` (one nested level, for
    /// archives such as `maven/3/apache-maven-3.9.16/bin/mvn`). Empty when
    /// the tool always lives on `PATH`.
    pub version_manager_path: &'static str,
}

/// The manifest, in canonical order.
pub const TOOLS: &[Tool] = &[
    Tool {
        id: "go",
        description: "Go 1.23+ acceptance gates (incoming webhooks, streams, replay)",
        version_command: &["go", "version"],
        version_regex: r"go(\d+\.\d+(?:\.\d+)?)",
        min_version: "1.23",
        install_documentation: "Install Go 1.23 or later from https://go.dev/doc/install",
        env_override: "",
        home_env: "",
        version_manager_path: "",
    },
    Tool {
        id: "jdk",
        description: "Java 21+ acceptance gates (OAuth lifecycle, pagination, streams)",
        version_command: &["java", "-version"],
        version_regex: r#""?(\d+\.\d+(?:\.\d+)?)"?"#,
        min_version: "21",
        install_documentation: "Install JDK 21 or later from https://adoptium.net, or point JAVA_HOME at one",
        env_override: "SUSPECT_JAVA_BIN",
        home_env: "JAVA_HOME",
        version_manager_path: "~/.local/share/mise/installs/java/temurin-21",
    },
    Tool {
        id: "maven",
        description: "Package compilation for the Java acceptance gates",
        version_command: &["mvn", "--version"],
        version_regex: r"Apache Maven (\d+\.\d+(?:\.\d+)?)",
        min_version: "3.8",
        install_documentation: "Install Maven 3.8 or later from https://maven.apache.org/install.html",
        env_override: "SUSPECT_MAVEN_BIN",
        home_env: "",
        version_manager_path: "~/.local/share/mise/installs/maven/3",
    },
    Tool {
        id: "dotnet",
        description: "C# acceptance gates (OAuth lifecycle, pagination, streams)",
        version_command: &["dotnet", "--version"],
        version_regex: r"(\d+\.\d+\.\d+)",
        min_version: "8.0",
        install_documentation: "Install the .NET 8 SDK or later from https://dotnet.microsoft.com/download",
        env_override: "SUSPECT_DOTNET_BIN",
        home_env: "",
        version_manager_path: "~/.local/share/mise/dotnet-root/dotnet",
    },
    Tool {
        id: "swift",
        description: "Swift acceptance gates (OAuth lifecycle, pagination, streams)",
        version_command: &["swift", "--version"],
        version_regex: r"(?:swift-lang-|Swift version )(\d+\.\d+(?:\.\d+)?)",
        min_version: "5.9",
        install_documentation: "Install Swift 5.9 or later from https://www.swift.org/install, or `mise x swift@latest`",
        env_override: "SUSPECT_SWIFT_BIN",
        home_env: "",
        version_manager_path: "",
    },
    Tool {
        id: "dart",
        description: "Dart acceptance gates (AOT elimination gate, pagination)",
        version_command: &["dart", "--version"],
        version_regex: r"Dart SDK version: (\d+\.\d+\.\d+)",
        min_version: "3.9",
        install_documentation: "Install the Dart SDK 3.9 or later (`mise x dart@3.9 -- dart --version`)",
        env_override: "SUSPECT_DART_BIN",
        home_env: "",
        version_manager_path: "",
    },
    Tool {
        id: "node",
        description: "TypeScript package compilation and behavioral gates",
        version_command: &["node", "--version"],
        version_regex: r"v(\d+\.\d+\.\d+)",
        min_version: "20",
        install_documentation: "Install Node.js 20 or later from https://nodejs.org",
        env_override: "",
        home_env: "",
        version_manager_path: "",
    },
    Tool {
        id: "python",
        description: "Python 3.11+ gates executing emitted Python SDKs and codecs",
        version_command: &["python3", "--version"],
        version_regex: r"Python (\d+\.\d+(?:\.\d+)?)",
        // The emitted `pyproject.toml` declares `requires-python=">=3.11"`.
        min_version: "3.11",
        install_documentation: "Install Python 3.11 or later from https://www.python.org/downloads (or `uv python install 3.11`), then point SUSPECT_PYTHON_BIN at its interpreter",
        env_override: "SUSPECT_PYTHON_BIN",
        home_env: "",
        version_manager_path: "",
    },
    Tool {
        id: "ruby",
        description: "Ruby acceptance gates (OAuth lifecycle, pagination, streams)",
        version_command: &["ruby", "--version"],
        version_regex: r"ruby (\d+\.\d+\.\d+)",
        min_version: "3.2",
        install_documentation: "Install Ruby 3.2 or later from https://www.ruby-lang.org/en/documentation/installation",
        env_override: "SUSPECT_RUBY_BIN",
        home_env: "SUSPECT_RUBY_HOME",
        version_manager_path: "~/.local/share/mise/installs/ruby/3",
    },
    Tool {
        id: "php",
        description: "PHP acceptance gates (OAuth lifecycle, pagination, streams)",
        version_command: &["php", "--version"],
        version_regex: r"PHP (\d+\.\d+\.\d+)",
        min_version: "8.2",
        install_documentation: "Install PHP 8.2 or later from https://www.php.net/manual/en/install.php",
        env_override: "SUSPECT_PHP_BIN",
        home_env: "",
        version_manager_path: concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../target/sdk-php-tools/php-8.3.32/php"
        ),
    },
    Tool {
        id: "terraform",
        description: "Terraform provider acceptance gates (plan/apply/import, addresses, modules)",
        version_command: &["terraform", "version"],
        version_regex: r"Terraform v(\d+\.\d+\.\d+)",
        // Emitted examples declare `ephemeral` credential variables (1.10+).
        min_version: "1.10",
        install_documentation: "Install Terraform 1.10 or later from https://developer.hashicorp.com/terraform/install (or `mise use terraform@latest`), or point SUSPECT_TERRAFORM_BIN at one",
        env_override: "SUSPECT_TERRAFORM_BIN",
        home_env: "",
        version_manager_path: "~/.local/share/mise/installs/terraform/latest/terraform",
    },
];

/// The outcome of a toolchain probe.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolStatus {
    /// The tool is present and at least `min_version`.
    Available {
        /// The detected version string.
        version: String,
    },
    /// The command could not be run at all.
    Missing,
    /// The tool is present but older than `min_version`.
    TooOld {
        /// The detected version string.
        found: String,
        /// The required minimum version string.
        required: String,
    },
}

impl ToolStatus {
    /// Whether the gate may run.
    #[must_use]
    pub fn is_available(&self) -> bool {
        matches!(self, Self::Available { .. })
    }
}

/// Probes one tool by id. Unknown ids are a caller bug and return
/// [`ToolStatus::Missing`] with a diagnostic comment via [`guidance`].
#[must_use]
pub fn probe_status(id: &str) -> (ToolStatus, &'static str) {
    let Some(tool) = TOOLS.iter().find(|t| t.id == id) else {
        return (ToolStatus::Missing, "unknown tool id");
    };
    let Some(executable) = resolve_path(id) else {
        return (ToolStatus::Missing, tool.install_documentation);
    };
    let output = match std::process::Command::new(&executable)
        .args(&tool.version_command[1..])
        .output()
    {
        Ok(output) => output,
        Err(_) => return (ToolStatus::Missing, tool.install_documentation),
    };
    if !output.status.success() {
        return (ToolStatus::Missing, tool.install_documentation);
    }
    let text = String::from_utf8_lossy(&output.stdout).into_owned()
        + &String::from_utf8_lossy(&output.stderr);
    let version = extract_version(&text, tool.version_regex);
    let Some(version) = version else {
        return (ToolStatus::Missing, tool.install_documentation);
    };
    if version_at_least(&version, tool.min_version) {
        (
            ToolStatus::Available { version },
            tool.install_documentation,
        )
    } else {
        (
            ToolStatus::TooOld {
                found: version,
                required: tool.min_version.to_owned(),
            },
            tool.install_documentation,
        )
    }
}

/// Resolves the executable for one tool: the `SUSPECT_*_BIN` override,
/// then the version-manager path when it exists, then `PATH`.
///
/// The returned path is what both the version probe and the actual gate
/// build run, so "probe says available" and "command runs" can't drift
/// apart.
#[must_use]
pub fn resolve_path(id: &str) -> Option<std::path::PathBuf> {
    let tool = TOOLS.iter().find(|t| t.id == id)?;
    if !tool.env_override.is_empty()
        && let Some(path) = std::env::var_os(tool.env_override)
        && !path.is_empty()
    {
        return Some(std::path::PathBuf::from(path));
    }
    if !tool.home_env.is_empty()
        && let Some(home) = std::env::var_os(tool.home_env)
        && !home.is_empty()
    {
        let candidate = std::path::PathBuf::from(home)
            .join("bin")
            .join(tool.version_command[0]);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    if !tool.version_manager_path.is_empty() {
        let managed = expand_home(tool.version_manager_path);
        if managed.is_file() {
            return Some(managed);
        }
        // Directory layouts (`…/installs/java/temurin-21`,
        // `…/installs/maven/3`) need the binary located inside them.
        if managed.is_dir()
            && let Some(candidate) = find_in_install_dir(&managed, tool.version_command[0])
        {
            return Some(candidate);
        }
    }
    find_on_path(tool.version_command[0])
}

/// Locates `name` in a version-manager install directory: directly, under
/// `bin/`, or under exactly one nested directory's `bin/` (the first in
/// sorted order, so resolution is deterministic).
fn find_in_install_dir(dir: &std::path::Path, name: &str) -> Option<std::path::PathBuf> {
    for candidate in [dir.join(name), dir.join("bin").join(name)] {
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    let mut nested = std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect::<Vec<_>>();
    nested.sort();
    nested
        .into_iter()
        .map(|path| path.join("bin").join(name))
        .find(|candidate| candidate.is_file())
}

/// The `JAVA_HOME` of the gated JDK (`gate("jdk")`), as reported by the JVM
/// itself (`java.home`), so PATH shims, `/usr/bin/java` stubs and
/// alternatives links all resolve to the real installation.
///
/// # Errors
/// The `jdk` gate's skip message when no JDK 21+ is resolvable, or a
/// description of why `java.home` could not be read.
pub fn java_home() -> Result<std::path::PathBuf, String> {
    let java = gate("jdk")?;
    let output = std::process::Command::new(&java)
        .args(["-XshowSettings:properties", "-version"])
        .output()
        .map_err(|error| format!("{}: {error}", java.display()))?;
    let text = String::from_utf8_lossy(&output.stderr).into_owned()
        + &String::from_utf8_lossy(&output.stdout);
    text.lines()
        .find_map(|line| line.trim().strip_prefix("java.home = "))
        .map(|home| std::path::PathBuf::from(home.trim()))
        .filter(|home| home.join("bin").is_dir())
        .ok_or_else(|| format!("{} did not report java.home", java.display()))
}

/// Expands a leading `~/` against `$HOME`.
fn expand_home(path: &str) -> std::path::PathBuf {
    match (path.strip_prefix("~/"), std::env::var_os("HOME")) {
        (Some(rest), Some(home)) => std::path::PathBuf::from(home).join(rest),
        _ => std::path::PathBuf::from(path),
    }
}

/// Walks `PATH` for an executable with the given name (the `which` dance
/// without a dependency).
fn find_on_path(name: &str) -> Option<std::path::PathBuf> {
    let path_var = std::env::var_os("PATH")?;
    std::env::split_paths(&path_var)
        .map(|dir| dir.join(name))
        .find(|candidate| candidate.is_file())
}

/// The acceptance-gate entry point: the resolved executable when the tool
/// is present and new enough, or the actionable skip message.
///
/// ```
/// let jdk = suspect_codegen::toolchain::gate("jdk");
/// match jdk {
///     Ok(java) => { /* run the gate with `java` */ }
///     Err(skip) => eprintln!("skipping: {skip}"),
/// }
/// ```
///
/// # Errors
/// The skip message naming what is missing, what was found, what is
/// required, and how to install it.
pub fn gate(id: &str) -> Result<std::path::PathBuf, String> {
    let (status, install) = probe_status(id);
    match status {
        ToolStatus::Available { .. } => {
            resolve_path(id).ok_or_else(|| format!("`{id}` resolved but not locatable"))
        }
        ToolStatus::Missing => Err(format!(
            "{}: not found; {install}",
            TOOLS
                .iter()
                .find(|t| t.id == id)
                .map_or(id, |t| t.description)
        )),
        ToolStatus::TooOld { found, required } => Err(format!(
            "{}: found {found}, requires >= {required}; {install}",
            TOOLS
                .iter()
                .find(|t| t.id == id)
                .map_or(id, |t| t.description)
        )),
    }
}

/// Extracts the first capture group of the compiled regex.
fn extract_version(text: &str, pattern: &str) -> Option<String> {
    let regex = regex::Regex::new(pattern).ok()?;
    regex
        .captures(text)
        .and_then(|captures| captures.get(1))
        .map(|m| m.as_str().to_owned())
}

/// Numeric dot-separated comparison: `1.9 < 1.10`.
#[must_use]
pub fn version_at_least(found: &str, min: &str) -> bool {
    let parse = |s: &str| -> Vec<u64> {
        s.split('.')
            .map(|p| p.trim().parse::<u64>().unwrap_or(0))
            .collect()
    };
    let (f, m) = (parse(found), parse(min));
    for i in 0..m.len().max(f.len()) {
        let a = f.get(i).copied().unwrap_or(0);
        let b = m.get(i).copied().unwrap_or(0);
        if a != b {
            return a > b;
        }
    }
    true
}

/// The actionable skip message for one tool: what it gates, what was
/// found (if anything), what is required, and how to install it.
#[must_use]
pub fn guidance(id: &str) -> String {
    let Some(tool) = TOOLS.iter().find(|t| t.id == id) else {
        return format!("unknown tool `{id}`");
    };
    match probe_status(id).0 {
        ToolStatus::Available { version } => {
            format!(
                "{}: available ({version}); requires >= {}",
                tool.description, tool.min_version
            )
        }
        ToolStatus::Missing => format!(
            "{}: not installed (requires >= {}); {}",
            tool.description, tool.min_version, tool.install_documentation
        ),
        ToolStatus::TooOld { found, required } => format!(
            "{}: found {found}, requires >= {required}; {}",
            tool.description, tool.install_documentation
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_compare_numerically_per_component() {
        assert!(version_at_least("1.23.0", "1.23"));
        assert!(version_at_least("1.24.1", "1.23"));
        assert!(!version_at_least("1.9", "1.10"));
        assert!(!version_at_least("21.0.1", "21.1"));
        assert!(version_at_least("21.0.8", "21"));
    }

    #[test]
    fn every_manifest_tool_has_coherent_metadata() {
        // Realistic command output per tool; the regex must capture the
        // version from the output its own command produces.
        let samples: &[(&str, &str)] = &[
            ("go", "go version go1.23.4 darwin/arm64"),
            ("jdk", "openjdk version \"21.0.8\" 2025-07-15"),
            ("maven", "Apache Maven 3.9.9 (build)"),
            ("dotnet", "8.0.412"),
            (
                "swift",
                "swift-driver version: 1.115 Swift version 6.2 (swift-lang-6.2.0)",
            ),
            ("dart", "Dart SDK version: 3.9.4 (stable)"),
            ("node", "v20.19.0"),
            ("ruby", "ruby 3.4.5 (2025-07-16)"),
            ("php", "PHP 8.3.32 (cli)"),
            ("python", "Python 3.11.13"),
            ("terraform", "Terraform v1.15.8\non darwin_arm64"),
        ];
        for tool in TOOLS {
            assert!(
                regex::Regex::new(tool.version_regex).is_ok(),
                "{}: version regex must compile",
                tool.id
            );
            let sample = samples
                .iter()
                .find(|(id, _)| *id == tool.id)
                .map(|(_, s)| *s)
                .expect("every tool has a sample");
            let extracted = extract_version(sample, tool.version_regex);
            assert!(
                extracted.is_some(),
                "{}: regex must extract a version from {:?}",
                tool.id,
                sample
            );
            assert!(
                version_at_least(extracted.as_deref().unwrap_or(""), tool.min_version),
                "{}: the sample {} must satisfy the manifest minimum {}",
                tool.id,
                extracted.unwrap_or_default(),
                tool.min_version
            );
            assert!(!tool.install_documentation.is_empty());
            assert!(!tool.description.is_empty());
        }
    }

    #[test]
    fn probes_run_without_panicking_and_guidance_is_actionable() {
        for tool in TOOLS {
            let (status, install) = probe_status(tool.id);
            let _ = status;
            assert_eq!(install, tool.install_documentation);
            let message = guidance(tool.id);
            assert!(
                message.contains(tool.description),
                "guidance must name what the tool gates: {message}"
            );
        }
    }

    #[test]
    fn path_resolution_prefers_override_then_managed_then_path() {
        // Every tool resolves through one of the three channels without
        // panicking; a missing tool yields None, not an error.
        for tool in TOOLS {
            let resolved = resolve_path(tool.id);
            if let Some(path) = &resolved {
                assert!(
                    path.is_file(),
                    "{}: resolved path must exist: {}",
                    tool.id,
                    path.display()
                );
            }
        }
        // Unknown ids resolve to nothing.
        assert!(resolve_path("definitely-not-a-tool").is_none());
    }

    #[test]
    fn install_directories_resolve_direct_bin_and_one_nested_bin() {
        let root = tempfile::tempdir().unwrap();
        let touch = |relative: &str| {
            let path = root.path().join(relative);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, b"").unwrap();
            path
        };
        let nested = touch("maven/3/apache-maven-3.9.16/bin/mvn");
        assert_eq!(
            find_in_install_dir(&root.path().join("maven/3"), "mvn"),
            Some(nested)
        );
        let bin = touch("java/temurin-21/bin/java");
        assert_eq!(
            find_in_install_dir(&root.path().join("java/temurin-21"), "java"),
            Some(bin)
        );
        let direct = touch("dotnet-root/dotnet");
        assert_eq!(
            find_in_install_dir(&root.path().join("dotnet-root"), "dotnet"),
            Some(direct)
        );
        assert_eq!(
            find_in_install_dir(&root.path().join("maven"), "none"),
            None
        );
    }

    #[test]
    fn gate_returns_executable_or_actionable_skip() {
        // For every tool the Err message must carry the install doc, and
        // the Ok case must be the same path resolve_path picked.
        for tool in TOOLS {
            match gate(tool.id) {
                Ok(path) => assert_eq!(Some(&path), resolve_path(tool.id).as_ref()),
                Err(skip) => {
                    assert!(
                        skip.contains(tool.install_documentation),
                        "skip message must carry install guidance: {skip}"
                    );
                }
            }
        }
        assert!(gate("definitely-not-a-tool").is_err());
    }
}
