//! Exact Python execution of checked portable validation instructions.
use crate::OutFile;
use suspect_schema::{OwnedProgram, ProgramSource};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationEmissionError {
    pub source: Option<ProgramSource>,
    pub message: String,
}
impl std::fmt::Display for ValidationEmissionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}
impl std::error::Error for ValidationEmissionError {}
/// Emit the checked program and its exact, iterative evaluator.
pub fn emit(program: &OwnedProgram) -> Result<Vec<OutFile>, ValidationEmissionError> {
    if !matches!(
        (program.version, program.profile),
        (OwnedProgram::V1_VERSION, OwnedProgram::V1_PROFILE)
            | (OwnedProgram::V2_VERSION, OwnedProgram::V2_PROFILE)
            | (OwnedProgram::V3_VERSION, OwnedProgram::V3_PROFILE)
    ) {
        return Err(ValidationEmissionError {
            source: None,
            message: "Python validation requires an exact supported v1/v2/v3 version/profile"
                .into(),
        });
    }
    program.check().map_err(|error| ValidationEmissionError {
        source: error.source,
        message: error.message,
    })?;
    if program.limits.max_depth > 512 || program.limits.max_number_bytes > 65_536 {
        return Err(ValidationEmissionError {
            source: program.roots.first().map(|root| root.source.clone()),
            message: "Python validation requires depth <=512 and numeric operands <=65536 bytes"
                .into(),
        });
    }
    let resources = program.version == OwnedProgram::V3_VERSION;
    let scoped = program.version != OwnedProgram::V1_VERSION;
    let mut files = vec![
        OutFile {
            path: "python/validation.py".into(),
            content: if resources {
                include_str!("python_validation/runtime_v3.py")
            } else if scoped {
                include_str!("python_validation/runtime_v2.py")
            } else {
                include_str!("python_validation/runtime.py")
            }
            .into(),
        },
        OutFile {
            path: "python/validation_number.py".into(),
            content: include_str!("python_validation/number.py").into(),
        },
        OutFile {
            path: "python/validation_program.json".into(),
            content: serde_json::to_string(program).expect("checked program JSON"),
        },
    ];
    if scoped {
        files.extend([
            OutFile {
                path: "python/validation_v1.py".into(),
                content: include_str!("python_validation/runtime.py").into(),
            },
            OutFile {
                path: "python/validation_guard.py".into(),
                content: include_str!("python_validation/guard.py").into(),
            },
        ]);
    }
    if resources {
        files.extend([
            OutFile {
                path: "python/validation_v2.py".into(),
                content: include_str!("python_validation/runtime_v2.py").into(),
            },
            OutFile {
                path: "python/validation_resource_guard.py".into(),
                content: include_str!("python_validation/resource_guard.py").into(),
            },
        ]);
    }
    Ok(files)
}
