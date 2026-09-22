//! C# projection of the shared, source-bound environment credential policy.
use super::{
    SdkPlan,
    emit::{quote, xml},
};
use crate::http_protocol::{CredentialHook, CredentialRequirement, ParameterLocation};

pub(super) const FACTORY: &str = "FromEnvironment";

pub(crate) fn key(requirement: &CredentialRequirement, configured: bool) -> String {
    if configured {
        super::emit::source(requirement.scheme().use_site().source())
    } else {
        super::protocol::credential_key(requirement)
    }
}

pub(super) fn factory(plan: &SdkPlan) -> String {
    let Some(environment) = plan.credential_env() else {
        return String::new();
    };
    let mut out = String::from(
        "    /// <summary>Snapshot configured process-environment credentials now. Missing, empty or unavailable values remain missing; protected calls fail before HTTP. An injected HttpClient stays caller-owned.</summary>\n    /// <remarks>Use the explicit Credentials constructor to supply the entire credential argument. No environment values are read during generation, import or per request.</remarks>\n    public static Client FromEnvironment(ClientOptions? options = null, HttpClient? httpClient = null)\n    {\n        var credentials = new Credentials\n        {\n",
    );
    for binding in environment.bindings() {
        let credential = plan
            .credential_bindings()
            .iter()
            .find(|credential| {
                credential.requirement.scheme().use_site().source()
                    == binding.scheme().use_site().source()
            })
            .expect("bound native declaration");
        let validate = match credential.requirement.credential() {
            CredentialHook::Bearer { .. } => "HttpRuntime.ValidateBearer".into(),
            CredentialHook::ApiKey { location, name } => match location {
                ParameterLocation::Header => format!(
                    "static value => new WireRequest().AddHeader({}, value)",
                    quote(name.value())
                ),
                ParameterLocation::Query => format!(
                    "static value => new WireRequest().AddQuery(WireEncoding.Percent({}, \"uri-component\") + \"=\" + WireEncoding.Percent(value, \"uri-component\"))",
                    quote(name.value())
                ),
                ParameterLocation::Cookie => format!(
                    "static value => new WireRequest().AddCookie(WireEncoding.Percent({}, \"uri-component\") + \"=\" + WireEncoding.Percent(value, \"uri-component\"))",
                    quote(name.value())
                ),
                _ => unreachable!("admitted API-key location"),
            },
            _ => unreachable!("credential_env v1 admits only bearer and API-key strings"),
        };
        out.push_str(&format!("            // Source scheme {}; variable name only.\n            {} = CredentialEnvironment.Read({}, {}),\n",xml(binding.name()),credential.property_name,quote(binding.variable()),validate));
    }
    out.push_str(
        "        };\n        return new Client(credentials, options, httpClient);\n    }\n",
    );
    out
}

pub(super) fn guide(plan: &SdkPlan) -> String {
    let Some(environment) = plan.credential_env() else {
        return String::new();
    };
    let mut out = String::from(
        "\n## Runtime environment credentials\n\n`Client.FromEnvironment(ClientOptions? options = null, HttpClient? httpClient = null)` snapshots the configured process variables at client creation. It preserves the source-default server. An injected HttpClient remains caller-owned; Task, cancellation and client disposal follow the normal Client contract.\n\n| Source scheme | Environment variable name |\n| --- | --- |\n",
    );
    for binding in environment.bindings() {
        out.push_str(&format!(
            "| {} | `{}` |\n",
            xml(binding.name()),
            binding.variable()
        ));
    }
    out.push_str("\n```csharp\nusing var client = Client.FromEnvironment();\n```\n\nOnly variable names are generated. Missing, empty, unavailable or attachment-invalid values remain missing; anonymous operations remain available and protected calls fail before HTTP with a secret-free `SdkException` of kind `Authentication`. The existing explicit Credentials constructor is authoritative for the whole argument, including empty objects, empty/null values and missing members. An explicit null Credentials argument retains its `RequestRepresentation` constructor failure. The parameterless `new Client()` remains the explicit anonymous-client form. Neither import nor requests re-read the environment.\n\nThe supported native targets are .NET 8 and later. Security- or platform-unavailable environment access is treated as missing, without logging exception details. The policy does not load .env files, acquire credentials, refresh tokens or alter authentication alternatives. `credential-env.json` retains source-bound variable-name metadata.\n");
    out
}

/// Configured packages distinguish scheme declarations even when two aliases
/// share a terminal. This keeps an env mapping from leaking into an unmapped alias.
/// Unconfigured packages keep the established source asset byte-for-byte.
pub(super) fn http_runtime(configured: bool) -> String {
    let original = include_str!("HttpRuntime.cs");
    if !configured {
        return original.into();
    }
    let before = "string Key(JsonElement requirement)=>ProtocolRuntime.Source(requirement.GetProperty(\"scheme\").GetProperty(\"terminal\"));";
    let after = "string Key(JsonElement requirement)=>ProtocolRuntime.Source(requirement.GetProperty(\"scheme\").GetProperty(\"use_site\"));";
    assert_eq!(
        original.matches(before).count(),
        1,
        "credential declaration identity seam"
    );
    original.replacen(before, after, 1)
}
