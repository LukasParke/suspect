//! Integration tests for the `dart` SDK backend, compiled as one test binary.
//!
//! Each `tests/<file>.rs` is a module here instead of its own binary, so a
//! full test build links one executable per backend rather than one per
//! file. Run one file with `cargo test -p suspect-codegen --test dart <file>::`.

#[path = "../dart_attribution.rs"]
mod dart_attribution;
#[path = "../dart_credential_env.rs"]
mod dart_credential_env;
#[path = "../dart_elimination.rs"]
mod dart_elimination;
#[path = "../dart_incoming.rs"]
mod dart_incoming;
#[path = "../dart_integration.rs"]
mod dart_integration;
#[path = "../dart_oauth.rs"]
mod dart_oauth;
#[path = "../dart_pagination.rs"]
mod dart_pagination;
#[path = "../dart_protocol.rs"]
mod dart_protocol;
#[path = "../dart_sdk.rs"]
mod dart_sdk;
#[path = "../dart_stream_typed.rs"]
mod dart_stream_typed;
