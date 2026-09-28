//! `suspect release plan`: compatibility report → changelog, semver
//! recommendation, and release notes.
//!
//! Wire-breaking errors force a major bump; warning-severity breaks
//! (constraint tightening, security additions) force a minor bump with an
//! explicit review note; only additions yield a minor recommendation; a
//! clean diff is a patch. Every recommendation names its evidence so a
//! reviewer can confirm or override it.

use std::path::{Path, PathBuf};

use clap::Args;

use crate::OutputFormat;

/// One release-plan review.
#[derive(Debug, Args)]
pub struct ReleasePlanArgs {
    /// The previous spec revision.
    #[arg(required = true)]
    pub old: PathBuf,
    /// The current spec revision.
    #[arg(required = true)]
    pub new: PathBuf,
    /// Output format for the plan.
    #[command(flatten)]
    pub text: crate::TextFormat,
}

/// One semantic difference between revisions, classified for release.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ReleaseChange {
    /// `removed` | `changed` | `added`.
    pub kind: String,
    /// JSONPath-style locator of the change in the new revision.
    pub location: String,
    /// Short human description.
    pub message: String,
}

/// The complete release plan.
#[derive(Debug, serde::Serialize)]
pub struct ReleasePlan {
    /// Recommended bump: `major` | `minor` | `patch`.
    pub semver: String,
    /// Why: the counts that drove the recommendation.
    pub rationale: String,
    /// Wire-breaking findings (errors) from the break differ.
    pub breaks: Vec<super::breaking::BreakFinding>,
    /// Semantic changes classified for the changelog.
    pub changes: Vec<ReleaseChange>,
    /// Changelog body (Markdown).
    pub changelog: String,
    /// Per-SDK release notes section.
    pub sdk_notes: String,
}

/// Plans a release between two spec revisions.
///
/// # Errors
/// File IO, workspace, or contract failures.
pub fn plan(old: &Path, new: &Path) -> anyhow::Result<ReleasePlan> {
    let breaks = super::breaking::break_findings(old, new)?;
    let old_doc = crate::load_doc(old)?;
    let new_doc = crate::load_doc(new)?;
    let overlay =
        suspect_overlay::synthesize_overlay(old_doc.root(), new_doc.root(), "release diff");

    let mut changes: Vec<ReleaseChange> = Vec::new();
    if let Some(actions) = overlay.get("actions")
        && let suspect_overlay::Value::Array(items) = actions
    {
        for action in items {
            let target = action
                .get("target")
                .and_then(|v| match v {
                    suspect_overlay::Value::Str(s) => Some(s.to_string()),
                    _ => None,
                })
                .unwrap_or_default();
            let (kind, message) = if action.get("remove").is_some() {
                ("removed", "member removed in the new revision".to_owned())
            } else if let Some(update) = action.get("update") {
                let summary = match update {
                    suspect_overlay::Value::Object(entries) => {
                        let names: Vec<String> =
                            entries.iter().map(|(k, _)| k.to_string()).collect();
                        format!("added or changed: {}", names.join(", "))
                    }
                    _ => "value updated".to_owned(),
                };
                ("changed", summary)
            } else {
                ("changed", "value updated".to_owned())
            };
            changes.push(ReleaseChange {
                kind: kind.to_owned(),
                location: target,
                message,
            });
        }
    }

    let removed = changes.iter().filter(|c| c.kind == "removed").count();
    let added = changes
        .iter()
        .filter(|c| c.kind == "changed" && c.message.starts_with("added"))
        .count();
    let error_breaks = breaks.iter().filter(|b| b.severity == "error").count();
    let warning_breaks = breaks.iter().filter(|b| b.severity == "warning").count();

    let (semver, rationale) = if error_breaks > 0 || removed > 0 {
        (
            "major",
            format!(
                "{error_breaks} wire-breaking error(s), {removed} removal(s) — consumers can break"
            ),
        )
    } else if warning_breaks > 0 {
        (
            "minor",
            format!(
                "{warning_breaks} tightened constraint(s)/security addition(s) — review before shipping"
            ),
        )
    } else if added > 0 {
        (
            "minor",
            format!("{added} addition(s) — backwards compatible"),
        )
    } else {
        ("patch", "no semantic differences detected".to_owned())
    };

    let mut changelog = String::from("## Changes\n\n");
    for kind in ["removed", "changed"] {
        let section: Vec<&ReleaseChange> = changes.iter().filter(|c| c.kind == kind).collect();
        if section.is_empty() {
            continue;
        }
        changelog.push_str(&format!(
            "### {}\n\n",
            kind[..1].to_uppercase() + &kind[1..]
        ));
        for change in section {
            changelog.push_str(&format!("- `{}` — {}\n", change.location, change.message));
        }
        changelog.push('\n');
    }
    if breaks.is_empty() && changes.is_empty() {
        changelog.push_str("_No semantic differences detected._\n");
    }

    let sdk_notes = if breaks.is_empty() {
        "No wire-breaking changes detected; regenerate SDKs to pick up additions.".to_owned()
    } else {
        format!(
            "{} wire-breaking change(s) detected. Run `suspect codegen-compare` for per-SDK native-interface deltas, regenerate each SDK, and review the breaking findings above before publishing.",
            breaks.len()
        )
    };

    Ok(ReleasePlan {
        semver: semver.to_owned(),
        rationale,
        breaks,
        changes,
        changelog,
        sdk_notes,
    })
}

/// Runs the release-plan command.
///
/// # Errors
/// File IO or contract failures.
pub fn release_plan(args: &ReleasePlanArgs) -> anyhow::Result<i32> {
    let plan = plan(&args.old, &args.new)?;
    match args.text.format {
        OutputFormat::Json => {
            println!("{}", serde_json::to_string_pretty(&plan)?);
        }
        OutputFormat::Sarif => {
            anyhow::bail!("SARIF output is not defined for release plans");
        }
        OutputFormat::Text => {
            println!("Recommended bump: {}", plan.semver.to_uppercase());
            println!("Rationale: {}", plan.rationale);
            println!();
            print!("{}", plan.changelog);
            println!();
            println!("## SDK notes");
            println!("{}", plan.sdk_notes);
        }
    }
    Ok(0)
}
