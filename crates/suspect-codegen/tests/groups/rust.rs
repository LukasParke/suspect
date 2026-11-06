//! Integration tests for the `rust` SDK backend, compiled as one test binary.
//!
//! Each `tests/<file>.rs` is a module here instead of its own binary, so a
//! full test build links one executable per backend rather than one per
//! file. Run one file with `cargo test -p suspect-codegen --test rust <file>::`.

#[path = "../rust_codecs.rs"]
mod rust_codecs;
#[path = "../rust_contract.rs"]
mod rust_contract;
#[path = "../rust_credential_env.rs"]
mod rust_credential_env;
#[path = "../rust_http.rs"]
mod rust_http;
#[path = "../rust_http_docs.rs"]
mod rust_http_docs;
#[path = "../rust_http_openrouter.rs"]
mod rust_http_openrouter;
#[path = "../rust_http_runtime.rs"]
mod rust_http_runtime;
#[path = "../rust_http_security.rs"]
mod rust_http_security;
#[path = "../rust_incoming.rs"]
mod rust_incoming;
#[path = "../rust_json.rs"]
mod rust_json;
#[path = "../rust_oauth.rs"]
mod rust_oauth;
#[path = "../rust_pagination.rs"]
mod rust_pagination;
#[path = "../rust_protocol.rs"]
mod rust_protocol;
#[path = "../rust_protocol_resources.rs"]
mod rust_protocol_resources;
#[path = "../rust_protocol_v3.rs"]
mod rust_protocol_v3;
#[path = "../rust_serde.rs"]
mod rust_serde;
#[path = "../rust_stream_typed.rs"]
mod rust_stream_typed;
#[path = "../rust_validation.rs"]
mod rust_validation;
#[path = "../rust_validation_v2.rs"]
mod rust_validation_v2;
#[path = "../rust_validation_v3.rs"]
mod rust_validation_v3;
