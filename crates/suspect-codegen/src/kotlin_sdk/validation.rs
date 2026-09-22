//! Checked portable validation emission, independent of native model admission.

use std::sync::Arc;
use suspect_ir::contract::{Contract, SchemaId, SourceId};
use suspect_schema::{Config, OwnedCompiler, OwnedProgram, ProgramInstruction, ProgramSource};

use super::{HttpDiagnostic, diagnostic};
use crate::OutFile;

/// Compile the canonical schema closure for the Kotlin portable executor.
///
/// This lower-level validation profile includes every checked instruction,
/// including patterns and tuples. Native model admission is a separate gate.
pub fn plan_validation(
    contract: Arc<Contract>,
    roots: &[SchemaId],
    config: Config,
) -> Result<OwnedProgram, Vec<HttpDiagnostic>> {
    compile(contract, roots, config, 1)
}

/// Explicit scoped-applicator compilation. Base closures still produce v1 bytes.
pub fn plan_validation_v2(
    contract: Arc<Contract>,
    roots: &[SchemaId],
    config: Config,
) -> Result<OwnedProgram, Vec<HttpDiagnostic>> {
    compile(contract, roots, config, 2)
}

/// Explicit indexed-resource and dynamic-reference compilation; all sources and
/// bindings come from Contract and no acquisition occurs during validation.
pub fn plan_validation_v3(
    contract: Arc<Contract>,
    roots: &[SchemaId],
    config: Config,
) -> Result<OwnedProgram, Vec<HttpDiagnostic>> {
    compile(contract, roots, config, 3)
}

fn compile(
    contract: Arc<Contract>,
    roots: &[SchemaId],
    config: Config,
    version: u8,
) -> Result<OwnedProgram, Vec<HttpDiagnostic>> {
    let compiler = OwnedCompiler::new(config);
    let owned = if version == 3 {
        compiler.compile_v3(contract.clone(), roots)
    } else if version == 2 {
        compiler.compile_v2(contract.clone(), roots)
    } else {
        compiler.compile(contract.clone(), roots)
    }
    .map_err(|errors| {
        errors
            .into_iter()
            .map(|error| HttpDiagnostic {
                source: error.source,
                at: error.span.unwrap_or(0..0),
                code: "kotlin-schema-unsupported",
                message: error.message,
            })
            .collect::<Vec<_>>()
    })?;
    let program = owned.program();
    admit(&contract, &program)?;
    Ok(program)
}

pub(super) fn admit(
    contract: &Contract,
    program: &OwnedProgram,
) -> Result<(), Vec<HttpDiagnostic>> {
    let error = |at: Option<&ProgramSource>, code, message| {
        let source = at
            .and_then(|at| {
                suspect_source::Uri::parse(&at.document).ok().map(|uri| {
                    at.pointer
                        .split('/')
                        .skip(1)
                        .fold(SourceId::new(uri, Default::default()), |source, part| {
                            source.child(&part.replace("~1", "/").replace("~0", "~"))
                        })
                })
            })
            .unwrap_or_else(|| SourceId::new(contract.entry().clone(), Default::default()));
        vec![diagnostic(contract, source, code, message)]
    };
    if let Err(issue) = program.check() {
        return Err(error(
            issue.source.as_ref(),
            "kotlin-program-invalid",
            issue.message,
        ));
    }
    if let Err(message) = check_limits(program) {
        return Err(error(
            program.roots.first().map(|r| &r.source),
            "kotlin-program-limit",
            message,
        ));
    }
    Ok(())
}

fn check_limits(program: &OwnedProgram) -> Result<(), String> {
    if !matches!(
        (program.version, program.profile),
        (OwnedProgram::V1_VERSION, OwnedProgram::V1_PROFILE)
            | (OwnedProgram::V2_VERSION, OwnedProgram::V2_PROFILE)
            | (OwnedProgram::V3_VERSION, OwnedProgram::V3_PROFILE)
    ) {
        return Err(
            "Kotlin admits only the exact checked v1/v2/v3 validation version/profile pairs".into(),
        );
    }
    let limits = &program.limits;
    if program.nodes.len() > 16_384
        || limits.max_depth > 128
        || limits.max_number_bytes > 4096
        || [
            limits.max_errors,
            limits.max_equality_steps,
            limits.max_evaluation_steps,
        ]
        .iter()
        .any(|n| *n > 100_000)
    {
        return Err("Kotlin validation requires at most 16384 nodes, depth 128, 4096 numeric bytes and 100000 visits/findings per budget".into());
    }
    // An exhaustive match keeps newly added shared opcodes behind a compiler
    // change, instead of letting an unknown instruction silently succeed.
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
    let value = serde_json::to_value(program).expect("portable program");
    let mut pending = vec![(&value, 0)];
    let mut count = 0;
    while let Some((value, depth)) = pending.pop() {
        count += 1;
        if count > 1_000_000 || depth >= 128 {
            return Err("compiled metadata exceeds the Kotlin JSON depth/value budget".into());
        }
        match value {
            serde_json::Value::Number(number) if number.to_string().len() > 4096 => {
                return Err("compiled metadata has a numeric literal exceeding 4096 bytes".into());
            }
            serde_json::Value::Array(values) => {
                pending.extend(values.iter().map(|v| (v, depth + 1)))
            }
            serde_json::Value::Object(values) => {
                pending.extend(values.values().map(|v| (v, depth + 1)))
            }
            _ => {}
        }
    }
    if serde_json::to_vec(program).expect("portable program").len() > 16 * 1024 * 1024 {
        return Err("compiled metadata exceeds the Kotlin 16 MiB program budget".into());
    }
    Ok(())
}

/// Emit the exact-JSON and portable-validation module assets under `kotlin/`.
///
/// Accepts only a structurally checked program and a legal Kotlin package. This
/// is also the standalone conformance seam; it does not manufacture native
/// models for schema shapes the SDK planner cannot faithfully represent.
pub fn emit_validation(program: &OwnedProgram, package: &str) -> Result<Vec<OutFile>, String> {
    program.check().map_err(|error| error.to_string())?;
    check_limits(program)?;
    if !super::valid_package(package) {
        return Err("invalid Kotlin package".into());
    }
    let path = package.replace('.', "/");
    let mut files = Vec::new();
    let runtime = runtime(program);
    for (name, template) in [
        ("Json.kt", include_str!("Json.kt")),
        ("Validation.kt", runtime.as_ref()),
    ] {
        files.push(OutFile {
            path: format!("kotlin/src/main/kotlin/{path}/{name}"),
            content: template.replace("__PACKAGE__", package),
        });
    }
    files.push(OutFile {
        path: format!("kotlin/src/main/resources/{path}/validation.json"),
        content: format!(
            "{}\n",
            serde_json::to_string(program).expect("portable program")
        ),
    });
    Ok(files)
}

pub(super) fn runtime(program: &OwnedProgram) -> std::borrow::Cow<'static, str> {
    let v1 = include_str!("Validation.kt");
    if program.version == OwnedProgram::V1_VERSION {
        return std::borrow::Cow::Borrowed(v1);
    }
    let (api, _) = v1
        .split_once("internal object ValidationProgram {")
        .expect("frozen v1 public API boundary");
    std::borrow::Cow::Owned(format!(
        "{api}{}",
        if program.version == OwnedProgram::V3_VERSION {
            super::validation_v3::runtime()
        } else {
            include_str!("ValidationV2.kt").to_owned()
        }
    ))
}
