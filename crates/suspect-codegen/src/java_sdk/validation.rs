//! Java admission of the complete current portable instruction set. A future
//! opcode must be handled explicitly here before Java can consume it.

use super::{HttpDiagnostic, plan_diag};
use std::sync::Arc;
use suspect_ir::contract::{Contract, SchemaId};
use suspect_schema::{Config, OwnedCompiler, OwnedProgram, ProgramInstruction};

/// Compile roots for the standalone Java validator, including schema forms whose
/// native model representation is deliberately outside the SDK model profile.
///
/// # Errors
/// Located unsupported assertions or runtime resource invariants reject emission.
pub fn plan_validation(
    contract: &Arc<Contract>,
    roots: &[SchemaId],
) -> Result<OwnedProgram, Vec<HttpDiagnostic>> {
    compile_validation(contract, roots, false)
}

/// Compile the explicitly selected, native-verified resource/dynamic V3 profile.
///
/// # Errors
/// Located malformed, unsupported or unbounded source/program declarations.
pub fn plan_validation_v3(
    contract: &Arc<Contract>,
    roots: &[SchemaId],
) -> Result<OwnedProgram, Vec<HttpDiagnostic>> {
    compile_validation(contract, roots, true)
}

fn compile_validation(
    contract: &Arc<Contract>,
    roots: &[SchemaId],
    resources: bool,
) -> Result<OwnedProgram, Vec<HttpDiagnostic>> {
    let compiler = OwnedCompiler::new(Config {
        max_depth: 128,
        ..Config::default()
    });
    let compiled = if resources {
        compiler.compile_v3(contract.clone(), roots)
    } else {
        compiler.compile_v2(contract.clone(), roots)
    }
    .map_err(|errors| {
        errors
            .into_iter()
            .map(|e| HttpDiagnostic {
                source: e.source,
                at: e.span.unwrap_or(0..0),
                code: "java-validation-compile",
                message: e.message,
            })
            .collect::<Vec<_>>()
    })?;
    let program = compiled.program();
    check(contract, &program)?;
    Ok(program)
}

pub(crate) fn check(
    contract: &Contract,
    program: &OwnedProgram,
) -> Result<(), Vec<HttpDiagnostic>> {
    program.check().map_err(|error| {
        vec![plan_diag(
            contract,
            "java-validation-program",
            error.to_string(),
        )]
    })?;
    if !matches!(
        (program.version, program.profile),
        (OwnedProgram::V1_VERSION, OwnedProgram::V1_PROFILE)
            | (OwnedProgram::V2_VERSION, OwnedProgram::V2_PROFILE)
            | (OwnedProgram::V3_VERSION, OwnedProgram::V3_PROFILE)
    ) {
        return Err(vec![plan_diag(
            contract,
            "java-validation-profile-unsupported",
            "Java verifies only the exact v1/v2/v3 executable profiles",
        )]);
    }
    if program.limits.max_depth > 128
        || program.limits.max_number_bytes > 65536
        || program.limits.max_evaluation_steps > 100_000_000
        || program.limits.max_equality_steps > 100_000_000
        || program.nodes.len() > 100_000
    {
        return Err(vec![plan_diag(
            contract,
            "java-validation-resource-limit",
            "portable program exceeds Java's finite stack/operand/graph profile",
        )]);
    }
    let mut numeric_work = 0usize;
    for node in &program.nodes {
        for check in &node.checks {
            #[allow(unreachable_patterns)] // Future instructions remain an explicit refusal.
            match &check.instruction {
                ProgramInstruction::Always { .. }
                | ProgramInstruction::Type { .. }
                | ProgramInstruction::Ref { .. }
                | ProgramInstruction::DynamicRef { .. }
                | ProgramInstruction::Properties { .. }
                | ProgramInstruction::AdditionalProperties { .. }
                | ProgramInstruction::Required { .. }
                | ProgramInstruction::Items { .. }
                | ProgramInstruction::PrefixItems { .. }
                | ProgramInstruction::AllOf { .. }
                | ProgramInstruction::AnyOf { .. }
                | ProgramInstruction::OneOf { .. }
                | ProgramInstruction::Not { .. }
                | ProgramInstruction::Enum { .. }
                | ProgramInstruction::Const { .. }
                | ProgramInstruction::UniqueItems
                | ProgramInstruction::Pattern { .. }
                | ProgramInstruction::If { .. }
                | ProgramInstruction::DependentRequired { .. }
                | ProgramInstruction::DependentSchemas { .. }
                | ProgramInstruction::PatternProperties { .. }
                | ProgramInstruction::AdditionalPropertiesWithPatterns { .. }
                | ProgramInstruction::PropertyNames { .. }
                | ProgramInstruction::UnevaluatedProperties { .. }
                | ProgramInstruction::UnevaluatedItems { .. } => {}
                ProgramInstruction::Contains {
                    minimum, maximum, ..
                } => {
                    for value in minimum.iter().chain(maximum.iter()) {
                        numeric_work = numeric_work.saturating_add(exponent_work(value));
                    }
                }
                ProgramInstruction::Bound { value, .. }
                | ProgramInstruction::MultipleOf { value }
                | ProgramInstruction::Count { value, .. } => {
                    numeric_work = numeric_work.saturating_add(exponent_work(value));
                }
                _ => {
                    let mut source = SchemaId::new(
                        suspect_source::Uri::parse(&check.source.document)
                            .expect("checked program URI"),
                        Default::default(),
                    );
                    for token in check.source.pointer.split('/').skip(1) {
                        source = source.child(&token.replace("~1", "/").replace("~0", "~"));
                    }
                    return Err(vec![super::diagnostic(
                        contract,
                        source,
                        "java-validation-opcode-unsupported",
                        "this opcode needs a separately verified Java executor and is never interpreted as a static reference or no-op",
                    )]);
                }
            }
        }
    }
    let value = serde_json::to_value(program).expect("portable program is JSON");
    let bytes = serde_json::to_vec(&value).expect("portable JSON").len();
    let mut pending = vec![(&value, 0usize)];
    let mut too_deep = false;
    while let Some((value, depth)) = pending.pop() {
        if depth > 128 {
            too_deep = true;
            break;
        }
        match value {
            serde_json::Value::Array(values) => {
                pending.extend(values.iter().map(|v| (v, depth + 1)))
            }
            serde_json::Value::Object(values) => {
                pending.extend(values.values().map(|v| (v, depth + 1)))
            }
            serde_json::Value::Number(number) => {
                numeric_work = numeric_work.saturating_add(exponent_work(&number.to_string()))
            }
            _ => {}
        }
    }
    // Four units per encoded byte conservatively covers the native metadata
    // parser's visits/scans. Symbolic BigInteger exponent parsing has an
    // additional finite cost, with leading zero padding excluded from magnitude.
    if bytes > 8 * 1024 * 1024
        || too_deep
        || bytes.saturating_mul(4).saturating_add(numeric_work) > 32 * 1024 * 1024
    {
        return Err(vec![plan_diag(
            contract,
            "java-validation-resource-limit",
            "portable program exceeds Java's metadata byte/depth/work loading policy",
        )]);
    }
    Ok(())
}

fn exponent_work(token: &str) -> usize {
    let significant = token.split_once(['e', 'E']).map_or(0, |(_, exponent)| {
        exponent.trim_start_matches(['+', '-', '0']).len()
    });
    significant.saturating_mul(significant) / 16
}
