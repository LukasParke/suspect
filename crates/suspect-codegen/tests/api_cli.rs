//! Explicit Go/Cobra CLI mapping admission, source-linked refusals,
//! deterministic emission and a portable native acceptance run.
#![cfg(feature = "http-protocol")]

use std::{
    collections::BTreeMap,
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::Arc,
};

use serde_json::Value;
use suspect_codegen::api_cli::*;
use suspect_ir::contract::Contract;
use suspect_ref::WorkspaceBuilder;
use suspect_source::Uri;

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/api-cli-v1")
}

fn contract() -> Arc<Contract> {
    let root = fixture();
    let workspace = Arc::new(WorkspaceBuilder::new().root(&root).build().unwrap());
    Arc::new(
        Contract::from_workspace(
            &workspace,
            &Uri::from_path(&root.join("openapi.json")).unwrap(),
        )
        .unwrap(),
    )
}

fn mapping() -> MappingProfile {
    parse_mapping(include_str!("fixtures/api-cli-v1/mapping.json")).unwrap()
}

fn config() -> CliTargetConfig {
    serde_json::from_str(include_str!("fixtures/api-cli-v1/target.json")).unwrap()
}

fn plan() -> CliPlan {
    plan_cli(contract(), mapping(), config()).unwrap()
}

fn files() -> BTreeMap<String, String> {
    emit_cli(&plan())
        .into_iter()
        .map(|file| (file.path, file.content))
        .collect()
}

fn codes(errors: &[suspect_codegen::application::Diagnostic]) -> Vec<&str> {
    let mut codes = errors.iter().map(|e| e.code).collect::<Vec<_>>();
    codes.sort_unstable();
    codes
}

#[test]
fn explicit_command_mapping_emits_one_self_contained_go_module() {
    let files = files();
    for path in [
        "go.mod",
        "go.sum",
        "cmd/widgetctl/main.go",
        "internal/cli/runtime.go",
        "internal/cli/commands.go",
        "internal/sdk/operations.go",
        "internal/sdk/codecs.go",
        "internal/sdk/http_runtime.go",
        "internal/sdk/http_environment.go",
        "application-surface.json",
    ] {
        assert!(files.contains_key(path), "missing {path}");
    }
    // The embedded SDK must never keep its own nested module.
    assert!(
        !files
            .keys()
            .any(|path| path.ends_with("internal/sdk/go.mod")),
        "embedded SDK kept a nested go.mod"
    );
    assert!(files.keys().all(|path| !path.starts_with("go/")));

    let module = &files["go.mod"];
    assert!(module.starts_with("module example.com/widget-cli\n"));
    assert!(module.contains("\ngo 1.24.0\n"));
    assert!(module.contains("\ntoolchain go1.27.1\n"));
    assert!(module.contains("github.com/spf13/cobra v1.10.2"));
    assert!(files["go.sum"].contains("github.com/spf13/cobra v1.10.2 h1:"));

    // The SDK is imported from inside this one module, never as a dependency.
    assert!(!module.contains("example.com/widget-cli/internal/sdk"));
    assert!(files["internal/cli/commands.go"].contains("example.com/widget-cli/internal/sdk"));
    assert!(files["cmd/widgetctl/main.go"].contains("example.com/widget-cli/internal/cli"));
}

#[test]
fn every_public_name_comes_from_the_mapping() {
    let files = files();
    let commands = &files["internal/cli/commands.go"];
    for fragment in [
        "Use: \"widgetctl\"",
        "Use: \"widgets\"",
        "Use: \"get\"",
        "Use: \"create\"",
        "Use: \"purge\"",
        "Flags().String(\"id\"",
        "Flags().Bool(\"verbose\"",
        "Flags().String(\"limit\"",
        "Flags().String(\"label\"",
        "Flags().String(\"trace\"",
        "Flags().String(\"scope\"",
        "Flags().String(bodyFlagName",
        "Flags().String(confirmFlagName",
    ] {
        assert!(commands.contains(fragment), "missing {fragment}");
    }
    // The runtime owns those two generated names, so no mapping can shadow them.
    let runtime = &files["internal/cli/runtime.go"];
    assert!(runtime.contains("bodyFlagName    = \"body-file\""));
    assert!(runtime.contains("confirmFlagName = \"confirm\""));
    assert!(runtime.contains("serverFlagName  = \"server-url\""));
    // Presence is always decided by Changed, never by a zero value.
    assert!(commands.contains("Changed(") || files["internal/cli/runtime.go"].contains("Changed("));
    // Exact numbers never pass through a float flag.
    assert!(!commands.contains("Flags().Float"));
    assert!(!commands.contains("Flags().Int"));
    assert!(commands.contains("sdk.ParseInteger("));
    // Only generated codecs touch JSON; encoding/json never sees an SDK model.
    assert!(!commands.contains("encoding/json"));
    assert!(commands.contains("sdk.Codecs."));
}

#[test]
fn credentials_are_constructed_only_when_an_api_command_runs() {
    let files = files();
    let commands = &files["internal/cli/commands.go"];
    // The SDK's own environment factory is the single credential reader.
    assert!(commands.contains("sdk.NewClientFromEnv("));
    assert!(!commands.contains("os.Getenv"));
    assert!(!commands.contains("os.LookupEnv"));
    assert!(!files["internal/cli/runtime.go"].contains("os.Getenv"));
    assert!(!files["internal/cli/runtime.go"].contains("os.LookupEnv"));
    // Client construction sits inside RunE, after help and completion.
    let run = commands.find("RunE:").expect("generated RunE");
    assert!(commands[run..].contains("sdk.NewClientFromEnv("));
    assert!(commands[..run].find("sdk.NewClientFromEnv(").is_none());
}

#[test]
fn unknown_mapping_and_config_fields_are_refused() {
    let mut mapping: Value = serde_json::from_str(include_str!("fixtures/api-cli-v1/mapping.json"))
        .expect("fixture mapping");
    mapping["commands"][0]["retries"] = Value::from(3);
    let error = parse_mapping(&mapping.to_string()).unwrap_err();
    assert!(error.to_string().contains("retries"), "{error}");

    let mut config: Value =
        serde_json::from_str(include_str!("fixtures/api-cli-v1/target.json")).expect("fixture");
    config["pagination"] = Value::from("auto");
    let error = serde_json::from_str::<CliTargetConfig>(&config.to_string()).unwrap_err();
    assert!(error.to_string().contains("pagination"), "{error}");

    // Unknown closed-variant tags are errors too, never a silent default.
    let mut mapping: Value =
        serde_json::from_str(include_str!("fixtures/api-cli-v1/mapping.json")).unwrap();
    mapping["commands"][0]["body"]["kind"] = Value::from("multipart");
    assert!(parse_mapping(&mapping.to_string()).is_err());
}

#[test]
fn unknown_profile_versions_are_refused() {
    let mut profile = mapping();
    profile.format = "suspect.application.cli.v2".into();
    let errors = plan_cli(contract(), profile, config()).unwrap_err();
    assert_eq!(codes(&errors), ["cli-mapping-version"]);
    assert_eq!(errors[0].mapping_pointer, "/format");
}

#[test]
fn missing_and_ambiguous_selectors_keep_their_mapping_pointers() {
    let mut profile = mapping();
    profile.commands[0].selector = "nowhere".into();
    profile.commands[1].selector = "dup".into();
    let errors = plan_cli(contract(), profile, config()).unwrap_err();
    assert_eq!(
        codes(&errors),
        [
            "application-operation-ambiguous",
            "application-operation-missing"
        ]
    );
    let pointers = errors
        .iter()
        .map(|e| e.mapping_pointer.as_str())
        .collect::<Vec<_>>();
    assert!(pointers.contains(&"/commands/0/selector"), "{pointers:?}");
    assert!(pointers.contains(&"/commands/1/selector"), "{pointers:?}");
    assert!(errors.iter().all(|e| e.at.end > e.at.start));
}

#[test]
fn duplicate_and_reserved_command_names_are_refused() {
    let mut profile = mapping();
    profile.commands[1].path = profile.commands[0].path.clone();
    let errors = plan_cli(contract(), profile, config()).unwrap_err();
    assert_eq!(codes(&errors), ["cli-command-collision"]);
    assert_eq!(errors[0].mapping_pointer, "/commands/1/path");

    for reserved in ["help", "completion"] {
        let mut profile = mapping();
        profile.commands[0].path = vec![reserved.into()];
        let errors = plan_cli(contract(), profile, config()).unwrap_err();
        assert_eq!(codes(&errors), ["cli-command-reserved"], "{reserved}");
    }

    // A leaf command may not also be a group of other commands.
    let mut profile = mapping();
    profile.commands[0].path = vec!["widgets".into()];
    let errors = plan_cli(contract(), profile, config()).unwrap_err();
    assert_eq!(codes(&errors), ["cli-command-collision"]);

    // Command segments must be usable, lower-case command tokens.
    let mut profile = mapping();
    profile.commands[0].path = vec!["Widgets".into(), "get".into()];
    let errors = plan_cli(contract(), profile, config()).unwrap_err();
    assert_eq!(codes(&errors), ["cli-command-name"]);
}

#[test]
fn duplicate_and_reserved_flag_names_are_refused() {
    let mut profile = mapping();
    profile.commands[0]
        .parameters
        .get_mut("limit")
        .unwrap()
        .flag = "label".into();
    let errors = plan_cli(contract(), profile, config()).unwrap_err();
    assert_eq!(codes(&errors), ["cli-flag-collision"]);
    assert_eq!(
        errors[0].mapping_pointer,
        "/commands/0/parameters/limit/flag"
    );

    for reserved in ["help", "body-file", "confirm", "server-url", "version"] {
        let mut profile = mapping();
        profile.commands[0].parameters.get_mut("id").unwrap().flag = reserved.into();
        let errors = plan_cli(contract(), profile, config()).unwrap_err();
        assert_eq!(codes(&errors), ["cli-flag-reserved"], "{reserved}");
    }

    let mut profile = mapping();
    profile.commands[0].parameters.get_mut("id").unwrap().flag = "--id".into();
    let errors = plan_cli(contract(), profile, config()).unwrap_err();
    assert_eq!(codes(&errors), ["cli-flag-name"]);
}

#[test]
fn every_source_parameter_needs_exactly_one_mapped_flag() {
    let mut profile = mapping();
    profile.commands[0].parameters.remove("label");
    let errors = plan_cli(contract(), profile, config()).unwrap_err();
    assert_eq!(codes(&errors), ["cli-parameter-unmapped"]);
    assert!(errors[0].message.contains("label"));

    let mut profile = mapping();
    profile.commands[0].parameters.insert(
        "invented".into(),
        ParameterMapping {
            flag: "invented".into(),
            description: "not a source parameter".into(),
        },
    );
    let errors = plan_cli(contract(), profile, config()).unwrap_err();
    assert_eq!(codes(&errors), ["cli-parameter-unknown"]);
    assert_eq!(errors[0].mapping_pointer, "/commands/0/parameters/invented");
}

#[test]
fn unsupported_media_is_refused_during_generation() {
    let mut profile = mapping();
    profile.commands[0].selector = "getBlob".into();
    profile.commands[0]
        .parameters
        .retain(|name, _| name == "id");
    let errors = plan_cli(contract(), profile, config()).unwrap_err();
    assert_eq!(codes(&errors), ["cli-unsupported-media"]);
    assert_eq!(errors[0].mapping_pointer, "/commands/0");
    assert!(errors[0].at.end > errors[0].at.start);
    assert!(!errors[0].source.pointer().is_empty());
}

#[test]
fn unrepresentable_parameters_are_refused_during_generation() {
    let mut profile = mapping();
    profile.commands[0].selector = "getMatrix".into();
    profile.commands[0]
        .parameters
        .retain(|name, _| name == "id");
    let errors = plan_cli(contract(), profile, config()).unwrap_err();
    assert_eq!(codes(&errors), ["cli-parameter-unrepresentable"]);
    assert_eq!(errors[0].mapping_pointer, "/commands/0/parameters/id");
}

#[test]
fn body_policy_must_match_the_source_operation() {
    let mut profile = mapping();
    profile.commands[1].body = BodyPolicy::None;
    let errors = plan_cli(contract(), profile, config()).unwrap_err();
    assert_eq!(codes(&errors), ["cli-body-policy"]);
    assert_eq!(errors[0].mapping_pointer, "/commands/1/body");

    let mut profile = mapping();
    profile.commands[0].body = BodyPolicy::JsonDocument;
    let errors = plan_cli(contract(), profile, config()).unwrap_err();
    assert_eq!(codes(&errors), ["cli-body-policy"]);
}

#[test]
fn runtime_bounds_and_identity_are_validated() {
    let mut broken = config();
    broken.runtime.request_deadline_ms = 0;
    let errors = plan_cli(contract(), mapping(), broken).unwrap_err();
    assert_eq!(codes(&errors), ["cli-runtime-bounds"]);

    let mut broken = config();
    broken.runtime.max_input_bytes = 0;
    let errors = plan_cli(contract(), mapping(), broken).unwrap_err();
    assert_eq!(codes(&errors), ["cli-runtime-bounds"]);

    let mut broken = config();
    broken.binary_name = "Widget Ctl".into();
    let errors = plan_cli(contract(), mapping(), broken).unwrap_err();
    assert_eq!(codes(&errors), ["cli-binary-name"]);

    let mut broken = config();
    broken.module_path = "../escape".into();
    let errors = plan_cli(contract(), mapping(), broken).unwrap_err();
    assert_eq!(codes(&errors), ["cli-package-identity"]);
}

#[test]
fn emission_is_byte_identical_for_the_same_inputs() {
    let first = emit_cli(&plan());
    let second = emit_cli(&plan());
    assert_eq!(first, second);
    let again = emit_cli(&plan_cli(contract(), mapping(), config()).unwrap());
    assert_eq!(first, again);
    assert!(first.len() > 20);
}

#[test]
fn the_surface_manifest_records_names_requiredness_and_representation() {
    let files = files();
    let surface: Value = serde_json::from_str(&files["application-surface.json"]).unwrap();
    assert_eq!(surface["format"], "suspect.application.cli.surface.v1");
    assert_eq!(surface["binary"], "widgetctl");
    assert_eq!(surface["version"], "1.4.0");
    assert_eq!(surface["outputFormat"], "compact_json");
    assert_eq!(surface["runtime"]["requestDeadlineMs"], 30000);
    assert_eq!(surface["runtime"]["maxInputBytes"], 1048576);
    assert_eq!(
        surface["credentialEnv"]["schemes"]["apiKey"],
        "WIDGET_API_KEY"
    );

    let commands = surface["commands"].as_array().unwrap();
    assert_eq!(commands.len(), 3);
    let get = &commands[0];
    assert_eq!(get["name"], "widgets get");
    assert_eq!(get["operationId"], "getWidget");
    assert_eq!(get["httpMethod"], "GET");
    assert_eq!(get["httpPath"], "/widgets/{id}");
    assert_eq!(get["source"]["document"], "openapi.json");
    assert_eq!(get["confirmation"]["kind"], "not_required");
    assert!(get["body"].is_null());
    let flags: BTreeMap<&str, &Value> = get["flags"]
        .as_array()
        .unwrap()
        .iter()
        .map(|flag| (flag["flag"].as_str().unwrap(), flag))
        .collect();
    assert_eq!(flags["id"]["required"], true);
    assert_eq!(flags["id"]["representation"], "string");
    assert_eq!(flags["id"]["in"], "path");
    assert_eq!(flags["verbose"]["required"], false);
    assert_eq!(flags["verbose"]["representation"], "boolean");
    assert_eq!(flags["limit"]["representation"], "exact_integer");
    assert_eq!(flags["limit"]["parameter"], "limit");

    let create = &commands[1];
    assert_eq!(create["name"], "widgets create");
    assert_eq!(create["body"]["flag"], "body-file");
    assert_eq!(create["body"]["required"], true);
    assert_eq!(create["body"]["mediaType"], "application/json");
    assert_eq!(create["body"]["representation"], "exact_json_document");
    assert_eq!(create["responses"][0]["status"], 201);
    assert_eq!(
        create["responses"][0]["representation"],
        "exact_json_document"
    );

    let purge = &commands[2];
    assert_eq!(purge["confirmation"]["kind"], "required");
    assert_eq!(purge["confirmation"]["token"], "purge-widgets");
    assert_eq!(purge["responses"][0]["status"], 204);
    assert_eq!(purge["responses"][0]["representation"], "no_content");

    // Deterministic review artifact: no build time, no generator paths.
    let text = &files["application-surface.json"];
    assert!(!text.contains(env!("CARGO_MANIFEST_DIR")));
    assert!(!text.contains("file://"));
    for banned in ["generatedAt", "timestamp", "Timestamp"] {
        assert!(!text.contains(banned), "{banned}");
    }
}

// ---------------------------------------------------------------------------
// Manifest document scope. A surface manifest is a review artifact that must
// compare equal across machines, so every document it names is recorded
// relative to the entry document's directory - and a document with no such
// relative form is refused during planning rather than leaked as an absolute
// generator path.
// ---------------------------------------------------------------------------

fn scope_contract_at(entry: &str) -> Arc<Contract> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/application-scope");
    let workspace = Arc::new(WorkspaceBuilder::new().root(&root).build().unwrap());
    Arc::new(
        Contract::from_workspace(&workspace, &Uri::from_path(&root.join(entry)).unwrap()).unwrap(),
    )
}

fn scope_contract() -> Arc<Contract> {
    scope_contract_at("api/openapi.json")
}

fn scope_mapping(selector: &str) -> MappingProfile {
    parse_mapping(&format!(
        r#"{{"format":"suspect.application.cli.v1","description":"Document scope probe.",
            "commands":[{{"selector":"{selector}","path":["things","get"],
              "summary":"Read one thing","description":"Reads exactly one thing.",
              "parameters":{{"id":{{"flag":"id","description":"Exact identifier."}}}},
              "body":{{"kind":"none"}},"confirmation":{{"kind":"not_required"}}}}]}}"#
    ))
    .unwrap()
}

/// The probe contract declares no security, so the application is anonymous.
fn scope_config() -> CliTargetConfig {
    let mut config = config();
    config.credential_env = None;
    config
}

#[test]
fn manifest_records_in_tree_documents_relative_to_the_entry_directory() {
    let plan = plan_cli(
        scope_contract(),
        scope_mapping("getThingInTree"),
        scope_config(),
    )
    .unwrap();
    let files: BTreeMap<String, String> = emit_cli(&plan)
        .into_iter()
        .map(|file| (file.path, file.content))
        .collect();
    let text = &files["application-surface.json"];
    let surface: Value = serde_json::from_str(text).unwrap();
    let command = &surface["commands"][0];

    // The entry document itself, and a document in one of its subdirectories,
    // both keep a path relative to the entry directory.
    assert_eq!(command["source"]["document"], "openapi.json", "{surface}");
    assert_eq!(
        command["flags"][0]["schema"]["document"], "nested/identifier.json",
        "{surface}"
    );
    assert_eq!(
        command["flags"][0]["schema"]["pointer"], "/id/schema",
        "{surface}"
    );
    assert!(!text.contains("file://"), "{text}");
    assert!(!text.contains(env!("CARGO_MANIFEST_DIR")), "{text}");
}

#[test]
fn a_document_outside_the_entry_tree_is_refused_during_planning() {
    let errors = plan_cli(
        scope_contract(),
        scope_mapping("getThingOutsideTree"),
        scope_config(),
    )
    .unwrap_err();
    assert_eq!(codes(&errors), ["application-document-outside-entry-tree"]);
    // Located at both the caller's own mapping and the offending contract source.
    assert_eq!(errors[0].mapping_pointer, "/commands/0/parameters/id");
    assert!(
        errors[0]
            .source
            .document()
            .as_str()
            .ends_with("/shared/identifier.json"),
        "{:?}",
        errors[0].source
    );
    assert!(
        errors[0]
            .message
            .contains("outside the entry document's directory"),
        "{}",
        errors[0].message
    );
}

/// A Path Item reached through a `$ref` resolves to a source the canonical Go
/// SDK plan does not record, which this target cannot bind. That is outside
/// the bounded profile, so it must be a located refusal - it was previously an
/// `expect` that panicked on ordinary input.
#[test]
fn a_referenced_path_item_is_refused_with_a_diagnostic_rather_than_a_panic() {
    let errors = plan_cli(
        scope_contract_at("api/referenced-path.json"),
        scope_mapping("getThingByReference"),
        scope_config(),
    )
    .unwrap_err();
    assert_eq!(codes(&errors), ["application-path-item-unplanned"]);
    assert_eq!(errors[0].mapping_pointer, "/commands/0/selector");
    assert!(
        errors[0]
            .source
            .document()
            .as_str()
            .ends_with("/nested/paths.json"),
        "{:?}",
        errors[0].source
    );
    assert!(
        errors[0]
            .message
            .contains("Path Item reached through a `$ref`"),
        "{}",
        errors[0].message
    );
}

#[test]
fn exit_codes_and_credential_variables_are_documented_in_help() {
    let files = files();
    let text = format!(
        "{}{}",
        files["internal/cli/runtime.go"], files["internal/cli/commands.go"]
    );
    assert!(text.contains("WIDGET_API_KEY"));
    for fragment in ["exit 0", "exit 2", "exit 3", "exit 1"] {
        assert!(text.contains(fragment), "missing {fragment}");
    }
}

// ---------------------------------------------------------------------------
// Portable native acceptance: build the emitted module with the real Go
// toolchain and drive the real executable against a loopback HTTP fixture.
// No upstream API is contacted.
// ---------------------------------------------------------------------------

struct Cli {
    binary: PathBuf,
    base: String,
    root: PathBuf,
}

struct Run {
    code: i32,
    stdout: String,
    stderr: String,
}

impl Cli {
    fn run(&self, args: &[&str], stdin: &str) -> Run {
        let mut child = Command::new(&self.binary)
            .args(args)
            .arg("--server-url")
            .arg(&self.base)
            .env_clear()
            .env("WIDGET_API_KEY", "loopback-key")
            .env("HOME", &self.root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        use std::io::Write;
        child
            .stdin
            .take()
            .unwrap()
            .write_all(stdin.as_bytes())
            .unwrap();
        let output = child.wait_with_output().unwrap();
        Run {
            code: output.status.code().unwrap(),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        }
    }
}

fn go(root: &Path, args: &[&str]) {
    let mut command = Command::new("go");
    command
        .args(args)
        .current_dir(root)
        .env("GOWORK", "off")
        // A consumer builds with the default -mod=readonly. Under -mod=mod the
        // toolchain would silently repair a missing require line or go.sum row,
        // so an incomplete pinned dependency set would still pass here while
        // failing for everybody else.
        .env("GOFLAGS", "-mod=readonly");
    if let Some(toolchain) = std::env::var_os("SUSPECT_GO_TOOLCHAIN") {
        command.env("GOTOOLCHAIN", toolchain);
    }
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "go {args:?} in {}\n{}\n{}",
        root.display(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
#[ignore = "requires the native Go toolchain and python3; loopback fixtures only"]
fn native_cli_builds_and_drives_a_loopback_api() {
    // The whole attempt is retained on failure; a passing run cleans it up.
    let root = tempfile::Builder::new()
        .prefix("suspect-api-cli-")
        .tempdir()
        .unwrap()
        .keep();
    println!("generated application root: {}", root.display());
    let emitted = emit_cli(&plan());
    let module: BTreeMap<&str, &str> = emitted
        .iter()
        .filter(|file| file.path == "go.mod" || file.path == "go.sum")
        .map(|file| (file.path.as_str(), file.content.as_str()))
        .collect();
    assert_eq!(module.len(), 2);
    suspect_codegen::write_files_with_owner(
        &emitted,
        &root,
        "api-cli-native-fixture",
        suspect_codegen::Adoption::Refuse,
    )
    .unwrap();

    go(&root, &["build", "./..."]);
    go(&root, &["test", "./..."]);
    go(&root, &["build", "-o", "widgetctl", "./cmd/widgetctl"]);

    // The pinned module metadata must be complete on its own: a readonly build
    // cannot repair it, and it must survive byte-for-byte so a later ownership
    // check never mistakes a toolchain rewrite for a user edit.
    for (path, expected) in &module {
        assert_eq!(
            &std::fs::read_to_string(root.join(path)).unwrap(),
            expected,
            "the Go toolchain rewrote {path}"
        );
    }

    // Help and completion must work with no credentials and no network.
    let help = Command::new(root.join("widgetctl"))
        .arg("--help")
        .env_clear()
        .env("HOME", &root)
        .output()
        .unwrap();
    assert!(help.status.success());
    let help_text = String::from_utf8_lossy(&help.stdout).into_owned();
    assert!(help_text.contains("widgets"), "{help_text}");
    assert!(help_text.contains("WIDGET_API_KEY"), "{help_text}");
    let completion = Command::new(root.join("widgetctl"))
        .args(["completion", "bash"])
        .env_clear()
        .env("HOME", &root)
        .output()
        .unwrap();
    assert!(completion.status.success());

    let script = root.join("loopback.py");
    std::fs::write(&script, include_str!("fixtures/api-cli-v1/loopback.py")).unwrap();
    let mut server = Command::new("python3")
        .arg(&script)
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut lines = BufReader::new(server.stdout.take().unwrap()).lines();
    let port = lines.next().unwrap().unwrap();
    let cli = Cli {
        binary: root.join("widgetctl"),
        base: format!("http://127.0.0.1:{port}/v1"),
        root: root.clone(),
    };

    // A GET with a path parameter plus query and header parameters. Omission,
    // false, zero and the empty string are four distinct wire states.
    let run = cli.run(
        &[
            "widgets",
            "get",
            "--id",
            "w-1",
            "--verbose=false",
            "--limit",
            "0",
            "--label",
            "",
            "--trace",
            "t-9",
        ],
        "",
    );
    assert_eq!(run.code, 0, "{} {}", run.stdout, run.stderr);
    let seen: Value = serde_json::from_str(run.stdout.trim()).unwrap();
    let mut pairs = seen["echo"]["query"]
        .as_str()
        .unwrap()
        .split('&')
        .collect::<Vec<_>>();
    pairs.sort_unstable();
    assert_eq!(pairs, ["label=", "limit=0", "verbose=false"]);
    assert_eq!(seen["echo"]["trace"], "t-9");
    assert_eq!(seen["echo"]["apiKey"], "loopback-key");
    assert_eq!(seen["id"], "w-1");
    // Exact large integers survive the whole response path.
    assert!(run.stdout.contains("9007199254740993"));

    let run = cli.run(&["widgets", "get", "--id", "w-1"], "");
    assert_eq!(run.code, 0, "{}", run.stderr);
    let seen: Value = serde_json::from_str(run.stdout.trim()).unwrap();
    assert_eq!(seen["echo"]["query"], "");
    assert_eq!(seen["echo"]["trace"], Value::Null);

    // A numeric flag reaches the wire as its exact token, never through a float.
    let run = cli.run(
        &[
            "widgets",
            "get",
            "--id",
            "w-1",
            "--limit",
            "9007199254740993",
        ],
        "",
    );
    assert_eq!(run.code, 0, "{}", run.stderr);
    let seen: Value = serde_json::from_str(run.stdout.trim()).unwrap();
    assert_eq!(seen["echo"]["query"], "limit=9007199254740993");
    let run = cli.run(&["widgets", "get", "--id", "w-1", "--limit", "1.5"], "");
    assert_eq!(run.code, 2, "{} {}", run.stdout, run.stderr);
    assert!(
        run.stderr.contains("not an exact integer"),
        "{}",
        run.stderr
    );

    // A POST whose exact JSON body carries a large integer and an explicit null.
    let body = root.join("widget.json");
    std::fs::write(
        &body,
        r#"{"name":"alpha","amount":9007199254740993,"active":false,"note":null,"tags":[]}"#,
    )
    .unwrap();
    let run = cli.run(
        &["widgets", "create", "--body-file", body.to_str().unwrap()],
        "",
    );
    assert_eq!(run.code, 0, "{} {}", run.stdout, run.stderr);
    let seen: Value = serde_json::from_str(run.stdout.trim()).unwrap();
    // The exact bytes the SDK codec put on the wire: the large integer keeps its
    // source token, and false, null and the empty array all survive as themselves.
    assert_eq!(
        seen["echo"]["body"],
        r#"{"active":false,"amount":9007199254740993,"name":"alpha","note":null,"tags":[]}"#
    );
    assert!(run.stdout.contains("9007199254740993"));

    // The same document through standard input.
    let run = cli.run(
        &["widgets", "create", "--body-file", "-"],
        r#"{"name":"alpha","amount":9007199254740993,"active":false,"note":null,"tags":[]}"#,
    );
    assert_eq!(run.code, 0, "{} {}", run.stdout, run.stderr);

    // A documented API failure is a distinct, stable exit code.
    let run = cli.run(
        &["widgets", "create", "--body-file", "-"],
        r#"{"name":"deny","amount":1}"#,
    );
    assert_eq!(run.code, 3, "{} {}", run.stdout, run.stderr);
    let seen: Value = serde_json::from_str(run.stdout.trim()).unwrap();
    assert_eq!(seen["message"], "rejected");
    assert!(run.stderr.contains("422"), "{}", run.stderr);

    // Confirmation is required before a destructive operation is sent.
    let run = cli.run(&["widgets", "purge", "--scope", "staging"], "");
    assert_eq!(run.code, 2);
    assert!(run.stdout.is_empty(), "{}", run.stdout);
    assert!(run.stderr.contains("DELETE /widgets"), "{}", run.stderr);
    let run = cli.run(
        &[
            "widgets",
            "purge",
            "--scope",
            "staging",
            "--confirm",
            "nope",
        ],
        "",
    );
    assert_eq!(run.code, 2);
    let run = cli.run(
        &[
            "widgets",
            "purge",
            "--scope",
            "staging",
            "--confirm",
            "purge-widgets",
        ],
        "",
    );
    assert_eq!(run.code, 0, "{} {}", run.stdout, run.stderr);
    assert!(run.stdout.is_empty(), "{}", run.stdout);
    let run = cli.run(
        &["widgets", "purge", "--scope", "staging", "--confirm", "-"],
        "purge-widgets\n",
    );
    assert_eq!(run.code, 0, "{} {}", run.stdout, run.stderr);

    // A missing required flag is a usage failure, never a request.
    let run = cli.run(&["widgets", "get"], "");
    assert_eq!(run.code, 2);
    assert!(run.stderr.contains("--id"), "{}", run.stderr);

    // An unrecognized command is a usage failure, never a successful help dump.
    for unknown in [vec!["bogus"], vec!["widgets", "bogus"]] {
        let run = cli.run(&unknown, "");
        assert_eq!(run.code, 2, "{unknown:?}: {} {}", run.stdout, run.stderr);
        assert!(run.stderr.contains("unknown command"), "{}", run.stderr);
    }
    // A bare group or root invocation is a help request and succeeds.
    for bare in [vec![], vec!["widgets"]] {
        let run = cli.run(&bare, "");
        assert_eq!(run.code, 0, "{bare:?}: {}", run.stderr);
        assert!(run.stdout.contains("Usage:"), "{}", run.stdout);
    }

    // A runtime failure is its own category.
    let broken = Cli {
        binary: root.join("widgetctl"),
        base: "http://127.0.0.1:1/v1".into(),
        root: root.clone(),
    };
    let run = broken.run(&["widgets", "get", "--id", "w-1"], "");
    assert_eq!(run.code, 1, "{} {}", run.stdout, run.stderr);

    let _ = server.kill();
    let _ = server.wait();
    std::fs::remove_dir_all(&root).unwrap();
}
