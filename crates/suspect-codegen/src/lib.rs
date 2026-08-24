//! Semantic Type Graph compiler: OpenAPI to idiomatic TypeScript, Rust, and
//! Go.
//!
//! Layer 1 lifts schemas into a semantic type graph (allOf composition,
//! oneOf+discriminator sums, string enums, constraint refinements); layer 2
//! plans per-language representations; layer 3 emits idiomatic source with
//! deterministic ordering and built-in drift checking.

pub mod go_emitter;
pub mod idents;
pub mod lift;
pub mod rust_emitter;
pub mod stg;
pub mod ts;

pub use stg::{
    Base, Graph, Ident, OpModel, OpParam, Refinements, StgField, StgNode, StgPrim, StgStringEnum,
    StgStruct, StgSum, StgType, StgUnion, WellKnownFormat,
};

use std::path::Path;

use suspect_ir::IrSpec;

/// Builds the semantic graph for one spec.
#[must_use]
pub fn build_graph(spec: &IrSpec) -> Graph {
    lift::lift(spec)
}

/// One generated file.
#[derive(Debug, Clone, PartialEq)]
pub struct OutFile {
    /// Relative path within the output directory.
    pub path: String,
    /// Full file content.
    pub content: String,
}

/// Per-target emission options.
#[derive(Debug, Clone, Default)]
pub struct EmitOptions {
    /// TypeScript only: emit a Zod schema twin.
    pub zod: bool,
}

/// Emits every target for `graph`.
///
/// # Errors
/// Returns an error message when a target backend fails.
pub fn emit_all(
    graph: &Graph,
    targets: &[&str],
    opts: &EmitOptions,
) -> Result<Vec<OutFile>, String> {
    let mut out = Vec::new();
    for target in targets {
        match *target {
            "ts" => {
                for (path, content) in ts::emit_ts(graph, &crate::ts::TsOptions { zod: opts.zod }) {
                    out.push(OutFile {
                        path: format!("ts/{path}"),
                        content,
                    });
                }
            }
            "rust" => {
                for (path, content) in rust_emitter::emit_rust(graph) {
                    out.push(OutFile {
                        path: format!("rust/src/{path}"),
                        content,
                    });
                }
                out.push(OutFile {
                    path: "rust/Cargo.toml".into(),
                    content: RUST_CARGO.toml().into(),
                });
            }
            "go" => {
                for (path, content) in go_emitter::emit_go(graph) {
                    out.push(OutFile {
                        path: format!("go/{path}"),
                        content,
                    });
                }
            }
            other => return Err(format!("unknown target `{other}`")),
        }
    }
    Ok(out)
}

struct RustCargo;
impl RustCargo {
    fn toml(&self) -> &'static str {
        r#"[package]
name = "generated-api"
version = "0.1.0"
edition = "2021"

[dependencies]
serde = { version = "1", features = ["derive"] }
serde_json = "1"
"#
    }
}
const RUST_CARGO: RustCargo = RustCargo;

/// Content-hash drift check: `true` when all files already match.
#[must_use]
pub fn matches_disk(files: &[OutFile], root: &Path) -> bool {
    files
        .iter()
        .all(|f| std::fs::read_to_string(root.join(&f.path)).is_ok_and(|disk| disk == f.content))
}

/// Writes files under `root`, creating directories as needed.
///
/// # Errors
/// Propagates filesystem failures.
pub fn write_files(files: &[OutFile], root: &Path) -> Result<(), String> {
    use std::io::Write;
    for f in files {
        let full = root.join(&f.path);
        if let Some(parent) = full.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let mut fh = std::fs::File::create(&full).map_err(|e| e.to_string())?;
        fh.write_all(f.content.as_bytes())
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}
