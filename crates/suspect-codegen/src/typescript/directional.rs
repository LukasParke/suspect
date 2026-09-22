//! Directional validation projection for request/response codec views.
//!
//! The model plan decides directional requiredness applicability through one
//! shared proof ([`super::directional_required_exception`]). This module
//! applies the same proof to the portable compiled validation program: a
//! `required` instruction loses exactly the member names whose read/write
//! annotation applicability that proof establishes for the view, evaluated at
//! each original object position. Every other instruction — instance types,
//! exact numeric bounds, patterns, cardinality, composition including `oneOf`
//! exclusivity, references and recursion — is retained unchanged, and every
//! provided value still validates against the complete unmodified assertions.
//! This is a requiredness projection, not a data-stripping or rejection
//! policy: no wire property is removed, no default is injected, and a
//! read-only value supplied in a request (or a write-only value in a
//! response) still validates and survives encoding.
//!
//! Explicit OpenAPI 3.1 request/response validation policy: OpenAPI 3.1+/3.2
//! use JSON Schema annotation semantics, under which the owning authority may
//! ignore or reject modifications to read-only values. Request and response
//! views therefore keep every declared property in the model type and relax
//! only the presence requirement — and only when the annotation is proven
//! applicable to that exact required property position by the shared walk.
//! OpenAPI 3.0's directional required needs a separate admitted validation
//! profile. Model and owned-codec admission explicitly refuse those requirements
//! rather than borrowing this modern annotation policy. The 3.0 subset without
//! directional requirements uses the same compiled instruction set.
//! Applicator positions whose annotation applicability cannot be
//! established (`allOf`, `anyOf`, `oneOf`, `if`/`then`/`else`, `not`) remain
//! source-linked model errors, so this projection is never reached for them.

use std::collections::BTreeMap;

use suspect_ir::contract::{Contract, SchemaId};
use suspect_schema::{OwnedProgram, ProgramCheck, ProgramInstruction};

use super::{ModelDiagnostic, ModelView};

/// Emitted module path of the shared validation program seam. The first
/// distinct projected program binds here so the unprojected neutral plan keeps
/// its existing artifact identity.
const BASE_PROGRAM_FILE: &str = "typescript/validation-program.ts";

/// One immutable validation program per requested model view.
pub(super) struct ViewPrograms {
    /// Requested view to emitted program module path. Views whose projected
    /// programs are structurally identical share one module.
    pub bindings: BTreeMap<ModelView, &'static str>,
    /// Distinct programs in canonical binding order: emitted module path plus
    /// the program it contains. The first entry is emitted through the shared
    /// validation seam, which also owns the validator runtimes and
    /// documentation.
    pub programs: Vec<(&'static str, OwnedProgram)>,
}

impl ViewPrograms {
    /// The program emitted through the shared validation seam.
    pub(super) fn base(&self) -> &OwnedProgram {
        &self.programs[0].1
    }
}

/// Projects the compiled validation program for every requested view.
///
/// Neutral binds to the unprojected program. Request and response projections
/// are deduplicated by program equality, so a closure without directional
/// annotations keeps exactly one program artifact no matter how many views are
/// requested. Output is deterministic in the canonical view order, independent
/// of the caller's selection order.
///
/// # Errors
/// Returns a source-linked diagnostic if a required member's annotation
/// applicability cannot be established by the shared proof, or if the
/// projected program fails its portable invariants.
pub(super) fn view_programs(
    contract: &Contract,
    base: OwnedProgram,
    views: &[ModelView],
) -> Result<ViewPrograms, ModelDiagnostic> {
    let mut programs: Vec<(&'static str, OwnedProgram)> = Vec::new();
    let mut bindings = BTreeMap::new();
    for view in [ModelView::Neutral, ModelView::Request, ModelView::Response] {
        if !views.contains(&view) {
            continue;
        }
        let program = match view {
            ModelView::Neutral => base.clone(),
            ModelView::Request | ModelView::Response => project(contract, &base, view)?,
        };
        let fallback_file = if programs.is_empty() {
            BASE_PROGRAM_FILE
        } else {
            match view {
                ModelView::Neutral => unreachable!("the neutral view is projected first"),
                ModelView::Request => "typescript/validation-program-request.ts",
                ModelView::Response => "typescript/validation-program-response.ts",
            }
        };
        let file = match programs
            .iter()
            .position(|(_, existing)| *existing == program)
        {
            Some(position) => programs[position].0,
            None => {
                programs.push((fallback_file, program));
                fallback_file
            }
        };
        bindings.insert(view, file);
    }
    Ok(ViewPrograms { bindings, programs })
}

/// Relaxes `required` instructions at their original object positions with the
/// shared directional proof, leaving every other compiled instruction and
/// source identity untouched.
fn project(
    contract: &Contract,
    base: &OwnedProgram,
    view: ModelView,
) -> Result<OwnedProgram, ModelDiagnostic> {
    let keyword = match view {
        ModelView::Request => "readOnly",
        ModelView::Response => "writeOnly",
        ModelView::Neutral => unreachable!("the neutral view is never projected"),
    };
    let fallback = SchemaId::new(contract.entry().clone(), Default::default());
    let mut program = base.clone();
    for node in &mut program.nodes {
        let object = super::codecs::source_id(contract, &node.source, &fallback);
        let mut kept = Vec::with_capacity(node.checks.len());
        for check in node.checks.drain(..) {
            match check.instruction {
                ProgramInstruction::Required { names } => {
                    let mut retained = Vec::with_capacity(names.len());
                    for name in names {
                        match super::directional_required_exception(contract, &object, &name, view)
                        {
                            // Proven exception: this view no longer requires
                            // the member's presence. All other assertions,
                            // including the property's own schema, remain.
                            Ok(true) => {}
                            Ok(false) => retained.push(name),
                            Err((child, applicator)) => {
                                return Err(super::codecs::finding(
                                    contract,
                                    child,
                                    "directional-annotation-evaluation",
                                    format!(
                                        "`{keyword}` under `{applicator}` requires evaluated annotation applicability before a directional requiredness view can be generated"
                                    ),
                                ));
                            }
                        }
                    }
                    if !retained.is_empty() {
                        kept.push(ProgramCheck {
                            source: check.source,
                            instruction: ProgramInstruction::Required { names: retained },
                        });
                    }
                }
                instruction => kept.push(ProgramCheck {
                    source: check.source,
                    instruction,
                }),
            }
        }
        node.checks = kept;
    }
    program.check().map_err(|error| {
        let source = error.source.as_ref().map_or_else(
            || fallback.clone(),
            |source| super::codecs::source_id(contract, source, &fallback),
        );
        super::codecs::finding(
            contract,
            source,
            "codec-directional-projection",
            error.message,
        )
    })?;
    Ok(program)
}
