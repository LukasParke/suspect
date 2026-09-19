//! Versioned environment policy lowering over the shared bound declarations.
use super::{Plan, emit::quote};
use crate::http_protocol::{CredentialHook, ParameterLocation};
use std::fmt::Write;

pub(super) fn runtime(plan: &Plan) -> String {
    let mut out =
        include_str!("CredentialEnvironment.kt").replace("__PACKAGE__", &plan.config.package_name);
    out.push_str("\ninternal object EnvironmentCredentials {\n    fun snapshot(environment: CredentialEnvironment): Map<String, ProtocolCredential> {\n        val values = EnvironmentSnapshot(environment)\n        return buildMap {\n");
    for binding in plan
        .credential_env()
        .expect("configured environment")
        .bindings()
    {
        let credential = &plan.credentials[binding.scheme().terminal().source()];
        let validate=match credential.wire.credential() {
            CredentialHook::Bearer{..}=>"bearer(value)".to_owned(),
            CredentialHook::ApiKey{location:ParameterLocation::Header,name}=>format!("protocolHeader(linkedMapOf(), {}, value)",quote(name.value())),
            CredentialHook::ApiKey{location:ParameterLocation::Query,..}=>"protocolPercent(value, \"uri-component\", \"query\", null, false, ProtocolBudget(Json.MAX_BYTES) {})".into(),
            CredentialHook::ApiKey{location:ParameterLocation::Cookie,..}=>"protocolPercent(value, \"none\", \"cookie\", null, false, ProtocolBudget(Json.MAX_BYTES) {})".into(),
            _=>unreachable!("shared credential_env v1 admission"),
        };
        writeln!(
            out,
            "            values.usable({}) {{ value -> {validate} }}?.let {{ put({}, ProtocolCredential.Token(it)) }}",
            quote(binding.variable()),
            quote(&format!("{}#{}",binding.scheme().use_site().source().document(),binding.scheme().use_site().source().pointer()))
        )
        .unwrap();
    }
    out.push_str("        }\n    }\n}\n");
    out
}

pub(super) fn client_constructors() -> &'static str {
    r#"/** Source-selected coroutine client. Close it with Kotlin use. */
public class Client private constructor(
    private val credentials: Credentials,
    private val transport: Transport,
    private val options: ClientOptions,
    private val environmentCredentials: Map<String, ProtocolCredential>?,
) : AutoCloseable {
    /** Use the complete explicit argument without environment reads or fallback. */
    public constructor(credentials: Credentials, transport: Transport = JdkTransport(), options: ClientOptions = ClientOptions()) :
        this(credentials, transport, options, null)

    /** Snapshot configured process variables at client creation. */
    public constructor(transport: Transport = JdkTransport(), options: ClientOptions = ClientOptions()) :
        this(Credentials(), transport, options, EnvironmentCredentials.snapshot(CredentialEnvironment.system))

    /** Explicit creation-time environment factories. */
    public companion object {
        /** Snapshot configured variables once. Missing, empty or unavailable values remain missing until operation authentication. */
        public fun fromEnv(transport: Transport = JdkTransport(), options: ClientOptions = ClientOptions(), environment: CredentialEnvironment = CredentialEnvironment.system): Client =
            Client(Credentials(), transport, options, EnvironmentCredentials.snapshot(environment))
    }
    /** Release this client's transport. */
    override fun close() { try { (transport as? AutoCloseable)?.close() } catch (error: Exception) { throw SdkException(FailureKind.TRANSPORT, "transport cleanup failed", cause = error) } }
"#
}
