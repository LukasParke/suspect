//! Integration tests for the shared (cross-backend) SDK backend, compiled as one test binary.
//!
//! Each `tests/<file>.rs` is a module here instead of its own binary, so a
//! full test build links one executable per backend rather than one per
//! file. Run one file with `cargo test -p suspect-codegen --test shared <file>::`.

// Support modules shared by several files of this group, loaded once.
#[path = "../python_support/mod.rs"]
mod python_support;

#[path = "../admission.rs"]
mod admission;
#[path = "../api_cli.rs"]
mod api_cli;
#[path = "../application.rs"]
mod application;
#[path = "../artifact_safety.rs"]
mod artifact_safety;
#[path = "../behavioral_fragments.rs"]
mod behavioral_fragments;
#[path = "../corpus_generation.rs"]
mod corpus_generation;
#[path = "../credential_env.rs"]
mod credential_env;
#[path = "../feature_manifest.rs"]
mod feature_manifest;
#[path = "../fragments.rs"]
mod fragments;
#[path = "../generation_session.rs"]
mod generation_session;
#[path = "../http_admission.rs"]
mod http_admission;
#[path = "../http_protocol.rs"]
mod http_protocol;
#[path = "../m2_browser.rs"]
mod m2_browser;
#[path = "../m2_vertical.rs"]
mod m2_vertical;
#[path = "../m3_native_docs.rs"]
mod m3_native_docs;
#[path = "../mcp.rs"]
mod mcp;
#[path = "../pinned_generation.rs"]
mod pinned_generation;
#[path = "../protocol_examples.rs"]
mod protocol_examples;
#[path = "../scoped_compile.rs"]
mod scoped_compile;
#[path = "../sdk_attribution.rs"]
mod sdk_attribution;
#[path = "../sdk_compat_profiles.rs"]
mod sdk_compat_profiles;
#[path = "../sdk_compatibility.rs"]
mod sdk_compatibility;
#[path = "../sdk_compatibility_attribution.rs"]
mod sdk_compatibility_attribution;
#[path = "../sdk_compatibility_context.rs"]
mod sdk_compatibility_context;
#[path = "../sdk_dialect_models.rs"]
mod sdk_dialect_models;
#[path = "../sdk_example_packages.rs"]
mod sdk_example_packages;
#[path = "../sdk_examples.rs"]
mod sdk_examples;
#[path = "../sdk_generation_options.rs"]
mod sdk_generation_options;
#[path = "../sdk_incoming_plan.rs"]
mod sdk_incoming_plan;
#[path = "../sdk_model_names.rs"]
mod sdk_model_names;
#[path = "../sdk_native_measurements.rs"]
mod sdk_native_measurements;
#[path = "../sdk_oauth_plan.rs"]
mod sdk_oauth_plan;
#[path = "../sdk_pagination.rs"]
mod sdk_pagination;
#[path = "../sdk_stream_plan.rs"]
mod sdk_stream_plan;
#[path = "../sdk_workflow_contract.rs"]
mod sdk_workflow_contract;
#[path = "../sdk_xml_media.rs"]
mod sdk_xml_media;
#[path = "../wave_a_openrouter.rs"]
mod wave_a_openrouter;
