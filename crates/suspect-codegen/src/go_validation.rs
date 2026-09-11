//! Native exact execution of checked portable validation programs.
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
pub fn emit(program: &OwnedProgram) -> Result<Vec<OutFile>, ValidationEmissionError> {
    program.check().map_err(|error| ValidationEmissionError {
        source: error.source,
        message: error.message,
    })?;
    if !((program.version == OwnedProgram::V1_VERSION
        && program.profile == OwnedProgram::V1_PROFILE)
        || (program.version == OwnedProgram::V2_VERSION
            && program.profile == OwnedProgram::V2_PROFILE)
        || (program.version == OwnedProgram::V3_VERSION
            && program.profile == OwnedProgram::V3_PROFILE))
    {
        return Err(ValidationEmissionError {
            source: program.roots.first().map(|root|root.source.clone()),
            message: "Go supports only the checked v1, scoped-applicators v2 and resource/dynamic v3 validation profiles".into(),
        });
    }
    if program.limits.max_depth > 512
        || program.limits.max_number_bytes > 65536
        || program.limits.max_evaluation_steps > i32::MAX as usize
        || program.limits.max_equality_steps > i32::MAX as usize
        || program.limits.max_errors > i32::MAX as usize
    {
        return Err(ValidationEmissionError {
            source: program.roots.first().map(|r| r.source.clone()),
            message: "Go program resource metadata exceeds the native profile".into(),
        });
    }
    Ok(vec![
        OutFile {
            path: "go/validation.go".into(),
            content: include_str!("go_validation/runtime.go").into(),
        },
        OutFile {
            path: "go/validation_number.go".into(),
            content: include_str!("go_validation/number.go").into(),
        },
        OutFile {
            path: "go/validation_pattern.go".into(),
            content: include_str!("go_validation/pattern.go").into(),
        },
        OutFile {
            path: "go/validation_scoped.go".into(),
            content: include_str!("go_validation/scoped.go").into(),
        },
        OutFile {
            path: "go/validation_scoped_pattern.go".into(),
            content: include_str!("go_validation/scoped_pattern.go").into(),
        },
        OutFile {
            path: "go/validation_resources.go".into(),
            content: include_str!("go_validation/resources.go").into(),
        },
        OutFile {
            path: "go/validation_program.json".into(),
            content: serde_json::to_string(program).unwrap(),
        },
    ])
}
