//! Portable-program admission for the versioned native v1/v2/v3 runtime profiles.
use super::{HttpDiagnostic, diagnostic};
use suspect_ir::contract::{Contract, SourceId};
use suspect_schema::{OwnedProgram, ProgramInstruction};

pub(super) fn check(
    contract: &Contract,
    program: &OwnedProgram,
) -> Result<(), Vec<HttpDiagnostic>> {
    let fallback = SourceId::new(contract.entry().clone(), Default::default());
    program.check().map_err(|error| {
        vec![diagnostic(
            contract,
            locate(contract, error.source.as_ref()).unwrap_or_else(|| fallback.clone()),
            "csharp-validation-program",
            error.message,
        )]
    })?;
    if program.limits.max_depth > 128
        || program.limits.max_number_bytes > 4096
        || program.nodes.len() > 65_536
        || program.limits.max_evaluation_steps > 1_000_000
        || program.limits.max_equality_steps > 1_000_000
        || program.limits.max_errors > i32::MAX as usize
    {
        return Err(vec![diagnostic(
            contract,
            fallback,
            "csharp-validation-resource-policy",
            "C# requires schema depth <=128, numeric operands <=4096 bytes, <=65536 program nodes and <=1000000 evaluation/equality steps",
        )]);
    }
    // Exhaustive matching makes a new portable opcode an integration error,
    // rather than allowing an unrecognized assertion to become a successful no-op.
    for node in &program.nodes {
        for check in &node.checks {
            match &check.instruction {
                ProgramInstruction::Always { .. }
                | ProgramInstruction::Type { .. }
                | ProgramInstruction::Ref { .. }
                | ProgramInstruction::Properties { .. }
                | ProgramInstruction::AdditionalProperties { .. }
                | ProgramInstruction::Required { .. }
                | ProgramInstruction::Items { .. }
                | ProgramInstruction::PrefixItems { .. }
                | ProgramInstruction::AllOf { .. }
                | ProgramInstruction::AnyOf { .. }
                | ProgramInstruction::OneOf { .. }
                | ProgramInstruction::Not { .. }
                | ProgramInstruction::Bound { .. }
                | ProgramInstruction::MultipleOf { .. }
                | ProgramInstruction::Count { .. }
                | ProgramInstruction::Enum { .. }
                | ProgramInstruction::Const { .. }
                | ProgramInstruction::UniqueItems
                | ProgramInstruction::Pattern { .. }
                | ProgramInstruction::If { .. }
                | ProgramInstruction::DependentRequired { .. }
                | ProgramInstruction::DependentSchemas { .. }
                | ProgramInstruction::Contains { .. }
                | ProgramInstruction::PatternProperties { .. }
                | ProgramInstruction::AdditionalPropertiesWithPatterns { .. }
                | ProgramInstruction::PropertyNames { .. }
                | ProgramInstruction::UnevaluatedProperties { .. }
                | ProgramInstruction::UnevaluatedItems { .. }
                | ProgramInstruction::DynamicRef { .. } => {}
            }
        }
    }
    if !matches!(
        (program.version, program.profile),
        (OwnedProgram::V1_VERSION, OwnedProgram::V1_PROFILE)
            | (OwnedProgram::V2_VERSION, OwnedProgram::V2_PROFILE)
            | (OwnedProgram::V3_VERSION, OwnedProgram::V3_PROFILE)
    ) {
        return Err(vec![diagnostic(
            contract,
            fallback,
            "csharp-validation-version-unsupported",
            "this C# runtime requires an exact v1, scoped v2 or resource/dynamic v3 validation format/profile pair",
        )]);
    }
    Ok(())
}
fn locate(contract: &Contract, source: Option<&suspect_schema::ProgramSource>) -> Option<SourceId> {
    let source = source?;
    let (uri, _) = contract
        .documents()
        .find(|(uri, _)| uri.to_string() == source.document)?;
    Some(source.pointer.strip_prefix('/').map_or_else(
        || SourceId::new(uri.clone(), Default::default()),
        |p| {
            p.split('/')
                .fold(SourceId::new(uri.clone(), Default::default()), |id, t| {
                    id.child(&t.replace("~1", "/").replace("~0", "~"))
                })
        },
    ))
}
