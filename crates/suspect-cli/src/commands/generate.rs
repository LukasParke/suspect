//! `suspect gen` subcommand — render documentation/SDK presets or custom manifests.

use std::path::Path;

use suspect_gen::{FilterRegistry, MinijinjaEngine, TemplateEngine};
use suspect_ir::IrSpec;
use suspect_source::Uri;

fn ir_for(spec: &Path) -> anyhow::Result<IrSpec> {
    let ws = super::workspace_dir_all(spec)?;
    let uri = Uri::from_path(spec)?;
    IrSpec::from_workspace(&ws, &uri).map_err(anyhow::Error::msg)
}

/// Runs `suspect gen`.
///
/// # Errors
/// Propagates workspace/IR/template rendering failures.
pub fn generate(
    spec: &Path,
    preset: Option<&str>,
    manifest: Option<&Path>,
    out: &Path,
    diff: bool,
) -> anyhow::Result<i32> {
    let ir = ir_for(spec)?;

    // Engine with all built-in filters registered.
    let mut engine = MinijinjaEngine::new();
    FilterRegistry::register(&mut engine);

    let (manifest_obj, ctx) = match (preset, manifest) {
        (Some(name), _) => {
            let Some(p) = suspect_gen::presets::get(name) else {
                anyhow::bail!("unknown preset {name:?}");
            };
            for (tpl_name, tpl_src) in p.templates {
                engine.add_template(tpl_name, tpl_src)?;
            }
            let parsed = suspect_gen::parse_manifest_str(p.manifest_toml)?;
            (parsed, (p.ctx_builder)(&ir))
        }
        (None, Some(path)) => {
            let parsed = suspect_gen::load_manifest(path)?;
            // Custom manifests render against every template they reference;
            // those templates must exist as files next to the manifest.
            let dir = path.parent().unwrap_or(Path::new("."));
            for rule in &parsed.outputs {
                let tpl_path = dir.join(&rule.template);
                let src = std::fs::read_to_string(&tpl_path)
                    .map_err(|e| anyhow::anyhow!("template {}: {e}", tpl_path.display()))?;
                engine.add_template(&rule.template, &src)?;
            }
            (parsed, serde_json::to_value(&ir)?)
        }
        (None, None) => anyhow::bail!("provide --preset <name> or --manifest <gen.toml>"),
    };

    std::fs::create_dir_all(out)?;
    let outcomes = suspect_gen::render_manifest(&engine, &manifest_obj, &ctx, out, diff)?;
    for o in &outcomes {
        let status = if diff {
            "diff"
        } else {
            match o.reason {
                suspect_gen::WriteReason::Created => "created",
                suspect_gen::WriteReason::Changed => "changed",
                suspect_gen::WriteReason::Unchanged => "unchanged",
                suspect_gen::WriteReason::PreservedRegionsApplied => "preserved",
            }
        };
        if let Some(d) = &o.diff {
            println!("--- {}\n{d}", o.path.display());
        } else {
            println!("{status:>9}  {}", o.path.display());
        }
    }
    Ok(0)
}
