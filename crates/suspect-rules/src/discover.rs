//! Rule discovery, config, worker-file staging, and scaffolding.

use std::fs;
use std::path::{Path, PathBuf};

use crate::{DEFAULT_RULES_DIR, Error, Result};

/// Runtime configuration (`.suspect/config.yaml` or CLI flags).
#[derive(Debug, Clone, Default)]
pub struct RulesConfig {
    /// Explicit rule files; when empty, `dir` (default `.suspect/rules`)
    /// is scanned for `*.ts`/`*.js`.
    pub rule_files: Vec<PathBuf>,
    /// Directory scanned when `rule_files` is empty.
    pub dir: Option<PathBuf>,
    /// Per-evaluate deadline in milliseconds.
    pub timeout_ms: Option<u64>,
    /// `auto` (default: use bun if found) | `require` | `off`.
    pub bun: Option<String>,
}

/// Locates rule files per the config: explicit files win, else the rules
/// dir is scanned (sorted, deterministic).
///
/// # Errors
/// [`Error::RuleLoad`] when an explicitly named file is missing.
pub fn discover_rule_files(workspace_root: &Path, config: &RulesConfig) -> Result<Vec<PathBuf>> {
    if !config.rule_files.is_empty() {
        for f in &config.rule_files {
            if !f.is_file() {
                return Err(Error::RuleLoad(format!(
                    "rule file missing: {}",
                    f.display()
                )));
            }
        }
        return Ok(config.rule_files.clone());
    }
    let dir = config
        .dir
        .clone()
        .unwrap_or_else(|| workspace_root.join(DEFAULT_RULES_DIR));
    let mut out = Vec::new();
    let Ok(entries) = fs::read_dir(&dir) else {
        return Ok(out);
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let ext = path.extension().and_then(|e| e.to_str());
        if matches!(ext, Some("ts") | Some("js") | Some("mts") | Some("mjs")) {
            out.push(path);
        }
    }
    out.sort();
    Ok(out)
}

/// Worker runtime files embedded at build time; staged into a
/// content-hashed cache dir so user rules can resolve
/// `@suspect/rules-sdk` via `NODE_PATH` and the worker imports its
/// siblings relatively.
pub const WORKER_FILES: [(&str, &str); 5] = [
    (
        "worker.ts",
        include_str!("../../../rules-runtime/src/worker.ts"),
    ),
    ("sdk.ts", include_str!("../../../rules-runtime/src/sdk.ts")),
    (
        "nodes.ts",
        include_str!("../../../rules-runtime/src/nodes.ts"),
    ),
    (
        "functions.ts",
        include_str!("../../../rules-runtime/src/functions.ts"),
    ),
    (
        "protocol.ts",
        include_str!("../../../rules-runtime/src/protocol.ts"),
    ),
];

/// Stages worker files into `<cache_root>/<hash>/` and returns the entry
/// path. Skipped when the hash dir already exists (content-addressed).
///
/// # Errors
/// [`Error::Io`] on filesystem failures.
pub fn stage_worker_files(cache_root: &Path) -> Result<PathBuf> {
    let mut hasher_input = String::new();
    for (name, content) in WORKER_FILES {
        hasher_input.push_str(name);
        hasher_input.push('\0');
        hasher_input.push_str(content);
        hasher_input.push('\0');
    }
    let hash = blake3_lite(&hasher_input);
    let dir = cache_root.join(&hash);
    if dir.join("worker.ts").is_file() {
        return Ok(dir.join("worker.ts"));
    }
    fs::create_dir_all(&dir)?;
    for (name, content) in WORKER_FILES {
        fs::write(dir.join(name), content)?;
    }
    // node_modules shim so user rules resolve `@suspect/rules-sdk` through
    // NODE_PATH without any plugin API.
    let shim = dir.join("node_modules/@suspect/rules-sdk");
    fs::create_dir_all(&shim)?;
    fs::write(
        shim.join("package.json"),
        r#"{"name":"@suspect/rules-sdk","version":"0.1.0","main":"index.ts"}"#,
    )?;
    fs::write(shim.join("index.ts"), "export * from \"../../sdk.ts\";\n")?;
    Ok(dir.join("worker.ts"))
}

/// FNV-1a 64 — deterministic content hash for cache dir naming (not a
/// security boundary).
#[must_use]
pub fn blake3_lite(input: &str) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in input.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("w{hash:016x}")
}

/// Locates `bun` on `PATH` and returns its path.
#[must_use]
pub fn find_bun() -> Option<PathBuf> {
    let path_var = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path_var) {
        let candidate = dir.join(if cfg!(windows) { "bun.exe" } else { "bun" });
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

const SCAFFOLD_TEMPLATE: &str = r#"/**
 * <NAME> — scaffolded by `suspect rules new`.
 */
import { defineRule, r } from "@suspect/rules-sdk";

export default defineRule({
  meta: {
    id: "<NAME>",
    description: "Describe what this rule enforces",
  },
  given: r.operation,
  check(op, ctx) {
    if (!op.summary) {
      ctx.report({
        message: `${op.method.toUpperCase()} ${op.path} is missing a summary`,
        at: op,
      });
    }
  },
});
"#;

/// Writes a scaffolded rule file; refuses to overwrite.
///
/// # Errors
/// [`Error::Io`] on filesystem failures; [`Error::RuleLoad`] when the
/// target already exists.
pub fn scaffold_rule(dir: &Path, name: &str) -> Result<PathBuf> {
    let path = dir.join(format!("{name}.ts"));
    if path.exists() {
        return Err(Error::RuleLoad(format!(
            "refusing to overwrite {}",
            path.display()
        )));
    }
    fs::create_dir_all(dir)?;
    fs::write(&path, SCAFFOLD_TEMPLATE.replace("<NAME>", name))?;
    Ok(path)
}
