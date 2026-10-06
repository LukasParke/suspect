//! `suspect bridge`: the live contract bridge, driven from the CLI.
//!
//! The bridge (`suspect-gateway::bridge::ContractBridge`) runs two loops:
//! spec changes become regeneration plans (mocks, SDK targets, docs,
//! validators), and observed traffic reconciled against the spec becomes
//! evolution proposals and conflicts. This command drives the propagation
//! loop — the one a watcher needs — until interrupted.

use std::path::{Path, PathBuf};
use std::time::Duration;

use suspect_gateway::bridge::ContractBridge;

/// Runs `suspect bridge` until interrupted.
///
/// # Errors
/// Workspace loading failures on the first tick.
pub fn bridge(
    spec: &Path,
    interval_ms: u64,
    max_ticks: Option<u64>,
    json: bool,
) -> anyhow::Result<i32> {
    let ws = super::workspace_for_entry(spec)?;
    let uri = suspect_source::Uri::from_path(spec)?;
    ws.get(&uri)
        .ok_or_else(|| anyhow::anyhow!("spec document not loaded: {uri}"))?;
    let mut state = ContractBridge::new(spec);
    let load_ir = |path: &Path| -> Option<suspect_ir::IrSpec> {
        suspect_ir::IrSpec::from_workspace(&ws, &suspect_source::Uri::from_path(path).ok()?).ok()
    };
    let _ = PathBuf::new();
    let mut tick_number: u64 = 0;
    loop {
        let result = state.tick(&load_ir);
        tick_number += 1;
        if json {
            println!(
                "{}",
                serde_json::json!({
                    "tick": tick_number,
                    "reloaded": result.reloaded,
                    "new_conflicts": result.new_conflicts,
                    "regen": result.regen,
                    "proposals": result.proposals,
                })
            );
        } else if result.reloaded
            && let Some(regen) = &result.regen
        {
            println!(
                "spec changed: regenerate {}{}{}{} — {}",
                if regen.mocks { "mocks " } else { "" },
                if regen.docs { "docs " } else { "" },
                if regen.validators { "validators " } else { "" },
                regen.sdk_targets.join(", "),
                regen.reason
            );
        }
        for proposal in &result.proposals {
            println!("proposal: {proposal:?}");
        }
        if result.new_conflicts > 0 {
            let conflicts = result.new_conflicts;
            println!("conflicts: {conflicts} (spec vs observed traffic)");
        }
        if let Some(max) = max_ticks
            && tick_number >= max
        {
            let (reloads, proposals, conflicts) = state.stats();
            if !json {
                println!(
                    "bridge: {reloads} reload(s), {proposals} proposal(s), {conflicts} conflict(s)"
                );
            }
            return Ok(0);
        }
        std::thread::sleep(Duration::from_millis(interval_ms));
    }
}
