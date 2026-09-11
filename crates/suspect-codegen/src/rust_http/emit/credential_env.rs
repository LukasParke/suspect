//! Optional helpers emitted only for a successfully source-bound environment policy.
use std::collections::BTreeMap;

use crate::credential_env::{CredentialEnvKind, CredentialEnvPlan};

pub(super) fn emit(plan: &CredentialEnvPlan, crate_name: &str) -> String {
    let variables: BTreeMap<_, _> = plan
        .bindings()
        .iter()
        .map(|b| b.variable())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .enumerate()
        .map(|(index, name)| (name, index))
        .collect();
    let mut code = String::from(
        "//! Runtime-only environment credential snapshots. No import-time reads.\n\nimpl crate::Credentials {\n    /// Snapshot mapped variables once. Missing, empty or non-Unicode values stay absent.\n    /// Attachment validation and security alternatives are resolved by each operation.\n    /// Explicit constructors never call this factory or merge environment defaults.\n    #[must_use]\n    pub fn from_env() -> Self {\n",
    );
    for (variable, index) in &variables {
        code.push_str(&format!("        let value_{index} = std::env::var({variable:?}).ok().filter(|value| !value.is_empty());\n"));
    }
    code.push_str(if plan.bindings().is_empty() {
        "        let credentials = Self::new();\n"
    } else {
        "        let mut credentials = Self::new();\n"
    });
    for binding in plan.bindings() {
        let index = variables[binding.variable()];
        let method = match binding.kind() {
            CredentialEnvKind::Bearer => "with_source_bearer",
            CredentialEnvKind::ApiKey => "with_source_api_key",
        };
        let source = super::descriptors::source(binding.scheme().use_site().source());
        code.push_str(&format!("        if let Some(value) = &value_{index} {{\n            credentials = credentials.{method}({source}, value.clone());\n        }}\n"));
    }
    code.push_str("        credentials\n    }\n}\n\nimpl<T> crate::Client<T> {\n    /// Construct a custom-transport client using one creation-time environment snapshot.\n    /// Missing credentials are deferred to protected operations; anonymous calls remain usable.\n    #[must_use]\n    pub fn with_transport_from_env(transport: T) -> Self {\n        Self::with_transport(transport, crate::Credentials::from_env())\n    }\n}\n\n#[cfg(feature = \"reqwest-rustls\")]\nimpl crate::Client<crate::reqwest_transport::ReqwestTransport> {\n    /// Snapshot configured environment variables and construct the standard reqwest client.\n    ///\n    /// # Errors\n    /// Only transport initialization returns an error here. Missing credentials fail\n    /// at a protected operation before HTTP using the existing source-linked SDK errors.\n    ///\n    /// ```no_run\n");
    code.push_str(&format!(
        "    /// let _client = {crate_name}::Client::from_env()?;\n"
    ));
    code.push_str("    /// # Ok::<(), Box<dyn std::error::Error>>(())\n    /// ```\n    pub fn from_env() -> Result<Self, reqwest::Error> {\n        Self::with_reqwest(crate::Credentials::from_env())\n    }\n}\n");
    code
}
