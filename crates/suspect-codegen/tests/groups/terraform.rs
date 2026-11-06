//! Integration tests for the `terraform` SDK backend, compiled as one test binary.
//!
//! Each `tests/<file>.rs` is a module here instead of its own binary, so a
//! full test build links one executable per backend rather than one per
//! file. Run one file with `cargo test -p suspect-codegen --test terraform <file>::`.

#[path = "../terraform.rs"]
mod terraform;
#[path = "../terraform_address.rs"]
mod terraform_address;
#[path = "../terraform_errors.rs"]
mod terraform_errors;
#[path = "../terraform_modules.rs"]
mod terraform_modules;
#[path = "../terraform_support.rs"]
mod terraform_support;
