//! Integration tests for the `kotlin` SDK backend, compiled as one test binary.
//!
//! Each `tests/<file>.rs` is a module here instead of its own binary, so a
//! full test build links one executable per backend rather than one per
//! file. Run one file with `cargo test -p suspect-codegen --test kotlin <file>::`.

#![recursion_limit = "512"]

// Support modules shared by several files of this group, loaded once.
#[allow(dead_code)]
#[path = "../kotlin_support/mod.rs"]
mod kotlin_support;

#[path = "../kotlin_attribution.rs"]
mod kotlin_attribution;
#[path = "../kotlin_credential_env.rs"]
mod kotlin_credential_env;
#[path = "../kotlin_elimination.rs"]
mod kotlin_elimination;
#[path = "../kotlin_incoming.rs"]
mod kotlin_incoming;
#[path = "../kotlin_integration.rs"]
mod kotlin_integration;
#[path = "../kotlin_oauth.rs"]
mod kotlin_oauth;
#[path = "../kotlin_pagination.rs"]
mod kotlin_pagination;
#[path = "../kotlin_protocol.rs"]
mod kotlin_protocol;
#[path = "../kotlin_protocol_documents.rs"]
mod kotlin_protocol_documents;
#[path = "../kotlin_sdk.rs"]
mod kotlin_sdk;
#[path = "../kotlin_stream_typed.rs"]
mod kotlin_stream_typed;
#[path = "../kotlin_validation_integration.rs"]
mod kotlin_validation_integration;
#[path = "../kotlin_validation_resource_integration.rs"]
mod kotlin_validation_resource_integration;
#[path = "../kotlin_validation_v2.rs"]
mod kotlin_validation_v2;
#[path = "../kotlin_validation_v3.rs"]
mod kotlin_validation_v3;
