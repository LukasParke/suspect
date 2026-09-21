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
    },
    Tool {
        id: "jdk",
        description: "Java 21+ acceptance gates (OAuth lifecycle, pagination, streams)",
        version_command: &["java", "-version"],
        version_regex: r#""?(\d+\.\d+(?:\.\d+)?)"?"#,
        min_version: "21",
        install_documentation: "Install JDK 21 or later from https://adoptium.net, or point JAVA_HOME at one",
    },
    Tool {
        id: "maven",
        description: "Package compilation for the Java acceptance gates",
        version_command: &["mvn", "--version"],
        version_regex: r"Apache Maven (\d+\.\d+(?:\.\d+)?)",
        min_version: "3.8",
        install_documentation: "Install Maven 3.8 or later from https://maven.apache.org/install.html",
    },
    Tool {
        id: "dotnet",
        description: "C# acceptance gates (OAuth lifecycle, pagination, streams)",
        version_command: &["dotnet", "--version"],
        version_regex: r"(\d+\.\d+\.\d+)",
        min_version: "8.0",
        install_documentation: "Install the .NET 8 SDK or later from https://dotnet.microsoft.com/download",
    },
    Tool {
        id: "swift",
        description: "Swift acceptance gates (OAuth lifecycle, pagination, streams)",
        version_command: &["swift", "--version"],
        version_regex: r"(?:swift-lang-|Swift version )(\d+\.\d+(?:\.\d+)?)",
        min_version: "5.9",
        install_documentation: "Install Swift 5.9 or later from https://www.swift.org/install, or `mise x swift@latest`",
    },
    Tool {
        id: "dart",
        description: "Dart acceptance gates (AOT elimination gate, pagination)",
        version_command: &["dart", "--version"],
        version_regex: r"Dart SDK version: (\d+\.\d+\.\d+)",
        min_version: "3.9",
        install_documentation: "Install the Dart SDK 3.9 or later (`mise x dart@3.9 -- dart --version`)",
    },
    Tool {
        id: "node",
        description: "TypeScript package compilation and behavioral gates",
        version_command: &["node", "--version"],
        version_regex: r"v(\d+\.\d+\.\d+)",
        min_version: "20",
        install_documentation: "Install Node.js 20 or later from https://nodejs.org",
    },
    Tool {
        id: "ruby",
        description: "Ruby acceptance gates (OAuth lifecycle, pagination, streams)",
        version_command: &["ruby", "--version"],
        version_regex: r"ruby (\d+\.\d+\.\d+)",
        min_version: "3.2",
        install_documentation: "Install Ruby 3.2 or later from https://www.ruby-lang.org/en/documentation/installation",
    },
    Tool {
        id: "php",
        description: "PHP acceptance gates (OAuth lifecycle, pagination, streams)",
        version_command: &["php", "--version"],
        version_regex: r"PHP (\d+\.\d+\.\d+)",
        min_version: "8.2",
        install_documentation: "Install PHP 8.2 or later from https://www.php.net/manual/en/install.php",
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
    let output = match std::process::Command::new(tool.version_command[0])
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
}
