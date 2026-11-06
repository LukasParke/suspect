//! Integration tests for the `go` SDK backend, compiled as one test binary.
//!
//! Each `tests/<file>.rs` is a module here instead of its own binary, so a
//! full test build links one executable per backend rather than one per
//! file. Run one file with `cargo test -p suspect-codegen --test go <file>::`.

#[path = "../go_codecs.rs"]
mod go_codecs;
#[path = "../go_credential_env.rs"]
mod go_credential_env;
#[path = "../go_credential_env_canonical.rs"]
mod go_credential_env_canonical;
#[path = "../go_credential_env_factory_capture.rs"]
mod go_credential_env_factory_capture;
#[path = "../go_examples_aggregate.rs"]
mod go_examples_aggregate;
#[path = "../go_http.rs"]
mod go_http;
#[path = "../go_http_names.rs"]
mod go_http_names;
#[path = "../go_incoming.rs"]
mod go_incoming;
#[path = "../go_json.rs"]
mod go_json;
#[path = "../go_models.rs"]
mod go_models;
#[path = "../go_oauth.rs"]
mod go_oauth;
#[path = "../go_pagination.rs"]
mod go_pagination;
#[path = "../go_protocol.rs"]
mod go_protocol;
#[path = "../go_runtime_regressions.rs"]
mod go_runtime_regressions;
#[path = "../go_schema_v2.rs"]
mod go_schema_v2;
#[path = "../go_schema_v3.rs"]
mod go_schema_v3;
#[path = "../go_stream_typed.rs"]
mod go_stream_typed;
#[path = "../go_validation.rs"]
mod go_validation;
