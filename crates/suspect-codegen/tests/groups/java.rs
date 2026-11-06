//! Integration tests for the `java` SDK backend, compiled as one test binary.
//!
//! Each `tests/<file>.rs` is a module here instead of its own binary, so a
//! full test build links one executable per backend rather than one per
//! file. Run one file with `cargo test -p suspect-codegen --test java <file>::`.

#[path = "../java_aggregate_examples.rs"]
mod java_aggregate_examples;
#[path = "../java_attribution.rs"]
mod java_attribution;
#[path = "../java_credential_env.rs"]
mod java_credential_env;
#[path = "../java_elimination.rs"]
mod java_elimination;
#[path = "../java_incoming.rs"]
mod java_incoming;
#[path = "../java_integration.rs"]
mod java_integration;
#[path = "../java_oauth.rs"]
mod java_oauth;
#[path = "../java_pagination.rs"]
mod java_pagination;
#[path = "../java_protocol.rs"]
mod java_protocol;
#[path = "../java_schema_v2.rs"]
mod java_schema_v2;
#[path = "../java_schema_v3.rs"]
mod java_schema_v3;
#[path = "../java_sdk.rs"]
mod java_sdk;
#[path = "../java_stream_typed.rs"]
mod java_stream_typed;
