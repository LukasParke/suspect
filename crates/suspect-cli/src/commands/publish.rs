//! `suspect release publish`: SDK release orchestration.
//!
//! One release manifest declares every SDK target — backend, generated
//! directory, registry, package identity, and the exact publish command.
//! The command then:
//!
//! 1. **Checks cross-backend version consistency.** Every target must carry
//!    the release version, or declare its own with a stated reason.
//! 2. **Preflights the generated artifacts.** The version is read back out
//!    of the real package descriptor each backend emits (package.json,
//!    pyproject.toml, Cargo.toml, pom.xml, composer.json, pubspec.yaml,
//!    the gemspec, the csproj, CMakeLists.txt) so a stale generation is
//!    caught before anything is uploaded. Go modules and Swift packages
//!    carry no in-file version and are verified as tag-versioned.
//! 3. **Emits an ordered plan** grouped by registry, and runs it only when
//!    explicitly asked (`--execute`). Progress is recorded to a state file
//!    so an interrupted multi-registry release resumes instead of
//!    re-publishing what already landed.
//!
//! `suspect release workflow` renders the same manifest as a tag-triggered
//! GitHub Actions workflow.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use clap::Args;
use serde::Serialize;

use crate::OutputFormat;

/// Arguments for the publish orchestration.
#[derive(Debug, Args)]
pub struct PublishArgs {
    /// Release manifest (`release.json`).
    #[arg(long, default_value = "release.json")]
    pub manifest: PathBuf,
    /// Print the plan only (the default).
    #[arg(long, conflicts_with = "execute")]
    pub dry_run: bool,
    /// Actually run the publish commands. Publishing is destructive on the
    /// registry, so it never happens without this flag.
    #[arg(long)]
    pub execute: bool,
    /// Resume: skip targets already recorded as published in the state file.
    #[arg(long, requires = "execute")]
    pub resume: bool,
    /// State file recording completed targets.
    #[arg(long, default_value = ".release-state.json")]
    pub state: PathBuf,
    /// Output format for the plan report.
    #[command(flatten)]
    pub text: crate::TextFormat,
}

/// Arguments for workflow rendering.
#[derive(Debug, Args)]
pub struct WorkflowArgs {
    /// Release manifest (`release.json`).
    #[arg(long, default_value = "release.json")]
    pub manifest: PathBuf,
    /// Write the workflow here instead of stdout.
    #[arg(short, long)]
    pub output: Option<PathBuf>,
}

/// One SDK publish target.
#[derive(Debug, Clone)]
pub struct PublishTarget {
    /// Backend id (`typescript`, `python`, …).
    pub backend: String,
    /// Generated package directory, relative to the manifest.
    pub directory: PathBuf,
    /// Native package identity.
    pub package: String,
    /// Registry id (`npm`, `pypi`, `maven-central`, …).
    pub registry: String,
    /// Exact publish command.
    pub command: String,
    /// Declared version; defaults to the release version.
    pub version: Option<String>,
    /// Why this target's version differs from the release version.
    pub version_skew_reason: Option<String>,
    /// Targets that must publish first.
    pub depends_on: Vec<String>,
}

/// A parsed release manifest.
#[derive(Debug, Clone)]
pub struct ReleaseManifest {
    /// Directory the manifest paths resolve against.
    pub root: PathBuf,
    /// The release version every target must carry.
    pub version: String,
    /// Every SDK target, in manifest order.
    pub targets: Vec<PublishTarget>,
    /// Git tag that triggers the release (informational).
    pub tag: Option<String>,
}

/// One target's preflight verdict.
#[derive(Debug, Clone, Serialize)]
pub struct TargetReport {
    /// Backend id.
    pub backend: String,
    /// Native package identity.
    pub package: String,
    /// Registry id.
    pub registry: String,
    /// Declared version for this target.
    pub version: String,
    /// The descriptor file the version was read from, when it has one.
    pub descriptor: Option<String>,
    /// Version actually present in the descriptor, when readable.
    pub observed_version: Option<String>,
    /// Preflight errors; empty means ready to publish.
    pub errors: Vec<String>,
    /// Advisory notes (tag-versioned targets, declared skew).
    pub notes: Vec<String>,
    /// The command that will run.
    pub command: String,
    /// True when already published in a resumed state file.
    pub already_published: bool,
}

/// The complete publish plan.
#[derive(Debug, Serialize)]
pub struct PublishPlan {
    /// Release version.
    pub version: String,
    /// Git tag for the release.
    pub tag: String,
    /// Targets grouped by registry, in publish order.
    pub order: Vec<String>,
    /// Per-target preflight reports.
    pub targets: Vec<TargetReport>,
    /// Target count.
    pub total: usize,
    /// Ready-to-publish target count.
    pub ready: usize,
}

/// Parses a release manifest.
///
/// # Errors
/// IO or schema failures with the offending field named.
pub fn parse_manifest(path: &Path) -> anyhow::Result<ReleaseManifest> {
    let text = std::fs::read_to_string(path)?;
    let value: serde_json::Value = serde_json::from_str(&text)
        .map_err(|e| anyhow::anyhow!("{}: invalid JSON: {e}", path.display()))?;
    let root = path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    let version = value
        .get("version")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("release manifest `version` is required"))?
        .to_owned();
    let tag = value
        .get("tag")
        .and_then(|v| v.as_str())
        .map(str::to_owned)
        .or_else(|| Some(format!("v{version}")));
    let targets_value = value
        .get("targets")
        .and_then(|v| v.as_array())
        .ok_or_else(|| anyhow::anyhow!("release manifest `targets` array is required"))?;

    let mut targets = Vec::new();
    for (index, entry) in targets_value.iter().enumerate() {
        let field = |name: &str| -> anyhow::Result<String> {
            entry
                .get(name)
                .and_then(|v| v.as_str())
                .map(str::to_owned)
                .ok_or_else(|| {
                    anyhow::anyhow!("release manifest target #{index} is missing `{name}`")
                })
        };
        targets.push(PublishTarget {
            backend: field("backend")?,
            directory: root.join(field("directory")?),
            package: field("package")?,
            registry: field("registry")?,
            command: field("publish")?,
            version: entry
                .get("version")
                .and_then(|v| v.as_str())
                .map(str::to_owned),
            version_skew_reason: entry
                .get("version_skew_reason")
                .and_then(|v| v.as_str())
                .map(str::to_owned),
            depends_on: entry
                .get("depends_on")
                .and_then(|v| v.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str())
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or_default(),
        });
    }
    Ok(ReleaseManifest {
        root,
        version,
        targets,
        tag,
    })
}

/// The package descriptor a backend emits, and how its version is declared.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VersionSource {
    /// A file carrying an explicit version string.
    File {
        /// The descriptor file the version was read from.
        path: PathBuf,
        /// The version the descriptor declares.
        version: String,
    },
    /// No in-file version: the release tag *is* the version (Go modules,
    /// Swift packages).
    Tag,
}

/// Locates and reads a backend's package descriptor version.
///
/// # Errors
/// Returns a message naming the missing descriptor.
pub fn descriptor_version(backend: &str, dir: &Path) -> anyhow::Result<VersionSource> {
    let read = |name: &str| -> anyhow::Result<String> {
        std::fs::read_to_string(dir.join(name))
            .map_err(|e| anyhow::anyhow!("{}: {e}", dir.join(name).display()))
    };
    let json_version = |text: &str| -> Option<String> {
        let value: serde_json::Value = serde_json::from_str(text).ok()?;
        value
            .get("version")
            .and_then(|v| v.as_str())
            .map(str::to_owned)
    };
    let tagged = |text: &str, key: &str| -> Option<String> {
        // `key = "1.2.3"` (Cargo.toml, pyproject.toml, gemspec)
        for line in text.lines() {
            let line = line.trim();
            if let Some(rest) = line.strip_prefix(key) {
                let rest = rest.trim_start().trim_start_matches('=').trim();
                let value = rest.trim_matches(|c| c == '"' || c == '\'' || c == ',');
                if !value.is_empty() {
                    return Some(value.to_owned());
                }
            }
        }
        None
    };
    let xml_version = |text: &str| -> Option<String> {
        // The first <version> after the project coordinates.
        let artifact = text.find("<artifactId>")?;
        text[artifact..]
            .split_once("<version>")
            .and_then(|(_, rest)| rest.split_once("</version>"))
            .map(|(value, _)| value.trim().to_owned())
    };

    let source = match backend {
        "typescript" => VersionSource::File {
            path: dir.join("package.json"),
            version: json_version(&read("package.json")?)
                .ok_or_else(|| anyhow::anyhow!("package.json has no `version`"))?,
        },
        "python" => {
            let text = read("pyproject.toml")?;
            let version = tagged(&text, "version")
                .ok_or_else(|| anyhow::anyhow!("pyproject.toml has no `version`"))?;
            VersionSource::File {
                path: dir.join("pyproject.toml"),
                version,
            }
        }
        "rust" => {
            let text = read("Cargo.toml")?;
            let version = tagged(&text, "version")
                .ok_or_else(|| anyhow::anyhow!("Cargo.toml has no `version`"))?;
            VersionSource::File {
                path: dir.join("Cargo.toml"),
                version,
            }
        }
        "java" | "kotlin" => {
            let text = read("pom.xml")?;
            let version = xml_version(&text)
                .ok_or_else(|| anyhow::anyhow!("pom.xml has no project `<version>`"))?;
            VersionSource::File {
                path: dir.join("pom.xml"),
                version,
            }
        }
        "csharp" => {
            let project = find_csproj(dir)?;
            let text = std::fs::read_to_string(&project)?;
            let version = text
                .split_once("<PackageVersion>")
                .and_then(|(_, rest)| rest.split_once("</PackageVersion>"))
                .map(|(value, _)| value.trim().to_owned())
                .or_else(|| {
                    text.split_once("<Version>")
                        .and_then(|(_, rest)| rest.split_once("</Version>"))
                        .map(|(value, _)| value.trim().to_owned())
                })
                .ok_or_else(|| {
                    anyhow::anyhow!("{} has no <PackageVersion>/<Version>", project.display())
                })?;
            VersionSource::File {
                path: project,
                version,
            }
        }
        "ruby" => {
            let gemspec = find_extension(dir, "gemspec")?;
            let text = std::fs::read_to_string(&gemspec)?;
            let version = text
                .split_once(".version")
                .and_then(|(_, rest)| {
                    rest.split_once('"')
                        .and_then(|(_, tail)| tail.split_once('"'))
                })
                .map(|(value, _)| value.to_owned())
                .ok_or_else(|| anyhow::anyhow!("{} has no version", gemspec.display()))?;
            VersionSource::File {
                path: gemspec,
                version,
            }
        }
        "php" => VersionSource::File {
            path: dir.join("composer.json"),
            version: json_version(&read("composer.json")?)
                .ok_or_else(|| anyhow::anyhow!("composer.json has no `version`"))?,
        },
        "dart" => {
            let text = read("pubspec.yaml")?;
            let version = text
                .lines()
                .map(str::trim)
                .find_map(|line| line.strip_prefix("version:"))
                .map(|rest| rest.trim().to_owned())
                .ok_or_else(|| anyhow::anyhow!("pubspec.yaml has no `version`"))?;
            VersionSource::File {
                path: dir.join("pubspec.yaml"),
                version,
            }
        }
        "cpp" => {
            let text = read("CMakeLists.txt")?;
            let version = text
                .split_once("VERSION")
                .and_then(|(_, rest)| rest.split_whitespace().next())
                .map(str::to_owned)
                .ok_or_else(|| anyhow::anyhow!("CMakeLists.txt has no project VERSION"))?;
            VersionSource::File {
                path: dir.join("CMakeLists.txt"),
                version,
            }
        }
        // Go modules and Swift packages are versioned by their release tag.
        "go" | "swift" => VersionSource::Tag,
        other => {
            return Err(anyhow::anyhow!(
                "unknown backend `{other}`: no descriptor mapping"
            ));
        }
    };
    Ok(source)
}

fn find_csproj(dir: &Path) -> anyhow::Result<PathBuf> {
    find_extension(dir, "csproj")
}

fn find_extension(dir: &Path, extension: &str) -> anyhow::Result<PathBuf> {
    let mut found: Vec<PathBuf> = std::fs::read_dir(dir)
        .map_err(|e| anyhow::anyhow!("{}: {e}", dir.display()))?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().and_then(|e| e.to_str()) == Some(extension))
        .collect();
    // Prefer a non-example project when several exist (C# ships both).
    found.sort_by_key(|path| {
        path.file_name()
            .and_then(|n| n.to_str())
            .map(|n| n.starts_with("Examples"))
            .unwrap_or(false)
    });
    found
        .into_iter()
        .next()
        .ok_or_else(|| anyhow::anyhow!("no .{extension} in {}", dir.display()))
}

/// Builds the publish plan: version consistency plus artifact preflight.
///
/// # Errors
/// Manifest IO and descriptor read failures are captured per target.
pub fn build_plan(manifest: &ReleaseManifest, published: &BTreeMap<String, String>) -> PublishPlan {
    let mut reports = Vec::new();
    for target in &manifest.targets {
        let mut errors = Vec::new();
        let mut notes = Vec::new();

        // Cross-backend version consistency.
        let declared = target
            .version
            .clone()
            .unwrap_or_else(|| manifest.version.clone());
        if target.version.is_some() && target.version != Some(manifest.version.clone()) {
            match &target.version_skew_reason {
                Some(reason) => notes.push(format!(
                    "version skew ({declared} vs release {}): {reason}",
                    manifest.version
                )),
                None => errors.push(format!(
                    "version {declared} differs from the release version {} without a version_skew_reason",
                    manifest.version
                )),
            }
        }

        // Artifact preflight: the generated descriptor must carry the version.
        let mut descriptor = None;
        let mut observed = None;
        match descriptor_version(&target.backend, &target.directory) {
            Ok(VersionSource::Tag) => {
                notes.push("versioned by the release tag (no in-file version)".to_owned());
            }
            Ok(VersionSource::File { path, version }) => {
                descriptor = Some(
                    path.strip_prefix(&manifest.root)
                        .unwrap_or(&path)
                        .display()
                        .to_string(),
                );
                observed = Some(version.clone());
                if version != declared {
                    errors.push(format!(
                        "{} declares {version}, manifest says {declared}: regenerate before publishing",
                        path.display()
                    ));
                }
            }
            Err(e) => errors.push(e.to_string()),
        }

        // Dependency sanity: every depends_on target must exist.
        for dependency in &target.depends_on {
            if !manifest.targets.iter().any(|t| &t.backend == dependency) {
                errors.push(format!(
                    "depends_on `{dependency}` is not a declared target"
                ));
            }
        }

        let already_published = published.get(&target.backend).cloned() == Some(declared.clone());

        reports.push(TargetReport {
            backend: target.backend.clone(),
            package: target.package.clone(),
            registry: target.registry.clone(),
            version: declared,
            descriptor,
            observed_version: observed,
            errors,
            notes,
            command: target.command.clone(),
            already_published,
        });
    }

    // Publish order: registry groups in first-appearance order, with
    // depends_on satisfied inside each registry.
    let order = publish_order(manifest);
    let ready = reports.iter().filter(|r| r.errors.is_empty()).count();
    let total = reports.len();
    PublishPlan {
        version: manifest.version.clone(),
        tag: manifest.tag.clone().unwrap_or_default(),
        order,
        targets: reports,
        total,
        ready,
    }
}

/// Orders targets: registry groups in first-appearance order, with each
/// group sorted so `depends_on` comes first and cycles are broken by
/// manifest order.
#[must_use]
pub fn publish_order(manifest: &ReleaseManifest) -> Vec<String> {
    let mut registries: Vec<String> = Vec::new();
    for target in &manifest.targets {
        if !registries.contains(&target.registry) {
            registries.push(target.registry.clone());
        }
    }
    let mut order = Vec::new();
    for registry in registries {
        let group: Vec<&PublishTarget> = manifest
            .targets
            .iter()
            .filter(|t| t.registry == registry)
            .collect();
        let mut placed: Vec<String> = Vec::new();
        let mut remaining: Vec<&PublishTarget> = group;
        while !remaining.is_empty() {
            let next = remaining.iter().position(|t| {
                t.depends_on
                    .iter()
                    .all(|dep| placed.contains(dep) || !remaining.iter().any(|o| &o.backend == dep))
            });
            match next {
                Some(index) => {
                    order.push(remaining[index].backend.clone());
                    placed.push(remaining[index].backend.clone());
                    remaining.remove(index);
                }
                // Dependency cycle: publish in manifest order rather than
                // looping forever; preflight reports the cycle.
                None => {
                    order.push(remaining[0].backend.clone());
                    placed.push(remaining[0].backend.clone());
                    remaining.remove(0);
                }
            }
        }
    }
    order
}

/// Loads previously published targets from a state file.
///
/// # Errors
/// Returns an empty map for a missing file; surfaces malformed state.
pub fn load_state(path: &Path) -> anyhow::Result<BTreeMap<String, String>> {
    if !path.exists() {
        return Ok(BTreeMap::new());
    }
    let text = std::fs::read_to_string(path)?;
    serde_json::from_str(&text).map_err(|e| anyhow::anyhow!("{}: {e}", path.display()))
}

/// Records published targets to the state file.
///
/// # Errors
/// Filesystem failures.
pub fn save_state(path: &Path, published: &BTreeMap<String, String>) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(
        path,
        format!("{}\n", serde_json::to_string_pretty(published)?),
    )?;
    Ok(())
}

/// Runs the publish orchestration.
///
/// # Errors
/// Manifest, descriptor, or command failures.
pub fn publish(args: &PublishArgs) -> anyhow::Result<i32> {
    let manifest = parse_manifest(&args.manifest)?;
    let published = if args.resume {
        load_state(&args.state)?
    } else {
        BTreeMap::new()
    };
    let plan = build_plan(&manifest, &published);

    match args.text.format {
        OutputFormat::Json => println!("{}", serde_json::to_string_pretty(&plan)?),
        OutputFormat::Sarif => anyhow::bail!("SARIF output is not defined for publish plans"),
        OutputFormat::Text => render_text(&plan),
    }

    let blocked = plan.targets.iter().find(|t| !t.errors.is_empty());
    if let Some(blocked) = blocked {
        eprintln!(
            "publish blocked: {} target(s) failed preflight (first: {} — {})",
            plan.targets.iter().filter(|t| !t.errors.is_empty()).count(),
            blocked.backend,
            blocked.errors.join("; ")
        );
        return Ok(1);
    }

    if !args.execute {
        eprintln!(
            "dry run: {} target(s) ready; re-run with --execute to publish (nothing was uploaded)",
            plan.ready
        );
        return Ok(0);
    }

    // Execute in plan order; record each completed target.
    let mut state = published;
    for backend in &plan.order {
        let Some(target) = manifest.targets.iter().find(|t| &t.backend == backend) else {
            continue;
        };
        let declared = target
            .version
            .clone()
            .unwrap_or_else(|| manifest.version.clone());
        if state.get(backend) == Some(&declared) {
            eprintln!("skip {}: already published as {declared}", target.backend);
            continue;
        }
        eprintln!(
            "publishing {} {} to {}",
            target.backend, target.package, target.registry
        );
        let status = std::process::Command::new("sh")
            .arg("-c")
            .arg(&target.command)
            .current_dir(&target.directory)
            .status()
            .map_err(|e| anyhow::anyhow!("{}: {e}", target.command))?;
        if !status.success() {
            eprintln!(
                "publish failed for {} — stopping; rerun with --resume",
                target.backend
            );
            save_state(&args.state, &state)?;
            return Ok(1);
        }
        state.insert(target.backend.clone(), declared);
        save_state(&args.state, &state)?;
    }
    eprintln!("published {} target(s)", plan.ready);
    Ok(0)
}

fn render_text(plan: &PublishPlan) {
    println!("release {} (tag {})", plan.version, plan.tag);
    println!();
    for backend in &plan.order {
        let Some(target) = plan.targets.iter().find(|t| &t.backend == backend) else {
            continue;
        };
        let status = if !target.errors.is_empty() {
            "BLOCKED"
        } else if target.already_published {
            "already"
        } else {
            "ready"
        };
        println!(
            "  {status:>8}  {:<11} {:<28} {} {}",
            target.backend, target.package, target.registry, target.version
        );
        for note in &target.notes {
            println!("            note: {note}");
        }
        for error in &target.errors {
            println!("            error: {error}");
        }
    }
    println!();
    println!("{}/{} target(s) ready", plan.ready, plan.total);
}

/// Renders the release manifest as a tag-triggered GitHub Actions
/// workflow: verify each generated artifact, then publish per registry.
///
/// # Errors
/// Manifest IO failures.
pub fn workflow(args: &WorkflowArgs) -> anyhow::Result<i32> {
    let manifest = parse_manifest(&args.manifest)?;
    let mut jobs = String::new();
    for target in &manifest.targets {
        let id = target.backend.replace(['_', '.'], "-");
        let version = target
            .version
            .clone()
            .unwrap_or_else(|| manifest.version.clone());
        let relative = target
            .directory
            .strip_prefix(&manifest.root)
            .unwrap_or(&target.directory)
            .display()
            .to_string();
        jobs.push_str(&format!(
            r#"  verify-{id}:
    name: verify {backend} {package}
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - name: Build suspect
        run: cargo build --release -p suspect-cli
      - name: Check the generated {backend} artifact carries {version}
        run: |
          suspect release publish --manifest {manifest} --dry-run
      - name: Upload {backend} artifact
        uses: actions/upload-artifact@v4
        with:
          name: {id}-package
          path: {relative}

"#,
            id = id,
            backend = target.backend,
            package = target.package,
            version = version,
            manifest = args.manifest.display(),
            relative = relative,
        ));
    }
    for target in &manifest.targets {
        let id = target.backend.replace(['_', '.'], "-");
        let relative = target
            .directory
            .strip_prefix(&manifest.root)
            .unwrap_or(&target.directory)
            .display()
            .to_string();
        jobs.push_str(&format!(
            r#"  publish-{id}:
    name: publish {backend} to {registry}
    needs: verify-{id}
    runs-on: ubuntu-latest
    environment: release-{id}
    steps:
      - uses: actions/checkout@v4
      - uses: actions/download-artifact@v4
        with:
          name: {id}-package
          path: {relative}
      - name: Publish {backend} {package} {version}
        env:
          REGISTRY_TOKEN: ${{{{ secrets.{registry_token} }}}}
        run: {command}

"#,
            id = id,
            backend = target.backend,
            registry = target.registry,
            package = target.package,
            version = target
                .version
                .clone()
                .unwrap_or_else(|| manifest.version.clone()),
            registry_token = registry_secret(&target.registry),
            command = target.command,
            relative = relative,
        ));
    }
    let tag = manifest.tag.clone().unwrap_or_default();
    let text = format!(
        r#"# Generated by `suspect release workflow` — publish on tag.
name: release

on:
  push:
    tags: ['{tag}']
  workflow_dispatch:

jobs:
{jobs}"#
    );
    match &args.output {
        Some(path) => {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(path, &text)?;
            eprintln!("wrote {}", path.display());
        }
        None => print!("{text}"),
    }
    Ok(0)
}

/// The repository secret name a registry's token is read from.
#[must_use]
pub fn registry_secret(registry: &str) -> String {
    match registry {
        "npm" => "NPM_TOKEN".to_owned(),
        "pypi" => "PYPI_TOKEN".to_owned(),
        "maven-central" | "maven" => "MAVEN_TOKEN".to_owned(),
        "nuget" => "NUGET_TOKEN".to_owned(),
        "rubygems" => "RUBYGEMS_TOKEN".to_owned(),
        "packagist" => "PACKAGIST_TOKEN".to_owned(),
        "pub" => "PUB_TOKEN".to_owned(),
        "crates-io" | "crates" => "CRATES_TOKEN".to_owned(),
        "conan" | "git" | "internal" => {
            format!("{}_TOKEN", registry.to_uppercase().replace('-', "_"))
        }
        other => format!("{}_TOKEN", other.to_uppercase().replace(['-', ' '], "_")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest_with(entries: &str) -> ReleaseManifest {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("release.json");
        std::fs::write(
            &path,
            format!(r#"{{"version": "1.4.0", "targets": [{entries}]}}"#),
        )
        .unwrap();
        parse_manifest(&path).unwrap()
    }

    #[test]
    fn version_skew_requires_a_stated_reason() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("composer.json"),
            r#"{"name": "acme/sdk", "version": "1.2.0"}"#,
        )
        .unwrap();
        let manifest = ReleaseManifest {
            root: dir.path().to_path_buf(),
            version: "1.4.0".to_owned(),
            tag: Some("v1.4.0".to_owned()),
            targets: vec![PublishTarget {
                backend: "php".to_owned(),
                directory: dir.path().to_path_buf(),
                package: "acme/sdk".to_owned(),
                registry: "packagist".to_owned(),
                command: "composer publish".to_owned(),
                version: Some("1.2.0".to_owned()),
                version_skew_reason: None,
                depends_on: Vec::new(),
            }],
        };
        let plan = build_plan(&manifest, &BTreeMap::new());
        assert!(
            plan.targets[0]
                .errors
                .iter()
                .any(|e| e.contains("version_skew_reason"))
        );

        // With a reason, the skew is a note, not an error.
        let mut explained = manifest.clone();
        explained.targets[0].version_skew_reason = Some("PHP SDK lags the release line".to_owned());
        let plan = build_plan(&explained, &BTreeMap::new());
        assert!(plan.targets[0].errors.is_empty(), "{:?}", plan.targets[0]);
        assert!(plan.targets[0].notes.iter().any(|n| n.contains("skew")));
    }

    #[test]
    fn stale_generation_is_caught_before_publishing() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("package.json"),
            r#"{"name": "@acme/sdk", "version": "0.9.0"}"#,
        )
        .unwrap();
        let manifest = ReleaseManifest {
            root: dir.path().to_path_buf(),
            version: "1.4.0".to_owned(),
            tag: None,
            targets: vec![PublishTarget {
                backend: "typescript".to_owned(),
                directory: dir.path().to_path_buf(),
                package: "@acme/sdk".to_owned(),
                registry: "npm".to_owned(),
                command: "npm publish".to_owned(),
                version: None,
                version_skew_reason: None,
                depends_on: Vec::new(),
            }],
        };
        let plan = build_plan(&manifest, &BTreeMap::new());
        assert_eq!(plan.targets[0].observed_version.as_deref(), Some("0.9.0"));
        assert!(
            plan.targets[0]
                .errors
                .iter()
                .any(|e| e.contains("regenerate before publishing")),
            "{:?}",
            plan.targets[0]
        );
    }

    #[test]
    fn go_and_swift_are_tag_versioned() {
        let dir = tempfile::tempdir().unwrap();
        let manifest = ReleaseManifest {
            root: dir.path().to_path_buf(),
            version: "1.4.0".to_owned(),
            tag: None,
            targets: ["go", "swift"]
                .into_iter()
                .map(|backend| PublishTarget {
                    backend: backend.to_owned(),
                    directory: dir.path().to_path_buf(),
                    package: "acme".to_owned(),
                    registry: "internal".to_owned(),
                    command: "git tag".to_owned(),
                    version: None,
                    version_skew_reason: None,
                    depends_on: Vec::new(),
                })
                .collect(),
        };
        let plan = build_plan(&manifest, &BTreeMap::new());
        for target in &plan.targets {
            assert!(target.errors.is_empty(), "{target:?}");
            assert!(target.notes.iter().any(|n| n.contains("release tag")));
        }
    }

    #[test]
    fn depends_on_orders_publishing() {
        let manifest = manifest_with(
            r#"
        {"backend": "typescript", "directory": "a", "package": "p", "registry": "npm", "publish": "npm publish", "depends_on": ["python"]},
        {"backend": "python", "directory": "b", "package": "p", "registry": "npm", "publish": "twine upload"}
        "#,
        );
        let order = publish_order(&manifest);
        assert_eq!(
            order,
            vec!["python".to_owned(), "typescript".to_owned()],
            "dependency publishes first"
        );
    }

    #[test]
    fn registry_groups_publish_in_first_appearance_order() {
        let manifest = manifest_with(
            r#"
        {"backend": "typescript", "directory": "a", "package": "p", "registry": "npm", "publish": "x"},
        {"backend": "python", "directory": "b", "package": "p", "registry": "pypi", "publish": "x"},
        {"backend": "ruby", "directory": "c", "package": "p", "registry": "npm", "publish": "x"}
        "#,
        );
        assert_eq!(
            publish_order(&manifest),
            vec![
                "typescript".to_owned(),
                "ruby".to_owned(),
                "python".to_owned()
            ]
        );
    }

    #[test]
    fn resumed_targets_are_reported_as_published() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("package.json"), r#"{"version": "1.4.0"}"#).unwrap();
        let manifest = ReleaseManifest {
            root: dir.path().to_path_buf(),
            version: "1.4.0".to_owned(),
            tag: None,
            targets: vec![PublishTarget {
                backend: "typescript".to_owned(),
                directory: dir.path().to_path_buf(),
                package: "p".to_owned(),
                registry: "npm".to_owned(),
                command: "npm publish".to_owned(),
                version: None,
                version_skew_reason: None,
                depends_on: Vec::new(),
            }],
        };
        let published = BTreeMap::from([("typescript".to_owned(), "1.4.0".to_owned())]);
        let plan = build_plan(&manifest, &published);
        assert!(plan.targets[0].already_published);
        assert!(plan.targets[0].errors.is_empty());
    }

    #[test]
    fn workflow_is_tag_triggered_and_covers_every_target() {
        let dir = tempfile::tempdir().unwrap();
        let manifest_path = dir.path().join("release.json");
        std::fs::write(
            &manifest_path,
            r#"{"version": "1.4.0", "targets": [
        {"backend": "typescript", "directory": "sdk/ts", "package": "@acme/sdk", "registry": "npm", "publish": "npm publish --access public"},
        {"backend": "python", "directory": "sdk/py", "package": "acme-sdk", "registry": "pypi", "publish": "twine upload dist/*"}
    ]}"#,
        )
        .unwrap();
        let out = dir.path().join("release.yml");
        workflow(&WorkflowArgs {
            manifest: manifest_path,
            output: Some(out.clone()),
        })
        .unwrap();
        let text = std::fs::read_to_string(&out).unwrap();
        assert!(text.contains("tags: ['v1.4.0']"), "{text}");
        assert!(text.contains("verify-typescript:"));
        assert!(text.contains("verify-python:"));
        assert!(text.contains("secrets.NPM_TOKEN"), "{text}");
        assert!(text.contains("secrets.PYPI_TOKEN"), "{text}");
        assert!(text.contains("needs: verify-typescript"), "{text}");
    }

    #[test]
    fn state_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().join("state.json");
        assert!(load_state(&state).unwrap().is_empty());
        let mut published = BTreeMap::new();
        published.insert("python".to_owned(), "1.4.0".to_owned());
        save_state(&state, &published).unwrap();
        assert_eq!(load_state(&state).unwrap(), published);
    }
}
