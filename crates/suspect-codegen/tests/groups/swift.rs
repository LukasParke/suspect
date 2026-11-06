//! Integration tests for the `swift` SDK backend, compiled as one test binary.
//!
//! Each `tests/<file>.rs` is a module here instead of its own binary, so a
//! full test build links one executable per backend rather than one per
//! file. Run one file with `cargo test -p suspect-codegen --test swift <file>::`.

// Support modules shared by several files of this group, loaded once.
#[allow(dead_code)]
#[path = "../../src/swift_sdk/validation_v2_support.rs"]
mod swift_validation_v2_support;

#[path = "../swift_attribution.rs"]
mod swift_attribution;
#[path = "../swift_credential_env.rs"]
mod swift_credential_env;
#[path = "../swift_credential_env_canonical.rs"]
mod swift_credential_env_canonical;
#[path = "../swift_elimination.rs"]
mod swift_elimination;
#[path = "../swift_incoming.rs"]
mod swift_incoming;
#[path = "../swift_oauth.rs"]
mod swift_oauth;
#[path = "../swift_pagination.rs"]
mod swift_pagination;
#[path = "../swift_protocol.rs"]
mod swift_protocol;
#[path = "../swift_sdk.rs"]
mod swift_sdk;
#[path = "../swift_stream_typed.rs"]
mod swift_stream_typed;
#[path = "../swift_validation_v2.rs"]
mod swift_validation_v2;
