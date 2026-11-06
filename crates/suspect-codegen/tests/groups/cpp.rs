//! Integration tests for the `cpp` SDK backend, compiled as one test binary.
//!
//! Each `tests/<file>.rs` is a module here instead of its own binary, so a
//! full test build links one executable per backend rather than one per
//! file. Run one file with `cargo test -p suspect-codegen --test cpp <file>::`.

#[path = "../cpp_attribution.rs"]
mod cpp_attribution;
#[path = "../cpp_credential_env.rs"]
mod cpp_credential_env;
#[path = "../cpp_elimination.rs"]
mod cpp_elimination;
#[path = "../cpp_incoming.rs"]
mod cpp_incoming;
#[path = "../cpp_integration.rs"]
mod cpp_integration;
#[path = "../cpp_oauth.rs"]
mod cpp_oauth;
#[path = "../cpp_pagination.rs"]
mod cpp_pagination;
#[path = "../cpp_protocol.rs"]
mod cpp_protocol;
#[path = "../cpp_sdk.rs"]
mod cpp_sdk;
#[path = "../cpp_stream_typed.rs"]
mod cpp_stream_typed;
