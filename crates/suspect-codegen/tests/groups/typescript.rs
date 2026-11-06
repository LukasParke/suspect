//! Integration tests for the `typescript` SDK backend, compiled as one test binary.
//!
//! Each `tests/<file>.rs` is a module here instead of its own binary, so a
//! full test build links one executable per backend rather than one per
//! file. Run one file with `cargo test -p suspect-codegen --test typescript <file>::`.

#[path = "../typescript_applicators.rs"]
mod typescript_applicators;
#[path = "../typescript_codecs.rs"]
mod typescript_codecs;
#[path = "../typescript_codecs_docs.rs"]
mod typescript_codecs_docs;
#[path = "../typescript_codecs_runtime.rs"]
mod typescript_codecs_runtime;
#[path = "../typescript_contract.rs"]
mod typescript_contract;
#[path = "../typescript_credential_env.rs"]
mod typescript_credential_env;
#[path = "../typescript_directional.rs"]
mod typescript_directional;
#[path = "../typescript_directional_adversarial.rs"]
mod typescript_directional_adversarial;
#[path = "../typescript_directional_package.rs"]
mod typescript_directional_package;
#[path = "../typescript_docs.rs"]
mod typescript_docs;
#[path = "../typescript_elimination.rs"]
mod typescript_elimination;
#[path = "../typescript_http.rs"]
mod typescript_http;
#[path = "../typescript_http_bundle.rs"]
mod typescript_http_bundle;
#[path = "../typescript_http_directional.rs"]
mod typescript_http_directional;
#[path = "../typescript_http_docs.rs"]
mod typescript_http_docs;
#[path = "../typescript_http_runtime.rs"]
mod typescript_http_runtime;
#[path = "../typescript_incoming.rs"]
mod typescript_incoming;
#[path = "../typescript_intersections.rs"]
mod typescript_intersections;
#[path = "../typescript_json.rs"]
mod typescript_json;
#[path = "../typescript_m2_toolchains.rs"]
mod typescript_m2_toolchains;
#[path = "../typescript_multipart_ignored_encoding.rs"]
mod typescript_multipart_ignored_encoding;
#[path = "../typescript_oauth.rs"]
mod typescript_oauth;
#[path = "../typescript_package.rs"]
mod typescript_package;
#[path = "../typescript_pagination.rs"]
mod typescript_pagination;
#[path = "../typescript_protocol.rs"]
mod typescript_protocol;
#[path = "../typescript_protocol_compatibility.rs"]
mod typescript_protocol_compatibility;
#[path = "../typescript_resources.rs"]
mod typescript_resources;
#[path = "../typescript_stream_typed.rs"]
mod typescript_stream_typed;
#[path = "../typescript_validation.rs"]
mod typescript_validation;
