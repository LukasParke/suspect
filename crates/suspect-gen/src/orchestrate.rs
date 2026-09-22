//! Manifest parsing and preservation-aware output orchestration.
//!
//! A [`Manifest`] lists output rules (template + target path). [`render_manifest`]
//! renders every target, splices preserved user-code regions from the
//! existing file back into the freshly rendered content, compares bytes
//! directly, and only rewrites files that actually changed. With
//! `diff_only = true` nothing is written; unified diffs are returned per
//! changed file instead.

use std::fs;
use std::path::{Path, PathBuf};

use suspect_artifact::{Adoption, Artifact, ArtifactBatch, Change, OwnershipChangeKind};

use crate::{GenError, PreparedContext, TemplateEngine};

/// Opening marker of a preserved user-code region.
///
/// Templates wrap user-owned regions in comments containing this string.
pub const BEGIN_MARK: &str = "suspect:begin:user-code";

/// Closing marker of a preserved user-code region.
pub const END_MARK: &str = "suspect:end:user-code";

/// One generation output rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutputRule {
    /// Name of the template to render.
    pub template: String,
    /// Target path relative to the output root; may contain `{{ }}`
    /// expressions evaluated against the render context.
    pub target: String,
}

/// A parsed generation manifest (`gen.toml`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Manifest {
    /// All `[[output]]` rules in file order.
    pub outputs: Vec<OutputRule>,
}

/// Parses manifest TOML text into a [`Manifest`].
///
/// Only a pragmatic subset is supported: `[output]` / `[[output]]` table
/// headers with `template` and `target` string keys, full-line `#`
/// comments, and blank lines. Other tables are ignored.
///
/// # Errors
/// When an output rule is missing its `template` or `target` key.
pub fn parse_manifest(text: &str) -> Result<Manifest, GenError> {
    let mut outputs = Vec::new();
    let mut current: Option<OutputRule> = None;
    let mut in_output_table = false;
    for raw_line in text.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with("[[") || line.starts_with('[') {
            let end = line
                .find(']')
                .ok_or_else(|| GenError(format!("unterminated table header: {line}")))?;
            let header = line[1..end].trim().trim_start_matches('[').to_owned();
            if header == "output" {
                if let Some(rule) = current.take() {
                    outputs.push(rule);
                }
                in_output_table = true;
                current = Some(OutputRule {
                    template: String::new(),
                    target: String::new(),
                });
            } else {
                if let Some(rule) = current.take() {
                    outputs.push(rule);
                }
                in_output_table = false;
            }
            continue;
        }
        if !in_output_table {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        let value = unquote(value.trim());
        if let Some(rule) = current.as_mut() {
            match key {
                "template" => rule.template = value,
                "target" => rule.target = value,
                _ => {}
            }
        }
    }
    if let Some(rule) = current.take() {
        outputs.push(rule);
    }
    for rule in &outputs {
        if rule.template.is_empty() || rule.target.is_empty() {
            return Err(GenError(format!(
                "output rule {:?} must set both 'template' and 'target'",
                rule.target
            )));
        }
    }
    Ok(Manifest { outputs })
}

/// Strips surrounding quotes and inline comments from a TOML value token.
///
/// The quote style is detected first: single-quoted strings are returned
/// verbatim (backslashes included), double-quoted strings honor `\\` and
/// `\"` escapes, and an inline `#` comment is stripped only from
/// unquoted values — a `#` inside quotes is data (e.g. `"gen #core.rs"`
/// or a Windows path).
fn unquote(value: &str) -> String {
    let s = value.trim();
    let bytes = s.as_bytes();
    if !bytes.is_empty() && (bytes[0] == b'"' || bytes[0] == b'\'') {
        let quote = bytes[0];
        let mut i = 1;
        while i < bytes.len() {
            let b = bytes[i];
            if quote == b'"' && b == b'\\' {
                i += 2;
                continue;
            }
            if b == quote {
                let inner = &s[1..i];
                return if quote == b'"' {
                    unescape_double_quoted(inner)
                } else {
                    inner.to_owned()
                };
            }
            i += 1;
        }
        // Unterminated quote: fall through and treat the token as plain text.
    }
    // Unquoted: a whitespace-preceded `#` starts an inline comment.
    s.split(" #").next().unwrap_or(s).trim().to_owned()
}

/// Unescapes `\\` and `\"` inside a double-quoted TOML value; any other
/// backslash sequence is kept verbatim.
fn unescape_double_quoted(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('\\') => out.push('\\'),
                Some('"') => out.push('"'),
                Some(other) => {
                    out.push('\\');
                    out.push(other);
                }
                None => out.push('\\'),
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Loads and parses a manifest from `path`.
///
/// # Errors
/// On I/O failure or malformed manifest content.
/// Parses a manifest from raw TOML text (embedded preset manifests).
///
/// # Errors
/// Same as [`parse_manifest`].
pub fn parse_manifest_str(text: &str) -> Result<Manifest, GenError> {
    parse_manifest(text)
}

/// Loads and parses a manifest file from disk.
///
/// # Errors
/// Propagates read errors and manifest parse errors.
pub fn load_manifest(path: &Path) -> Result<Manifest, GenError> {
    let text = fs::read_to_string(path)?;
    parse_manifest(&text)
}

/// Why a rendered file was or was not written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteReason {
    /// The target did not exist and was created.
    Created,
    /// The target existed with different content and was rewritten.
    Changed,
    /// The rendered content matches the existing file byte-for-byte.
    Unchanged,
    /// Preserved regions were spliced in before writing.
    PreservedRegionsApplied,
    /// An obsolete, byte-identical owned artifact was or would be removed.
    Removed,
    /// An obsolete ownership entry referred to an already absent file.
    ObsoleteMissing,
    /// Ownership or user edits prevent changing this path.
    OwnershipConflict,
}

/// The result of rendering one manifest output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderOutcome {
    /// Absolute path of the target file.
    pub path: PathBuf,
    /// Whether the file was actually written.
    pub wrote: bool,
    /// Classification of what happened.
    pub reason: WriteReason,
    /// Unified diff between the on-disk and rendered content; populated
    /// for changed files when rendering with `diff_only`.
    pub diff: Option<String>,
    /// Why a user file or retained user content prevents generation.
    pub conflict: Option<String>,
}

/// Renders every output in `manifest` under `out_root`.
///
/// Target paths are template-evaluated against `ctx`. When the target
/// already exists, user-code regions delimited by [`BEGIN_MARK`] /
/// [`END_MARK`] markers are carried over into the new content before the
/// byte comparison decides between rewriting and skipping. With
/// `diff_only = true` no file is ever written; instead each changed file's
/// outcome carries a unified diff in [`RenderOutcome::diff`].
///
/// # Errors
/// When template rendering fails, a rendered target is absolute or
/// escapes `out_root`, either file has malformed preservation markers,
/// or writing (non-diff mode) fails.
pub fn render_manifest(
    engine: &dyn TemplateEngine,
    manifest: &Manifest,
    ctx: &serde_json::Value,
    out_root: &Path,
    diff_only: bool,
) -> Result<Vec<RenderOutcome>, GenError> {
    render_manifest_owned(
        engine,
        manifest,
        ctx,
        out_root,
        diff_only,
        "suspect-gen:default",
        Adoption::Refuse,
    )
}

/// Render one stable logical owner's complete output set. Use distinct owners
/// when several presets or custom manifests share an output root. Ownership
/// identity must remain stable when output rules or checkout location change.
///
/// `Adoption::Identical` explicitly adopts only byte-identical preexisting files;
/// the default API never takes over unowned output. Metadata participates in
/// drift outcomes, and preserved user content is protected from obsolete deletion.
///
/// # Errors
/// Rendering/path/marker errors, invalid ownership metadata, user-file conflicts
/// in write mode, or filesystem failures. Diff mode never writes output.
pub fn render_manifest_owned(
    engine: &dyn TemplateEngine,
    manifest: &Manifest,
    ctx: &serde_json::Value,
    out_root: &Path,
    diff_only: bool,
    owner: &str,
    adoption: Adoption,
) -> Result<Vec<RenderOutcome>, GenError> {
    let prepared = engine.prepare_context(ctx);
    let mut rendered = Vec::with_capacity(manifest.outputs.len());
    for rule in &manifest.outputs {
        rendered.push((
            PathBuf::from(render_inline(&rule.target, &prepared)?),
            engine.render_prepared(&rule.template, &prepared)?,
        ));
    }
    let mut batch = ArtifactBatch::prepare(
        out_root,
        rendered.iter().map(|(path, content)| Artifact {
            path,
            content: content.as_bytes(),
        }),
    )
    .map_err(|error| GenError(error.to_string()))?;
    let out_root = batch.root().to_path_buf();
    let mut preserved_paths = std::collections::BTreeSet::new();
    for file in batch.files_mut() {
        let path = out_root.join(file.path());
        let old = artifact_text(file.existing().unwrap_or(b""), &path)?;
        let fresh = artifact_text(file.content(), &path)?;
        let (existing_managed, old_regions) = managed_regions(old)?;
        let (_, new_regions) = managed_regions(fresh)?;
        if old_regions > 0 && old_regions != new_regions {
            return Err(GenError(format!(
                "{}: changing the number of preserved user-code regions requires an explicit content migration",
                path.display()
            )));
        }
        let (spliced, count) = splice_preserved_regions(old, fresh)?;
        let retained = count > 0 && spliced != fresh;
        let (planned_managed, _) = managed_regions(&spliced)?;
        if count > 0 {
            preserved_paths.insert(file.path().to_path_buf());
        }
        file.replace_content(spliced.into_bytes());
        file.set_managed_content(
            "suspect-user-code-v1",
            &existing_managed,
            &planned_managed,
            retained,
        );
    }
    let batch = batch
        .with_ownership(owner, adoption)
        .map_err(|error| GenError(error.to_string()))?;
    let mut outcomes = Vec::new();
    for file in batch.files() {
        let path = out_root.join(file.path());
        let change = file.change();
        let finding = batch
            .report()
            .changes
            .iter()
            .find(|finding| finding.path == file.path())
            .expect("planned ownership finding");
        let reason = if finding.kind == OwnershipChangeKind::Conflict {
            WriteReason::OwnershipConflict
        } else {
            match change {
                Change::Created => WriteReason::Created,
                Change::Unchanged => WriteReason::Unchanged,
                Change::Changed if preserved_paths.contains(file.path()) => {
                    WriteReason::PreservedRegionsApplied
                }
                Change::Changed => WriteReason::Changed,
            }
        };
        let diff = if diff_only && change != Change::Unchanged {
            Some(unified_diff(
                artifact_text(file.existing().unwrap_or(b""), &path)?,
                artifact_text(file.content(), &path)?,
            ))
        } else {
            None
        };
        outcomes.push(RenderOutcome {
            path,
            wrote: !diff_only && change != Change::Unchanged,
            reason,
            diff,
            conflict: finding.conflict.clone(),
        });
    }
    for finding in &batch.report().changes {
        if batch.files().iter().any(|file| file.path() == finding.path) {
            continue;
        }
        let path = out_root.join(&finding.path);
        outcomes.push(RenderOutcome {
            path: path.clone(),
            wrote: !diff_only && finding.kind == OwnershipChangeKind::Obsolete,
            reason: match finding.kind {
                OwnershipChangeKind::Obsolete => WriteReason::Removed,
                OwnershipChangeKind::MissingObsolete => WriteReason::ObsoleteMissing,
                _ => WriteReason::OwnershipConflict,
            },
            diff: if diff_only && finding.kind == OwnershipChangeKind::Obsolete {
                Some(unified_diff(
                    artifact_text(batch.obsolete_content(&finding.path).unwrap_or(b""), &path)?,
                    "",
                ))
            } else {
                None
            },
            conflict: finding.conflict.clone(),
        });
    }
    let metadata = batch.manifest();
    let path = out_root.join(metadata.path());
    outcomes.push(RenderOutcome {
        path: path.clone(),
        wrote: !diff_only && metadata.change() != Change::Unchanged,
        reason: match metadata.change() {
            Change::Created => WriteReason::Created,
            Change::Changed => WriteReason::Changed,
            Change::Unchanged => WriteReason::Unchanged,
        },
        diff: if diff_only && metadata.change() != Change::Unchanged {
            Some(unified_diff(
                artifact_text(metadata.existing().unwrap_or(b""), &path)?,
                artifact_text(metadata.content(), &path)?,
            ))
        } else {
            None
        },
        conflict: None,
    });
    if !diff_only {
        batch
            .commit()
            .map_err(|error| GenError(error.to_string()))?;
    }
    Ok(outcomes)
}

fn managed_regions(content: &str) -> Result<(Vec<u8>, usize), GenError> {
    let lines: Vec<_> = content.split_inclusive('\n').collect();
    let pairs = marker_pairs(&lines)?;
    let mut managed = String::new();
    let mut cursor = 0;
    for &(begin, end) in &pairs {
        for line in &lines[cursor..=begin] {
            managed.push_str(line);
        }
        cursor = end;
    }
    for line in &lines[cursor..] {
        managed.push_str(line);
    }
    Ok((managed.into_bytes(), pairs.len()))
}

fn artifact_text<'a>(content: &'a [u8], path: &Path) -> Result<&'a str, GenError> {
    std::str::from_utf8(content)
        .map_err(|error| GenError(format!("artifact {} is not UTF-8: {error}", path.display())))
}

/// Evaluates `{{ ... }}` expressions in a target path against `ctx`.
fn render_inline(template: &str, ctx: &PreparedContext<'_>) -> Result<String, GenError> {
    let mut env = minijinja::Environment::new();
    env.add_template_owned("__target__", template)?;
    Ok(env
        .get_template("__target__")?
        .render(ctx.minijinja_value())?)
}

/// Recognizes a whole-line marker: after trimming, optionally stripping
/// leading comment tokens (`//` or `#`) and whitespace again, the line
/// must equal `mark` exactly. A line that merely *mentions* the marker
/// text is data, not a marker.
fn is_marker_line(line: &str, mark: &str) -> bool {
    let mut t = line.trim();
    while let Some(rest) = t.strip_prefix("//").or_else(|| t.strip_prefix('#')) {
        t = rest.trim_start();
    }
    t == mark
}

/// Locates `(begin_idx, end_idx)` line pairs whose begin line is a
/// whole-line [`BEGIN_MARK`] marker and whose matching later line is a
/// whole-line [`END_MARK`] marker (see [`is_marker_line`]).
///
/// # Errors
/// When markers are malformed: an [`END_MARK`] with no open region, a
/// [`BEGIN_MARK`] inside an already open region, or a [`BEGIN_MARK`]
/// never closed by end of input. Errors name the offending 1-based line.
fn marker_pairs(lines: &[&str]) -> Result<Vec<(usize, usize)>, GenError> {
    let mut pairs = Vec::new();
    let mut open: Option<usize> = None;
    for (i, line) in lines.iter().enumerate() {
        let lineno = i + 1;
        if is_marker_line(line, BEGIN_MARK) {
            if open.is_some() {
                return Err(GenError(format!(
                    "line {lineno}: '{}' inside an already open region",
                    BEGIN_MARK
                )));
            }
            open = Some(i);
        } else if is_marker_line(line, END_MARK) {
            match open.take() {
                Some(begin) => pairs.push((begin, i)),
                None => {
                    return Err(GenError(format!(
                        "line {lineno}: '{END_MARK}' without an open region"
                    )));
                }
            }
        }
    }
    if let Some(i) = open {
        return Err(GenError(format!(
            "line {}: '{BEGIN_MARK}' region is never closed",
            i + 1
        )));
    }
    Ok(pairs)
}

/// Splices the user code captured between each `old` begin/end marker
/// pair into `new_content` between the corresponding fresh pair; marker
/// lines themselves come from the fresh template. Pairs are matched
/// positionally. Returns the spliced content and how many regions were
/// applied.
///
/// # Errors
/// When either input has malformed markers (see [`marker_pairs`]).
pub(crate) fn splice_preserved_regions(
    old: &str,
    new_content: &str,
) -> Result<(String, usize), GenError> {
    let old_lines: Vec<&str> = old.split_inclusive('\n').collect();
    let new_lines: Vec<&str> = new_content.split_inclusive('\n').collect();
    let old_pairs = marker_pairs(&old_lines)?;
    let new_pairs = marker_pairs(&new_lines)?;
    let count = old_pairs.len().min(new_pairs.len());
    if count == 0 {
        return Ok((new_content.to_owned(), 0));
    }

    let mut out = String::new();
    let mut cursor = 0usize;
    for i in 0..count {
        let (ob, oe) = old_pairs[i];
        let (nb, ne) = new_pairs[i];
        for line in &new_lines[cursor..nb] {
            out.push_str(line);
        }
        out.push_str(new_lines[nb]);
        // splice the preserved user code
        for line in &old_lines[ob + 1..oe] {
            out.push_str(line);
        }
        out.push_str(new_lines[ne]);
        cursor = ne + 1;
    }
    for line in &new_lines[cursor.min(new_lines.len())..] {
        out.push_str(line);
    }
    Ok((out, count))
}

/// One line-level diff operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DiffOp<'a> {
    /// Line present in both inputs.
    Equal(&'a str),
    /// Line removed from the old input.
    Delete(&'a str),
    /// Line added by the new input.
    Insert(&'a str),
}

/// Computes a longest-common-subsequence line diff between `old` and `new`.
#[must_use]
fn line_diff<'a>(old: &'a [&'a str], new: &'a [&'a str]) -> Vec<DiffOp<'a>> {
    // lcs[i][j] = LCS length of old[i..] and new[j..]
    let mut lcs = vec![vec![0usize; new.len() + 1]; old.len() + 1];
    for i in (0..old.len()).rev() {
        for j in (0..new.len()).rev() {
            lcs[i][j] = if old[i] == new[j] {
                lcs[i + 1][j + 1] + 1
            } else {
                lcs[i + 1][j].max(lcs[i][j + 1])
            };
        }
    }
    let mut ops = Vec::new();
    let (mut i, mut j) = (0usize, 0usize);
    while i < old.len() && j < new.len() {
        if old[i] == new[j] {
            ops.push(DiffOp::Equal(old[i]));
            i += 1;
            j += 1;
        } else if lcs[i + 1][j] >= lcs[i][j + 1] {
            ops.push(DiffOp::Delete(old[i]));
            i += 1;
        } else {
            ops.push(DiffOp::Insert(new[j]));
            j += 1;
        }
    }
    ops.extend(old[i..].iter().map(|l| DiffOp::Delete(l)));
    ops.extend(new[j..].iter().map(|l| DiffOp::Insert(l)));
    ops
}

/// Renders a unified diff (hunk headers `@@ -a,b +c,d @@`, 3 context lines)
/// between `old` and `new` text. Purely textual: never touches the disk.
#[must_use]
pub fn unified_diff(old: &str, new: &str) -> String {
    const CONTEXT: usize = 3;
    let old_lines: Vec<&str> = old.lines().collect();
    let new_lines: Vec<&str> = new.lines().collect();
    let ops = line_diff(&old_lines, &new_lines);

    // Map each op to its position in the respective inputs.
    #[derive(Clone, Copy)]
    struct Pos {
        old_line: Option<usize>,
        new_line: Option<usize>,
    }
    let mut positions = Vec::with_capacity(ops.len());
    let (mut oi, mut ni) = (0usize, 0usize);
    for op in &ops {
        let pos = match op {
            DiffOp::Equal(_) => {
                let p = Pos {
                    old_line: Some(oi),
                    new_line: Some(ni),
                };
                oi += 1;
                ni += 1;
                p
            }
            DiffOp::Delete(_) => {
                let p = Pos {
                    old_line: Some(oi),
                    new_line: None,
                };
                oi += 1;
                p
            }
            DiffOp::Insert(_) => {
                let p = Pos {
                    old_line: None,
                    new_line: Some(ni),
                };
                ni += 1;
                p
            }
        };
        positions.push(pos);
    }

    let changed: Vec<usize> = (0..ops.len())
        .filter(|&k| !matches!(ops[k], DiffOp::Equal(_)))
        .collect();
    if changed.is_empty() {
        return String::new();
    }

    // Group changed indices into hunk ranges padded with context.
    let mut hunks: Vec<(usize, usize)> = Vec::new();
    for &k in &changed {
        match hunks.last_mut() {
            Some((_, end)) if k <= *end + 2 * CONTEXT => *end = k + CONTEXT,
            _ => hunks.push((k.saturating_sub(CONTEXT), k + CONTEXT)),
        }
    }

    let mut out = String::new();
    for &(start, end) in &hunks {
        let end = end.min(ops.len() - 1);
        let slice = &ops[start..=end];
        let old_count = slice
            .iter()
            .filter(|op| !matches!(op, DiffOp::Insert(_)))
            .count();
        let new_count = slice
            .iter()
            .filter(|op| !matches!(op, DiffOp::Delete(_)))
            .count();
        // Derive the first shown old/new line from the slice contents:
        // a hunk that starts on an Insert has no old line (and vice
        // versa) at `start`, so scan for the first present position.
        let first_old = positions[start..=end]
            .iter()
            .find_map(|p| p.old_line)
            .map_or(0, |l| l + 1);
        let first_new = positions[start..=end]
            .iter()
            .find_map(|p| p.new_line)
            .map_or(0, |l| l + 1);
        out.push_str(&format!(
            "@@ -{},{} +{},{} @@\n",
            if old_count == 0 {
                first_old.saturating_sub(1)
            } else {
                first_old
            },
            old_count,
            if new_count == 0 {
                first_new.saturating_sub(1)
            } else {
                first_new
            },
            new_count,
        ));
        for op in slice {
            match op {
                DiffOp::Equal(l) => out.push_str(&format!(" {l}\n")),
                DiffOp::Delete(l) => out.push_str(&format!("-{l}\n")),
                DiffOp::Insert(l) => out.push_str(&format!("+{l}\n")),
            }
        }
    }
    out
}
