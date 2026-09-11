//! Shared scoped discovery with the C# compiler's actual native resource obligations.
use super::{HttpDiagnostic, diagnostic};
use crate::{examples, http_protocol::ProtocolPlan};
use std::sync::Arc;
use suspect_ir::contract::Contract;
use suspect_schema::{OwnedOutcome, OwnedSchema};

pub(super) fn plan(
    contract: Arc<Contract>,
    protocol: &ProtocolPlan,
    compiled: &OwnedSchema,
) -> Result<examples::ExamplePlan, Vec<HttpDiagnostic>> {
    let examples = if compiled.program().version == suspect_schema::OwnedProgram::V3_VERSION {
        examples::plan_protocol_examples_v3(contract.clone(), protocol, Default::default())
    } else {
        examples::plan_protocol_examples_v2(contract.clone(), protocol, Default::default())
    };
    let mut errors = Vec::new();
    // Discovery has its own bounded candidate policy. An emitted C# recipe also
    // owes the exact selected SDK closure's depth/work/numeric limits; neither
    // a wider discovery profile nor a logical trial may waive that obligation.
    for operation in examples.operations() {
        for entry in &operation.entries {
            match compiled.validate(&entry.schema, &entry.value) {
                OwnedOutcome::Valid => {}
                OwnedOutcome::Invalid(findings) => {
                    for finding in findings {
                        errors.push(diagnostic(
                            &contract,
                            finding.source,
                            "csharp-native-example-invalid",
                            finding.message,
                        ));
                    }
                }
                OwnedOutcome::EvaluationFailure(finding) => errors.push(diagnostic(
                    &contract,
                    finding.source,
                    "csharp-native-example-evaluation",
                    finding.message,
                )),
            }
        }
    }
    if errors.is_empty() {
        Ok(examples)
    } else {
        Err(errors)
    }
}
