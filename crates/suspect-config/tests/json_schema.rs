//! The checked-in JSON Schemas match the setting table.
//!
//! `vsce` packages only the extension directory, and the VS Code settings
//! generator (P4) reads the schema there, so the files live in
//! `editors/vscode/schemas/`.

use std::path::Path;

#[test]
fn the_checked_in_schemas_match_the_setting_table() {
    for (file, schema) in [
        (
            "suspect.schema.json",
            suspect_config::json_schema::settings(),
        ),
        (
            "suspect.project.schema.json",
            suspect_config::json_schema::manifest(),
        ),
    ] {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../editors/vscode/schemas")
            .join(file);
        let expected = serde_json::to_string_pretty(&schema).unwrap() + "\n";
        if std::env::var_os("SUSPECT_BLESS").is_some() {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, &expected).unwrap();
            continue;
        }
        assert_eq!(
            std::fs::read_to_string(&path).unwrap_or_default(),
            expected,
            "{file} is stale; regenerate with `SUSPECT_BLESS=1 cargo test -p suspect-config --test json_schema`"
        );
    }
}
