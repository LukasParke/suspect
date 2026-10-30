//! The server's own protocol surface: the custom requests and the
//! `workspace/executeCommand` ids it implements, each with a version.
//!
//! One table drives both what `initialize` advertises in
//! `capabilities.experimental.suspect` and what it lists in
//! `executeCommandProvider`, so a client can check for a feature instead of
//! trying it. Bump an entry's version when its params or result change
//! incompatibly (positional command arguments included); adding an optional
//! field is not a bump. [`PROTOCOL`] changes only if the envelope does.
//!
//! A client may own the user-facing side of a server command (a picker,
//! progress, a readable result). It lists those ids in its own
//! `capabilities.experimental.suspect.clientCommands`; the server leaves
//! them out of `executeCommandProvider`, so the client does not register
//! them twice, and still executes them when the client forwards them.

use serde_json::{Map, Value, json};
use tower_lsp::lsp_types::ClientCapabilities;
use tower_lsp::lsp_types::request::Request;

use crate::{generation_contract, links, render_sdk, run_lenses};

/// A protocol entry and its version.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Versioned {
    /// The request method or command id.
    pub name: &'static str,
    /// Bumped on an incompatible change to params or result.
    pub version: u32,
}

/// The envelope version of `experimental.suspect`.
pub const PROTOCOL: u32 = 1;

/// Custom requests the server answers.
pub const REQUESTS: &[Versioned] = &[
    Versioned {
        name: <generation_contract::GenerationContractRequest as Request>::METHOD,
        version: 1,
    },
    Versioned {
        name: <render_sdk::RenderSdkRequest as Request>::METHOD,
        version: 1,
    },
    // Without `live-run` the method exists but can only fail, so it is not
    // advertised.
    #[cfg(feature = "live-run")]
    Versioned {
        name: <run_lenses::RunWorkflowRequest as Request>::METHOD,
        version: 1,
    },
];

/// `workspace/executeCommand` ids the server implements (`run_command`).
pub const COMMANDS: &[Versioned] = &[
    Versioned {
        name: links::SHOW_REFS_COMMAND,
        version: 1,
    },
    Versioned {
        name: links::OPEN_REF_COMMAND,
        version: 1,
    },
    Versioned {
        name: links::ADD_OPERATION_ID_COMMAND,
        version: 1,
    },
    Versioned {
        name: "suspect.generateExample",
        version: 1,
    },
    Versioned {
        name: "suspect.showRefGraph",
        version: 1,
    },
    Versioned {
        name: "suspect.breakingChanges",
        version: 1,
    },
    Versioned {
        name: "suspect.contractCoverage",
        version: 1,
    },
    Versioned {
        name: "suspect.changeImpact",
        version: 1,
    },
    Versioned {
        name: "suspect.verifyContract",
        version: 1,
    },
    Versioned {
        name: "suspect.runService",
        version: 1,
    },
    Versioned {
        name: "suspect.extractSchema",
        version: 1,
    },
    Versioned {
        name: "suspect.inlineSchema",
        version: 1,
    },
    Versioned {
        name: "suspect.editorLatency",
        version: 1,
    },
    Versioned {
        name: run_lenses::RUN_WORKFLOW_COMMAND,
        version: 1,
    },
    Versioned {
        name: run_lenses::RENDER_PREVIEW_COMMAND,
        version: 1,
    },
];

fn table(entries: &[Versioned]) -> Value {
    Value::Object(
        entries
            .iter()
            .map(|e| (e.name.to_owned(), Value::from(e.version)))
            .collect::<Map<_, _>>(),
    )
}

/// The `capabilities.experimental` value `initialize` returns. `commands`
/// lists every server command, including the ones a client declared as
/// its own.
#[must_use]
pub fn experimental() -> Value {
    json!({ "suspect": {
        "protocol": PROTOCOL,
        "requests": table(REQUESTS),
        "commands": table(COMMANDS),
    } })
}

/// The client's `capabilities.experimental.suspect` object, when it sent one.
fn client_section(caps: &ClientCapabilities) -> Option<&Value> {
    caps.experimental.as_ref()?.get("suspect")
}

/// The command ids the client declared it owns
/// (`experimental.suspect.clientCommands`). Empty for a client that sent
/// none, which then gets the full list.
#[must_use]
pub fn client_commands(caps: &ClientCapabilities) -> Vec<String> {
    client_section(caps)
        .and_then(|s| s.get("clientCommands"))
        .and_then(Value::as_array)
        .map(|ids| {
            ids.iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

/// The ids for `executeCommandProvider.commands`: every server command
/// except the ones the client owns.
#[must_use]
pub fn registered_commands(client_owned: &[String]) -> Vec<String> {
    COMMANDS
        .iter()
        .filter(|c| !client_owned.iter().any(|owned| owned == c.name))
        .map(|c| c.name.to_owned())
        .collect()
}

/// The extension version a client reported: VS Code sends
/// `capabilities.experimental.suspect.extensionVersion`, clients without
/// custom capabilities (Zed) send `initializationOptions.extensionVersion`.
#[must_use]
pub fn extension_version(
    caps: Option<&ClientCapabilities>,
    initialization_options: Option<&Value>,
) -> Option<String> {
    caps.and_then(client_section)
        .and_then(|s| s.get("extensionVersion"))
        .or_else(|| initialization_options?.get("extensionVersion"))
        .and_then(Value::as_str)
        .map(str::to_owned)
}

/// `major.minor` of a version string; `None` when it does not parse.
fn major_minor(version: &str) -> Option<(u64, u64)> {
    let core = version.trim().trim_start_matches('v');
    let core = core.split(['-', '+']).next()?;
    let mut parts = core.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    Some((major, minor))
}

/// The warning for a server serving an extension built for another
/// release: major or minor differ (for `0.x` that is the minor alone).
/// A patch difference, or a version that does not parse, is not a
/// mismatch.
#[must_use]
pub fn version_mismatch(server: &str, extension: &str) -> Option<String> {
    let (s, e) = (major_minor(server)?, major_minor(extension)?);
    (s != e).then(|| {
        format!(
            "suspect {server} is serving an extension built for {}.{}.x. Editor features may not match.",
            e.0, e.1
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn caps(experimental: Value) -> ClientCapabilities {
        ClientCapabilities {
            experimental: Some(experimental),
            ..ClientCapabilities::default()
        }
    }

    #[test]
    fn names_are_unique() {
        for list in [REQUESTS, COMMANDS] {
            let mut names: Vec<_> = list.iter().map(|e| e.name).collect();
            names.sort_unstable();
            let before = names.len();
            names.dedup();
            assert_eq!(names.len(), before, "duplicate entry in {list:?}");
        }
    }

    #[test]
    fn experimental_lists_every_entry_with_its_version() {
        let value = experimental();
        let suspect = &value["suspect"];
        assert_eq!(suspect["protocol"], PROTOCOL);
        for e in REQUESTS {
            assert_eq!(suspect["requests"][e.name], e.version, "{}", e.name);
        }
        for e in COMMANDS {
            assert_eq!(suspect["commands"][e.name], e.version, "{}", e.name);
        }
        assert_eq!(
            suspect["requests"].as_object().map(serde_json::Map::len),
            Some(REQUESTS.len())
        );
    }

    #[test]
    fn run_workflow_is_advertised_only_with_live_run() {
        let listed = REQUESTS.iter().any(|e| e.name == "suspect/runWorkflow");
        assert_eq!(listed, cfg!(feature = "live-run"));
    }

    #[test]
    fn declared_client_commands_are_left_out_of_the_registration() {
        let owned = client_commands(&caps(json!({ "suspect": {
            "clientCommands": ["suspect.showRefGraph", "suspect.runWorkflow", 7, "not.a.server.id"]
        } })));
        assert_eq!(
            owned,
            [
                "suspect.showRefGraph",
                "suspect.runWorkflow",
                "not.a.server.id"
            ]
        );
        let registered = registered_commands(&owned);
        assert_eq!(registered.len(), COMMANDS.len() - 2);
        assert!(!registered.iter().any(|c| c == "suspect.showRefGraph"));
        assert!(!registered.iter().any(|c| c == "suspect.runWorkflow"));
        assert!(registered.iter().any(|c| c == "suspect.showRefs"));
    }

    #[test]
    fn a_client_without_the_section_gets_every_command() {
        assert!(client_commands(&ClientCapabilities::default()).is_empty());
        assert!(client_commands(&caps(json!({ "other": {} }))).is_empty());
        assert_eq!(registered_commands(&[]).len(), COMMANDS.len());
    }

    /// Every arm of `run_command`'s match is in [`COMMANDS`], and every
    /// entry has an arm. Read from the source, because the match is the
    /// only place the dispatch is written down.
    #[test]
    fn commands_match_run_command_arms() {
        let source = include_str!("lib.rs");
        let start = source
            .find("    async fn run_command(")
            .expect("run_command in lib.rs");
        let body = &source[start..];
        let end = body.find("\n    }\n").expect("end of run_command");
        let consts = [
            ("links::SHOW_REFS_COMMAND", links::SHOW_REFS_COMMAND),
            ("links::OPEN_REF_COMMAND", links::OPEN_REF_COMMAND),
            (
                "links::ADD_OPERATION_ID_COMMAND",
                links::ADD_OPERATION_ID_COMMAND,
            ),
            (
                "run_lenses::RUN_WORKFLOW_COMMAND",
                run_lenses::RUN_WORKFLOW_COMMAND,
            ),
            (
                "run_lenses::RENDER_PREVIEW_COMMAND",
                run_lenses::RENDER_PREVIEW_COMMAND,
            ),
        ];
        let mut arms = Vec::new();
        for line in body[..end].lines() {
            // Arms of the top-level match sit at exactly twelve spaces.
            let Some(rest) = line.strip_prefix("            ") else {
                continue;
            };
            if rest.starts_with(' ') {
                continue;
            }
            let Some((pattern, _)) = rest.split_once(" => ") else {
                continue;
            };
            for alt in pattern.split('|').map(str::trim) {
                if alt == "_" {
                    continue;
                }
                if let Some(lit) = alt.strip_prefix('"').and_then(|s| s.strip_suffix('"')) {
                    arms.push(lit.to_owned());
                } else {
                    let (_, value) = consts
                        .iter()
                        .find(|(path, _)| *path == alt)
                        .unwrap_or_else(|| panic!("unknown run_command arm {alt}"));
                    arms.push((*value).to_owned());
                }
            }
        }
        let mut arms_sorted = arms.clone();
        arms_sorted.sort();
        let mut listed: Vec<String> = COMMANDS.iter().map(|c| c.name.to_owned()).collect();
        listed.sort();
        assert_eq!(arms_sorted, listed);
    }

    #[test]
    fn extension_version_prefers_capabilities_over_initialization_options() {
        let with_caps = caps(json!({ "suspect": { "extensionVersion": "0.2.0" } }));
        let options = json!({ "extensionVersion": "0.3.0" });
        assert_eq!(
            extension_version(Some(&with_caps), Some(&options)).as_deref(),
            Some("0.2.0")
        );
        assert_eq!(
            extension_version(Some(&ClientCapabilities::default()), Some(&options)).as_deref(),
            Some("0.3.0")
        );
        assert_eq!(extension_version(None, None), None);
        assert_eq!(extension_version(None, Some(&json!({ "lint": {} }))), None);
    }

    #[test]
    fn versions_mismatch_on_major_or_minor_only() {
        assert_eq!(version_mismatch("0.1.0", "0.1.7"), None, "patch");
        assert_eq!(
            version_mismatch("0.1.0", "v0.1.0-dev"),
            None,
            "prefix, pre-release"
        );
        assert_eq!(
            version_mismatch("0.3.1", "0.2.0").as_deref(),
            Some(
                "suspect 0.3.1 is serving an extension built for 0.2.x. Editor features may not match."
            )
        );
        assert!(version_mismatch("1.2.0", "2.2.0").is_some(), "major");
        assert!(version_mismatch("1.2.0", "1.3.0").is_some(), "minor");
        assert_eq!(version_mismatch("0.1.0", "unknown"), None, "unparseable");
    }
}
