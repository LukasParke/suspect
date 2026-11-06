//! Integration tests for the `ruby` SDK backend, compiled as one test binary.
//!
//! Each `tests/<file>.rs` is a module here instead of its own binary, so a
//! full test build links one executable per backend rather than one per
//! file. Run one file with `cargo test -p suspect-codegen --test ruby <file>::`.

#[path = "../ruby_attribution.rs"]
mod ruby_attribution;
#[path = "../ruby_compatibility_credentials.rs"]
mod ruby_compatibility_credentials;
#[path = "../ruby_credential_env.rs"]
mod ruby_credential_env;
#[path = "../ruby_document_servers.rs"]
mod ruby_document_servers;
#[path = "../ruby_elimination.rs"]
mod ruby_elimination;
#[path = "../ruby_incoming.rs"]
mod ruby_incoming;
#[path = "../ruby_integration.rs"]
mod ruby_integration;
#[path = "../ruby_oauth.rs"]
mod ruby_oauth;
#[path = "../ruby_pagination.rs"]
mod ruby_pagination;
#[path = "../ruby_readme.rs"]
mod ruby_readme;
#[path = "../ruby_schema_v2.rs"]
mod ruby_schema_v2;
#[path = "../ruby_schema_v3.rs"]
mod ruby_schema_v3;
#[path = "../ruby_sdk.rs"]
mod ruby_sdk;
#[path = "../ruby_stream_typed.rs"]
mod ruby_stream_typed;
