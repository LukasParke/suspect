//! Lower the shared, source-bound policy without reading process values.
use super::{Plan, emit::quote};
use crate::http_protocol::{CredentialHook, ParameterLocation};
use std::fmt::Write;

pub(super) fn render(plan: &Plan) -> String {
    let policy = plan
        .credential_env()
        .expect("configured credential environment");
    let mut out = String::from(include_str!("environment.dart"));
    out.push_str("\nCredentials _credentialsFromEnvironment(String? Function(String) read, int maximum) {\n  final values=<String,String?>{};\n  String? snapshot(String name) => values.putIfAbsent(name, () => _snapshotEnvironment(read,name,maximum));\n  return Credentials(\n");
    for binding in policy.bindings() {
        let credential = plan
            .credentials()
            .iter()
            .find(|c| &c.source == binding.scheme().use_site().source())
            .expect("bound native credential declaration");
        let (bearer, header) = match credential.hook {
            CredentialHook::Bearer { .. } => (true, false),
            CredentialHook::ApiKey { location, .. } => {
                (false, location == ParameterLocation::Header)
            }
            _ => unreachable!("shared v1 policy admits string bearer/API keys only"),
        };
        writeln!(
            out,
            "    {}: _environmentCredential(snapshot({}), bearer: {bearer}, header: {header}),",
            credential.name,
            quote(binding.variable())
        )
        .unwrap();
    }
    out.push_str("  );\n}\n");
    out
}

pub(super) fn guide(plan: &Plan) -> String {
    let mut table =
        String::from("| Source scheme | Native member | Variable name |\n| --- | --- | --- |\n");
    for binding in plan.credential_env().expect("configured policy").bindings() {
        let credential = plan
            .credentials()
            .iter()
            .find(|c| &c.source == binding.scheme().use_site().source())
            .expect("bound credential");
        let name = binding
            .name()
            .replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('[', "&#91;")
            .replace(']', "&#93;")
            .replace('|', "&#124;")
            .replace('`', "&#96;")
            .replace(['\r', '\n'], " ");
        writeln!(
            table,
            "| {name} | `{}` | `{}` |",
            credential.name,
            binding.variable()
        )
        .unwrap();
    }
    include_str!("CREDENTIAL-ENV.md").replace("__BINDINGS__", &table)
}
