//! Optional constructor surface and runtime for the shared bound environment plan.
use crate::credential_env::CredentialEnvPlan;

pub(super) fn runtime(plan: &CredentialEnvPlan) -> String {
    let mut source = include_str!("credential_env.py").to_owned();
    source.push_str("\n_BINDINGS: tuple[tuple[str, str], ...] = (\n");
    for binding in plan.bindings() {
        source.push_str(&format!(
            "    ({}, {}),\n",
            super::native_examples::quote(binding.name()),
            super::native_examples::quote(binding.variable()),
        ));
    }
    source.push_str(")\n");
    source
}

// Private module aliases isolate policy helpers from source-allocated model,
// operation and group names; no public factory name is added to the allocator.
pub(super) const IMPORTS: &str = "from collections.abc import Mapping as _Mapping\nimport httpx as _httpx\nfrom ._types import Credential as _Credential\nfrom . import _credential_env as _credential_env\n";

pub(super) fn constructor(asynchronous: bool) -> String {
    let transport = if asynchronous {
        "AsyncBaseTransport"
    } else {
        "BaseTransport"
    };
    format!(
        "\n    def __init__(self, *, auth: _Mapping[str, _Credential] | None | _credential_env._OmittedAuth = _credential_env._OMITTED_AUTH, server_url: str | None = None,\n                 server: int | str = 0, server_variables: _Mapping[str, str] | None = None,\n                 document_url: str | None = None, auth_alternative: int | None = None,\n                 transport: _httpx.{transport} | None = None, max_response_bytes: int | None = None,\n                 max_capture_bytes: int = 4096, timeout: float | None = 30) -> None:\n        super().__init__(auth=_credential_env._credentials(auth), server_url=server_url, server=server,\n                         server_variables=server_variables, document_url=document_url, auth_alternative=auth_alternative,\n                         transport=transport, max_response_bytes=max_response_bytes, max_capture_bytes=max_capture_bytes, timeout=timeout)\n"
    )
}
