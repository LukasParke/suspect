//! Manifest parsing and preservation-aware output orchestration.
//!
//! A [`Manifest`] lists output rules (template + target path). [`render_manifest`]
//! renders every target, splices preserved user-code regions from the
//! existing file back into the freshly rendered content, compares content
//! hashes, and only rewrites files that actually changed. With
//! `diff_only = true` nothing is written; unified diffs are returned per
//! changed file instead.

use std::fs;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::{GenError, TemplateEngine};

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

/// Strips surrounding double/single quotes and inline comments from a
/// TOML value token.
fn unquote(value: &str) -> String {
    let value = value.split(" #").next().unwrap_or(value).trim();
    let bytes = value.as_bytes();
    if bytes.len() >= 2
        && (bytes[0] == b'"' && bytes[bytes.len() - 1] == b'"'
            || bytes[0] == b'\'' && bytes[bytes.len() - 1] == b'\'')
    {
        value[1..value.len() - 1].replace("\\\"", "\"")
    } else {
        value.to_owned()
    }
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
}

/// Renders every output in `manifest` under `out_root`.
///
/// Target paths are template-evaluated against `ctx`. When the target
/// already exists, user-code regions delimited by [`BEGIN_MARK`] /
/// [`END_MARK`] markers are carried over into the new content before the
/// sha256 comparison decides between rewriting and skipping. With
/// `diff_only = true` no file is ever written; instead each changed file's
/// outcome carries a unified diff in [`RenderOutcome::diff`].
///
/// # Errors
/// When template rendering fails or writing (non-diff mode) fails.
pub fn render_manifest(
    engine: &dyn TemplateEngine,
    manifest: &Manifest,
    ctx: &serde_json::Value,
    out_root: &Path,
    diff_only: bool,
) -> Result<Vec<RenderOutcome>, GenError> {
    let mut outcomes = Vec::with_capacity(manifest.outputs.len());
    for rule in &manifest.outputs {
        let rel_target = render_inline(&rule.target, ctx)?;
        let path = out_root.join(rel_target);
        let mut new_content = engine.render(&rule.template, ctx)?;

        let existing = fs::read_to_string(&path).ok();
        let mut preserved = false;
        if let Some(old) = &existing {
            let (spliced, count) = splice_preserved_regions(old, &new_content);
            if count > 0 {
                new_content = spliced;
                preserved = true;
            }
        }

        let unchanged = existing
            .as_deref()
            .is_some_and(|old| content_hash(old) == content_hash(&new_content));
        let reason = if existing.is_none() {
            WriteReason::Created
        } else if unchanged {
            WriteReason::Unchanged
        } else if preserved {
            WriteReason::PreservedRegionsApplied
        } else {
            WriteReason::Changed
        };

        if diff_only {
            let diff = if unchanged {
                None
            } else {
                Some(unified_diff(
                    existing.as_deref().unwrap_or(""),
                    &new_content,
                ))
            };
            outcomes.push(RenderOutcome {
                path,
                wrote: false,
                reason,
                diff,
            });
        } else if unchanged {
            outcomes.push(RenderOutcome {
                path,
                wrote: false,
                reason,
                diff: None,
            });
        } else {
            write_new(&path, &new_content)?;
            outcomes.push(RenderOutcome {
                path,
                wrote: true,
                reason,
                diff: None,
            });
        }
    }
    Ok(outcomes)
}

/// Writes `content`, creating parent directories as needed.
fn write_new(path: &Path, content: &str) -> Result<(), GenError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, content)?;
    Ok(())
}

/// Evaluates `{{ ... }}` expressions in a target path against `ctx`.
fn render_inline(template: &str, ctx: &serde_json::Value) -> Result<String, GenError> {
    let mut env = minijinja::Environment::new();
    env.add_template_owned("__target__", template)?;
    Ok(env.get_template("__target__")?.render(ctx)?)
}

/// Locates `(begin_idx, end_idx)` line pairs whose begin line contains
/// [`BEGIN_MARK`] and whose matching later line contains [`END_MARK`].
#[must_use]
fn marker_pairs(lines: &[&str]) -> Vec<(usize, usize)> {
    let mut pairs = Vec::new();
    let mut open: Option<usize> = None;
    for (i, line) in lines.iter().enumerate() {
        if line.contains(BEGIN_MARK) {
            open = Some(i);
        } else if line.contains(END_MARK) && open.is_some() {
            pairs.push((open.take().unwrap(), i));
        }
    }
    pairs
}

/// pair; marker lines themselves come from the fresh template. Pairs are
/// matched positionally. Returns the spliced content and how many regions
/// were applied.
#[must_use]
pub(crate) fn splice_preserved_regions(old: &str, new_content: &str) -> (String, usize) {
    let old_lines: Vec<&str> = old.lines().collect();
    let new_lines: Vec<&str> = new_content.lines().collect();
    let old_pairs = marker_pairs(&old_lines);
    let new_pairs = marker_pairs(&new_lines);
    let count = old_pairs.len().min(new_pairs.len());
    if count == 0 {
        return (new_content.to_owned(), 0);
    }

    let mut out = String::new();
    let mut cursor = 0usize;
    for i in 0..count {
        let (ob, oe) = old_pairs[i];
        let (nb, ne) = new_pairs[i];
        for line in &new_lines[cursor..nb] {
            out.push_str(line);
            out.push('\n');
        }
        out.push_str(new_lines[nb]);
        out.push('\n');
        // splice the preserved user code
        for line in &old_lines[ob + 1..oe] {
            out.push_str(line);
            out.push('\n');
        }
        out.push_str(new_lines[ne]);
        out.push('\n');
        cursor = ne + 1;
    }
    for line in &new_lines[cursor.min(new_lines.len())..] {
        out.push_str(line);
        out.push('\n');
    }
    (out, count)
}

/// Computes the sha256 digest of `content`.
#[must_use]
fn content_hash(content: &str) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    hasher.finalize().into()
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
            Some((_, end)) if k <= *end + 2 * CONTEXT => *end = k,
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
        let first_old = positions[start].old_line.map_or(0, |l| l + 1);
        let first_new = positions[start].new_line.map_or(0, |l| l + 1);
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
