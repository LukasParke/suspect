//! Integration tests for the `python` SDK backend, compiled as one test binary.
//!
//! Each `tests/<file>.rs` is a module here instead of its own binary, so a
//! full test build links one executable per backend rather than one per
//! file. Run one file with `cargo test -p suspect-codegen --test python <file>::`.

// Support modules shared by several files of this group, loaded once.
#[path = "../python_support/mod.rs"]
mod python_support;

#[path = "../python_applicators.rs"]
mod python_applicators;
#[path = "../python_codecs.rs"]
mod python_codecs;
#[path = "../python_credential_env.rs"]
mod python_credential_env;
#[path = "../python_document_servers.rs"]
mod python_document_servers;
#[path = "../python_http.rs"]
mod python_http;
#[path = "../python_http_names.rs"]
mod python_http_names;
#[path = "../python_incoming.rs"]
mod python_incoming;
#[path = "../python_json.rs"]
mod python_json;
#[path = "../python_models.rs"]
mod python_models;
#[path = "../python_null_models.rs"]
mod python_null_models;
#[path = "../python_oauth.rs"]
mod python_oauth;
#[path = "../python_pagination.rs"]
mod python_pagination;
#[path = "../python_protocol.rs"]
mod python_protocol;
#[path = "../python_quickstart.rs"]
mod python_quickstart;
#[path = "../python_resources.rs"]
mod python_resources;
#[path = "../python_runtime_regressions.rs"]
mod python_runtime_regressions;
#[path = "../python_schema_v2.rs"]
mod python_schema_v2;
#[path = "../python_schema_v3.rs"]
mod python_schema_v3;
#[path = "../python_stream_typed.rs"]
mod python_stream_typed;
#[path = "../python_validation.rs"]
mod python_validation;
