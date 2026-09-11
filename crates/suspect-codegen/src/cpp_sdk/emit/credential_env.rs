//! Optional native environment factories. Only declared variable names are
//! emitted; the SDK reads bounded values when the caller invokes a factory.
use super::{SdkPlan, string};
use crate::{
    credential_env::CredentialEnvKind,
    http_protocol::{CredentialHook, ParameterLocation},
};
use std::fmt::Write;

pub(super) fn declarations(plan: &SdkPlan) -> String {
    if plan.credential_env().is_none() {
        return String::new();
    }
    format!(
        "    /// Snapshot configured process variables once; unavailable credentials stay absent.\n    /// Explicit constructors never supplement their credential argument from the environment.\n    [[nodiscard]] static Client from_env_with_transport(std::shared_ptr<const Transport> transport, ClientOptions options = {{}});\n#if defined({}_HAS_CURL)\n    /// Snapshot environment credentials and create the declared verified libcurl adapter.\n    [[nodiscard]] static Result<Client, TransportError> from_env(ClientOptions options = {{}}, CurlOptions curl = {{}});\n#endif\n",
        plan.config.name
    )
}
pub(super) fn definitions(plan: &SdkPlan) -> String {
    let Some(policy) = plan.credential_env() else {
        return String::new();
    };
    let mut out = String::from(
        r#"namespace {
Presence<std::string> credential_env_read(const char* variable) {
    const char* value=std::getenv(variable);
    if(!value)return std::nullopt;
    std::size_t size=0;while(size<=8192&&value[size]!='\0')++size;
    if(!size||size>8192)return std::nullopt;
    return std::string(value,size);
}
Credentials credential_env_snapshot() {
    Credentials credentials;
"#,
    );
    for (index, binding) in policy.bindings().iter().enumerate() {
        let credential = &plan.credentials()[binding.scheme().use_site().source()];
        let valid = match binding.kind() {
            CredentialEnvKind::Bearer => {
                r#"[](std::string_view token) {
            while(!token.empty()&&token.back()=='=')token.remove_suffix(1);
            if(token.empty())return false;
            for(unsigned char c:token)if(!((c>='a'&&c<='z')||(c>='A'&&c<='Z')||(c>='0'&&c<='9'))&&std::string_view("-._~+/").find(static_cast<char>(c))==std::string_view::npos)return false;
            return true;
        }"#
            }
            CredentialEnvKind::ApiKey => match credential.wire.credential() {
                CredentialHook::ApiKey {
                    location: ParameterLocation::Header,
                    ..
                } => "detail::header_value",
                CredentialHook::ApiKey {
                    location: ParameterLocation::Cookie,
                    ..
                } => {
                    r#"[](std::string_view token) {
            for(unsigned char c:token)if(c<=32||c>=127||std::string_view("\",;\\").find(static_cast<char>(c))!=std::string_view::npos)return false;
            return true;
        }"#
                }
                CredentialHook::ApiKey {
                    location: ParameterLocation::Query,
                    ..
                } => "[](std::string_view) { return true; }",
                _ => unreachable!("shared v1 binding requires a supported API key attachment"),
            },
        };
        writeln!(out,"    if(auto value{index}=credential_env_read({}.c_str()); value{index}) {{\n        const auto usable={valid};\n        if(usable(*value{index}))credentials.{}=std::move(*value{index});\n    }}",string(binding.variable()),credential.field_name).unwrap();
    }
    out.push_str("    return credentials;\n}\n} // namespace\nClient Client::from_env_with_transport(std::shared_ptr<const Transport> transport, ClientOptions options) {\n    return Client(std::move(transport), credential_env_snapshot(), std::move(options));\n}\n");
    writeln!(out,"#if defined({}_HAS_CURL)\nResult<Client, TransportError> Client::from_env(ClientOptions options, CurlOptions curl) {{\n    return with_curl(credential_env_snapshot(), std::move(options), std::move(curl));\n}}\n#endif",plan.config.name).unwrap();
    out
}
pub(super) fn documentation(plan: &SdkPlan) -> String {
    let Some(policy) = plan.credential_env() else {
        return String::new();
    };
    let mut out = String::from(
        "\n## Configured runtime environment credentials\n\nThis package includes the explicit v1 environment policy below. Only variable names\nare generated; values are copied with `std::getenv` when an env factory is called.\n\n",
    );
    for binding in policy.bindings() {
        writeln!(
            out,
            "- Source scheme `{}` reads `{}`.",
            super::prose(binding.name()),
            super::prose(binding.variable())
        )
        .unwrap();
    }
    out.push_str("\n`Client::from_env(ClientOptions options = {}, CurlOptions curl = {})` returns\n`Result<Client, TransportError>` and is available with the libcurl build.\n`Client::from_env_with_transport(std::shared_ptr<const Transport>, ClientOptions = {})`\nreturns `Client` and is also available in core-only builds. Both snapshot at\nconstruction; later environment changes affect only new clients. Default server\nselection remains source-backed.\n\nExplicit `Client(transport, credentials, options)` and `Client::with_curl(credentials,\noptions, curl)` never read or fill credentials from the environment, including empty\ncredential objects or absent/empty individual fields. Missing, empty, over-8192-byte,\nor attachment-incompatible environment values remain absent. Protected operations\nthen fail with the existing secret-free `SdkError::Kind::RequestValidation` before\ntransport; anonymous operations remain usable. Environment access and process\nmutation must follow the platform's `std::getenv` synchronization requirements.\n\nNo `.env` files, token acquisition, retries, per-call environment reads, or server\nrewrites are introduced. Normal RAII ownership, stop tokens and deadlines apply.\n");
    out
}
