//! The staged `@suspect/rules-sdk` shim reports the toolchain version, so
//! it moves with `cargo xtask bump` instead of a literal.

#[test]
fn staged_sdk_shim_version_is_the_crate_version() {
    let cache = tempfile::tempdir().expect("tempdir");
    let entry = suspect_rules::stage_worker_files(cache.path()).expect("stage worker files");
    let manifest = entry
        .parent()
        .expect("staged dir")
        .join("node_modules/@suspect/rules-sdk/package.json");
    let json: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(manifest).expect("shim manifest"))
            .expect("shim manifest is JSON");
    assert_eq!(json["name"], "@suspect/rules-sdk");
    assert_eq!(json["version"], env!("CARGO_PKG_VERSION"));
}
