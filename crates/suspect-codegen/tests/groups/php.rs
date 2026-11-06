//! Integration tests for the `php` SDK backend, compiled as one test binary.
//!
//! Each `tests/<file>.rs` is a module here instead of its own binary, so a
//! full test build links one executable per backend rather than one per
//! file. Run one file with `cargo test -p suspect-codegen --test php <file>::`.

#[path = "../php_attribution.rs"]
mod php_attribution;
#[path = "../php_credential_env.rs"]
mod php_credential_env;
#[path = "../php_elimination.rs"]
mod php_elimination;
#[path = "../php_incoming.rs"]
mod php_incoming;
#[path = "../php_integration.rs"]
mod php_integration;
#[path = "../php_oauth.rs"]
mod php_oauth;
#[path = "../php_pagination.rs"]
mod php_pagination;
#[path = "../php_protocol.rs"]
mod php_protocol;
#[path = "../php_sdk.rs"]
mod php_sdk;
#[path = "../php_stream_typed.rs"]
mod php_stream_typed;
