//! Integration tests for the `csharp` SDK backend, compiled as one test binary.
//!
//! Each `tests/<file>.rs` is a module here instead of its own binary, so a
//! full test build links one executable per backend rather than one per
//! file. Run one file with `cargo test -p suspect-codegen --test csharp <file>::`.

#[path = "../csharp_attribution.rs"]
mod csharp_attribution;
#[path = "../csharp_elimination.rs"]
mod csharp_elimination;
#[path = "../csharp_incoming.rs"]
mod csharp_incoming;
#[path = "../csharp_integration.rs"]
mod csharp_integration;
#[path = "../csharp_oauth.rs"]
mod csharp_oauth;
#[path = "../csharp_pagination.rs"]
mod csharp_pagination;
#[path = "../csharp_positional.rs"]
mod csharp_positional;
#[path = "../csharp_protocol.rs"]
mod csharp_protocol;
#[path = "../csharp_sdk.rs"]
mod csharp_sdk;
#[path = "../csharp_stream_typed.rs"]
mod csharp_stream_typed;
