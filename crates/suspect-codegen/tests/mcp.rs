//! Explicit TypeScript MCP tool mapping admission, source-linked refusals,
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
use suspect_codegen::mcp::*;
use suspect_ir::contract::Contract;
use suspect_ref::WorkspaceBuilder;
use suspect_source::Uri;

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/mcp-v1")
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
    parse_mapping(include_str!("fixtures/mcp-v1/mapping.json")).unwrap()
}

fn config() -> McpTargetConfig {
    serde_json::from_str(include_str!("fixtures/mcp-v1/target.json")).unwrap()
}

fn plan() -> ServerPlan {
    plan_server(contract(), mapping(), config()).unwrap()
}

fn files() -> BTreeMap<String, String> {
    emit_server(&plan())
        .into_iter()
        .map(|file| (file.path, file.content))
        .collect()
}

fn codes(errors: &[suspect_codegen::application::Diagnostic]) -> Vec<&str> {
    let mut codes = errors.iter().map(|e| e.code).collect::<Vec<_>>();
    codes.sort_unstable();
    codes
}

/// Every projected tool input schema, keyed by tool name, read back out of the
/// emitted server source so the assertions see exactly what ships.
fn schemas(server: &str) -> BTreeMap<String, Value> {
    let mut found = BTreeMap::new();
    for block in server.split("\nconst schema_").skip(1) {
        let (name, rest) = block.split_once(" = ").expect("generated schema constant");
        let text = rest
            .split_once(" as const;\n")
            .expect("generated schema literal")
            .0;
        found.insert(
            name.to_owned(),
            serde_json::from_str(text).expect("generated schema literal is JSON"),
        );
    }
    found
}

#[test]
fn the_mapping_and_target_configuration_round_trip_through_their_closed_vocabulary() {
    let profile = mapping();
    assert_eq!(profile.format, PROFILE);
    assert_eq!(profile.tools.len(), 3);
    assert_eq!(profile.tools[0].name, "read_widget");
    assert!(profile.tools[0].annotations.read_only);
    assert!(profile.tools[2].annotations.destructive);
    assert_eq!(
        profile.tools[1].input.body,
        BodyBinding::JsonValue {
            property: "widget".into(),
            description: "The complete widget document to create.".into(),
        }
    );
    let text = serde_json::to_string(&profile).unwrap();
    let again: MappingProfile = parse_mapping(&text).unwrap();
    assert_eq!(serde_json::to_string(&again).unwrap(), text);

    let target = config();
    assert_eq!(target.package_name, "widget-mcp-server");
    assert_eq!(target.server_name, "widget-control-plane");
    assert_eq!(target.mcp_server_version, MCP_SERVER_VERSION);
    assert_eq!(target.typescript_version, TYPESCRIPT_VERSION);
    assert_eq!(target.runtime.call_deadline_ms, 30_000);
    assert_eq!(target.logs.policy, LogPolicy::Calls);
    let text = serde_json::to_string(&target).unwrap();
    assert_eq!(
        serde_json::to_string(&serde_json::from_str::<McpTargetConfig>(&text).unwrap()).unwrap(),
        text
    );
}

#[test]
fn unknown_mapping_and_config_fields_are_refused() {
    let mut profile: Value =
        serde_json::from_str(include_str!("fixtures/mcp-v1/mapping.json")).unwrap();
    profile["tools"][0]["retries"] = Value::from(3);
    let error = parse_mapping(&profile.to_string()).unwrap_err();
    assert!(error.to_string().contains("retries"), "{error}");

    let mut profile: Value =
        serde_json::from_str(include_str!("fixtures/mcp-v1/mapping.json")).unwrap();
    profile["tools"][0]["annotations"]["cached"] = Value::from(true);
    assert!(parse_mapping(&profile.to_string()).is_err());

    // Unknown closed-variant tags are errors too, never a silent default.
    let mut profile: Value =
        serde_json::from_str(include_str!("fixtures/mcp-v1/mapping.json")).unwrap();
    profile["tools"][1]["input"]["body"]["kind"] = Value::from("multipart");
    assert!(parse_mapping(&profile.to_string()).is_err());

    let mut target: Value =
        serde_json::from_str(include_str!("fixtures/mcp-v1/target.json")).unwrap();
    target["retries"] = Value::from("auto");
    let error = serde_json::from_str::<McpTargetConfig>(&target.to_string()).unwrap_err();
    assert!(error.to_string().contains("retries"), "{error}");

    let mut target: Value =
        serde_json::from_str(include_str!("fixtures/mcp-v1/target.json")).unwrap();
    target["logs"]["policy"] = Value::from("everything");
    assert!(serde_json::from_str::<McpTargetConfig>(&target.to_string()).is_err());
}

#[test]
fn unknown_profile_versions_are_refused() {
    let mut profile = mapping();
    profile.format = "suspect.application.mcp.v2".into();
    let errors = plan_server(contract(), profile, config()).unwrap_err();
    assert_eq!(codes(&errors), ["mcp-mapping-version"]);
    assert_eq!(errors[0].mapping_pointer, "/format");
}

#[test]
fn missing_and_ambiguous_selectors_keep_their_mapping_pointers() {
    let mut profile = mapping();
    profile.tools[0].selector = "nowhere".into();
    profile.tools[1].selector = "dup".into();
    let errors = plan_server(contract(), profile, config()).unwrap_err();
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
    assert!(pointers.contains(&"/tools/0/selector"), "{pointers:?}");
    assert!(pointers.contains(&"/tools/1/selector"), "{pointers:?}");
    assert!(errors.iter().all(|e| e.at.end > e.at.start));
}

#[test]
fn duplicate_and_reserved_tool_names_are_refused() {
    let mut profile = mapping();
    profile.tools[1].name = profile.tools[0].name.clone();
    let errors = plan_server(contract(), profile, config()).unwrap_err();
    assert_eq!(codes(&errors), ["mcp-tool-collision"]);
    assert_eq!(errors[0].mapping_pointer, "/tools/1/name");

    for reserved in RESERVED_TOOL_NAMES {
        let mut profile = mapping();
        profile.tools[0].name = (*reserved).to_owned();
        let errors = plan_server(contract(), profile, config()).unwrap_err();
        assert_eq!(codes(&errors), ["mcp-tool-reserved"], "{reserved}");
        assert_eq!(errors[0].mapping_pointer, "/tools/0/name");
    }

    for malformed in ["Read Widget", "read-widget-", "9lives", "read/widget", ""] {
        let mut profile = mapping();
        profile.tools[0].name = malformed.to_owned();
        let errors = plan_server(contract(), profile, config()).unwrap_err();
        assert_eq!(codes(&errors), ["mcp-tool-name"], "{malformed:?}");
    }
}

#[test]
fn duplicate_and_malformed_input_property_names_are_refused() {
    let mut profile = mapping();
    profile.tools[0]
        .input
        .parameters
        .get_mut("limit")
        .unwrap()
        .property = "label".into();
    let errors = plan_server(contract(), profile, config()).unwrap_err();
    assert_eq!(codes(&errors), ["mcp-property-collision"]);
    assert_eq!(
        errors[0].mapping_pointer,
        "/tools/0/input/parameters/limit/property"
    );

    // A body property may not collide with a parameter property either.
    let mut profile = mapping();
    profile.tools[1].input.parameters.clear();
    profile.tools[1].selector = "getWidget".into();
    for (name, property) in [
        ("id", "id"),
        ("verbose", "verbose"),
        ("limit", "limit"),
        ("label", "label"),
        ("trace", "widget"),
    ] {
        profile.tools[1].input.parameters.insert(
            name.into(),
            PropertyMapping {
                property: property.into(),
                description: "mapped".into(),
            },
        );
    }
    profile.tools[1].input.body = BodyBinding::None;
    profile.tools[1]
        .input
        .parameters
        .get_mut("trace")
        .unwrap()
        .property = "id".into();
    let errors = plan_server(contract(), profile, config()).unwrap_err();
    assert_eq!(codes(&errors), ["mcp-property-collision"]);

    let mut profile = mapping();
    profile.tools[0]
        .input
        .parameters
        .get_mut("id")
        .unwrap()
        .property = "Widget Id".into();
    let errors = plan_server(contract(), profile, config()).unwrap_err();
    assert_eq!(codes(&errors), ["mcp-property-name"]);
}

#[test]
fn every_source_parameter_needs_exactly_one_mapped_property() {
    let mut profile = mapping();
    profile.tools[0].input.parameters.remove("label");
    let errors = plan_server(contract(), profile, config()).unwrap_err();
    assert_eq!(codes(&errors), ["mcp-parameter-unmapped"]);
    assert!(errors[0].message.contains("label"));

    let mut profile = mapping();
    profile.tools[0].input.parameters.insert(
        "invented".into(),
        PropertyMapping {
            property: "invented".into(),
            description: "not a source parameter".into(),
        },
    );
    let errors = plan_server(contract(), profile, config()).unwrap_err();
    assert_eq!(codes(&errors), ["mcp-parameter-unknown"]);
    assert_eq!(
        errors[0].mapping_pointer,
        "/tools/0/input/parameters/invented"
    );
}

#[test]
fn the_body_binding_must_match_the_source_operation() {
    let mut profile = mapping();
    profile.tools[1].input.body = BodyBinding::None;
    let errors = plan_server(contract(), profile, config()).unwrap_err();
    assert_eq!(codes(&errors), ["mcp-body-policy"]);
    assert_eq!(errors[0].mapping_pointer, "/tools/1/input/body");

    let mut profile = mapping();
    profile.tools[0].input.body = BodyBinding::JsonValue {
        property: "widget".into(),
        description: "there is no request body".into(),
    };
    let errors = plan_server(contract(), profile, config()).unwrap_err();
    assert_eq!(codes(&errors), ["mcp-body-policy"]);
}

#[test]
fn unsupported_media_is_refused_during_generation() {
    let mut profile = mapping();
    profile.tools[0].selector = "getBlob".into();
    profile.tools[0]
        .input
        .parameters
        .retain(|name, _| name == "id");
    let errors = plan_server(contract(), profile, config()).unwrap_err();
    assert_eq!(codes(&errors), ["mcp-unsupported-media"]);
    assert_eq!(errors[0].mapping_pointer, "/tools/0");
    assert!(errors[0].at.end > errors[0].at.start);
    assert!(!errors[0].source.pointer().is_empty());
}

#[test]
fn ambiguous_schema_projections_are_refused_at_plan_time() {
    // A string-or-number union would reach the tool boundary as one JSON
    // string with two possible meanings, so it is refused rather than guessed.
    let mut profile = mapping();
    profile.tools[1].selector = "createSelector".into();
    let errors = plan_server(contract(), profile, config()).unwrap_err();
    assert_eq!(codes(&errors), ["mcp-unsupported-schema"]);
    assert_eq!(errors[0].mapping_pointer, "/tools/1/input/body");
    assert!(errors[0].at.end > errors[0].at.start, "{:?}", errors[0]);
    assert!(
        errors[0].message.contains("string"),
        "{}",
        errors[0].message
    );

    // Arbitrary JSON cannot keep exact numbers across the tool boundary.
    let mut profile = mapping();
    profile.tools[1].selector = "createLoose".into();
    let errors = plan_server(contract(), profile, config()).unwrap_err();
    assert_eq!(codes(&errors), ["mcp-unsupported-schema"]);
    assert!(
        errors[0].message.contains("arbitrary JSON"),
        "{}",
        errors[0].message
    );

    // A self-referencing body has no finite projection.
    let mut profile = mapping();
    profile.tools[1].selector = "createNode".into();
    let errors = plan_server(contract(), profile, config()).unwrap_err();
    assert_eq!(codes(&errors), ["mcp-unsupported-schema"]);
    assert_eq!(errors[0].mapping_pointer, "/tools/1/input/body");
    assert!(
        errors[0].message.contains("recursi"),
        "{}",
        errors[0].message
    );
}

#[test]
fn runtime_bounds_and_identity_are_validated() {
    for mutate in [
        (|c: &mut McpTargetConfig| c.runtime.call_deadline_ms = 0) as fn(&mut McpTargetConfig),
        |c: &mut McpTargetConfig| c.runtime.max_input_bytes = 0,
        |c: &mut McpTargetConfig| c.runtime.max_result_bytes = 0,
    ] {
        let mut broken = config();
        mutate(&mut broken);
        let errors = plan_server(contract(), mapping(), broken).unwrap_err();
        assert_eq!(codes(&errors), ["mcp-runtime-bounds"]);
    }

    let mut broken = config();
    broken.package_name = "Widget MCP".into();
    let errors = plan_server(contract(), mapping(), broken).unwrap_err();
    assert_eq!(codes(&errors), ["mcp-package-identity"]);

    let mut broken = config();
    broken.version = "^1.4".into();
    let errors = plan_server(contract(), mapping(), broken).unwrap_err();
    assert_eq!(codes(&errors), ["mcp-package-identity"]);

    let mut broken = config();
    broken.bin_name = "widget mcp".into();
    let errors = plan_server(contract(), mapping(), broken).unwrap_err();
    assert_eq!(codes(&errors), ["mcp-bin-name"]);

    let mut broken = config();
    broken.server_name = String::new();
    let errors = plan_server(contract(), mapping(), broken).unwrap_err();
    assert_eq!(codes(&errors), ["mcp-server-identity"]);

    let mut broken = config();
    broken.server_url_env = Some("not a variable".into());
    let errors = plan_server(contract(), mapping(), broken).unwrap_err();
    assert_eq!(codes(&errors), ["mcp-server-identity"]);

    // Only the verified official SDK and toolchain pins are implemented.
    for mutate in [
        (|c: &mut McpTargetConfig| c.mcp_server_version = "1.30.0".into())
            as fn(&mut McpTargetConfig),
        |c: &mut McpTargetConfig| c.mcp_client_version = "1.30.0".into(),
        |c: &mut McpTargetConfig| c.typescript_version = "5.4.0".into(),
        |c: &mut McpTargetConfig| c.node_minimum_major = 14,
    ] {
        let mut broken = config();
        mutate(&mut broken);
        let errors = plan_server(contract(), mapping(), broken).unwrap_err();
        assert_eq!(codes(&errors), ["mcp-toolchain-pin"]);
    }

    // The embedded canonical TypeScript SDK is compiled into the application
    // and requires Node 22, so an application may not declare the official MCP
    // SDK's looser floor of 20 - it could not honour it.
    let mut broken = config();
    broken.node_minimum_major = 20;
    let errors = plan_server(contract(), mapping(), broken).unwrap_err();
    assert_eq!(codes(&errors), ["mcp-toolchain-pin"]);
    assert_eq!(errors[0].mapping_pointer, "/node_minimum_major");

    // An operation whose credentials are mandatory cannot be exposed without an
    // explicit credential environment policy: tool inputs never carry secrets.
    let mut broken = config();
    broken.credential_env = None;
    let errors = plan_server(contract(), mapping(), broken).unwrap_err();
    assert_eq!(codes(&errors), ["mcp-credentials-unconfigured"]);
}

#[test]
fn a_self_contained_esm_package_is_emitted() {
    let files = files();
    for path in [
        "package.json",
        "package-lock.json",
        "tsconfig.json",
        "README.md",
        "application-surface.json",
        "server/main.ts",
        "server/runtime.ts",
        "typescript/operations.ts",
        "typescript/model-codecs.ts",
        "typescript/json.ts",
        "typescript/runtime.ts",
        "typescript/http/types.ts",
    ] {
        assert!(files.contains_key(path), "missing {path}");
    }
    // The embedded SDK never keeps its own package metadata or self-importing
    // examples: this output root owns exactly one installable package.
    for absent in [
        "typescript/package.json",
        "typescript/package-lock.json",
        "typescript/tsconfig.json",
        "typescript/examples/first-request.ts",
        "typescript/examples/validated.ts",
    ] {
        assert!(!files.contains_key(absent), "unexpected {absent}");
    }

    let package: Value = serde_json::from_str(&files["package.json"]).unwrap();
    assert_eq!(package["name"], "widget-mcp-server");
    assert_eq!(package["version"], "1.4.0");
    assert_eq!(package["private"], true);
    assert_eq!(package["type"], "module");
    assert_eq!(package["bin"]["widget-mcp"], "./dist/server/main.js");
    assert_eq!(
        package["dependencies"]["@modelcontextprotocol/server"],
        MCP_SERVER_VERSION
    );
    assert_eq!(package["devDependencies"]["typescript"], TYPESCRIPT_VERSION);
    // The client package is an acceptance-test tool, never a server dependency.
    assert!(package["dependencies"]["@modelcontextprotocol/client"].is_null());
    // The floor is the stricter of the official MCP SDK's own minimum and the
    // embedded canonical TypeScript SDK's, which the application compiles in.
    assert_eq!(package["engines"]["node"], ">=22");
    assert_eq!(NODE_MINIMUM_MAJOR, 22);

    let lock: Value = serde_json::from_str(&files["package-lock.json"]).unwrap();
    assert_eq!(lock["lockfileVersion"], 3);
    assert_eq!(lock["name"], "widget-mcp-server");
    assert_eq!(lock["packages"][""]["version"], "1.4.0");
    let server = &lock["packages"]["node_modules/@modelcontextprotocol/server"];
    assert_eq!(server["version"], MCP_SERVER_VERSION);
    assert!(
        server["integrity"].as_str().unwrap().starts_with("sha512-"),
        "{server}"
    );
    assert!(
        lock["packages"]["node_modules/typescript"]["integrity"]
            .as_str()
            .unwrap()
            .starts_with("sha512-")
    );
}

#[test]
fn the_entry_point_is_stdio_only_and_logs_only_on_standard_error() {
    let files = files();
    let server = &files["server/main.ts"];
    let runtime = &files["server/runtime.ts"];
    assert!(server.contains("StdioServerTransport"));
    assert!(server.contains("@modelcontextprotocol/server/stdio"));
    assert!(server.contains("new McpServer("));
    assert!(server.contains("fromJsonSchema"));
    // Only the official SDK writes to standard output.
    for text in [server.as_str(), runtime.as_str()] {
        assert!(!text.contains("process.stdout"), "{text}");
        assert!(!text.contains("console.log("), "{text}");
    }
    assert!(runtime.contains("process.stderr.write("));
    // Neither HTTP transports nor resources, prompts, retries or pagination.
    for absent in [
        "registerResource",
        "registerPrompt",
        "StreamableHTTP",
        "createMcpHandler",
        "Pages(",
        "Items(",
        "NextPage(",
    ] {
        assert!(!server.contains(absent), "unexpected {absent}");
    }
    // Cancellation comes from the request context, combined with a finite
    // per-call deadline compiled from the target configuration.
    assert!(runtime.contains("ctx.mcpReq.signal"));
    assert!(server.contains("CALL_DEADLINE_MS = 30000"));
    assert!(server.contains("MAX_INPUT_BYTES = 262144"));
    assert!(server.contains("MAX_RESULT_BYTES = 1048576"));
    assert!(server.contains("LOG_POLICY = 'calls'"));
}

#[test]
fn argument_transformation_is_a_separate_phase_from_the_api_call() {
    let files = files();
    let server = &files["server/main.ts"];
    let runtime = &files["server/runtime.ts"];
    // Each tool binds its arguments in its own function, so nothing is sent
    // until every argument has been accepted by the generated codecs.
    for symbol in ["read_widget", "create_widget", "purge_widgets"] {
        assert!(
            server.contains(&format!(
                "function bind_{symbol}(args: Record<string, unknown>)"
            )),
            "missing bind_{symbol}"
        );
        assert!(
            server.contains(&format!("async function call_{symbol}(input: Operations.")),
            "missing call_{symbol}"
        );
        assert!(server.contains(&format!("bindArguments: bind_{symbol},")));
        assert!(server.contains(&format!("call: call_{symbol},")));
    }
    // The projected schema carries only type and shape, so a codec rejection is
    // the remaining source assertion failing. It is detected explicitly, and it
    // can never be reported as a condition at the API.
    assert!(runtime.contains("import { ModelCodecError } from '../typescript/codecs.js';"));
    assert!(runtime.contains("error instanceof ModelCodecError"));
    // Every argument runs through the codec seam that names its property, so a
    // rejection can always say which argument to correct.
    assert!(!server.contains(".decode("), "{server}");
    assert_eq!(
        server.matches("toNativeInput(Codecs.").count(),
        schemas(server)
            .values()
            .map(|schema| schema["properties"].as_object().unwrap().len())
            .sum::<usize>()
    );
    let call_phase = runtime
        .split_once("const reply = await binding.call(")
        .expect("generated call phase")
        .1;
    assert!(
        call_phase.contains("'upstream-failure'"),
        "the upstream category belongs to the call phase"
    );
    let argument_phase = runtime
        .split_once("input = binding.bindArguments(args);")
        .expect("generated argument phase")
        .0;
    assert!(
        !argument_phase.contains("'upstream-failure'"),
        "the argument phase must never reach the upstream category"
    );
}

#[test]
fn tool_listing_order_and_names_come_only_from_the_mapping() {
    let server = &files()["server/main.ts"];
    let order: Vec<&str> = server
        .match_indices("server.registerTool(")
        .map(|(at, _)| {
            server[at..]
                .split('\'')
                .nth(1)
                .expect("generated tool name literal")
        })
        .collect();
    assert_eq!(order, ["read_widget", "create_widget", "purge_widgets"]);
    // Registration is unconditional and static: the listing can never depend on
    // runtime state, an environment read or a network reply.
    let registration = server
        .split_once("server.registerTool(")
        .expect("generated registration")
        .1;
    assert!(!registration.contains("if ("), "{registration}");
    assert!(!registration.contains("process.env"));
    assert!(server.contains("title: 'Read widget'"));
    assert!(server.contains("readOnlyHint: true"));
    assert!(server.contains("destructiveHint: true"));
}

#[test]
fn schema_declared_numbers_are_projected_as_exact_token_strings() {
    let files = files();
    let server = &files["server/main.ts"];
    let schemas = schemas(server);
    let read = &schemas["read_widget"];
    assert_eq!(
        read["$schema"],
        "https://json-schema.org/draft/2020-12/schema"
    );
    assert_eq!(read["type"], "object");
    assert_eq!(read["additionalProperties"], false);
    assert_eq!(read["required"], serde_json::json!(["id"]));

    let limit = &read["properties"]["limit"];
    assert_eq!(limit["type"], "string");
    assert!(limit["pattern"].as_str().unwrap().contains("[1-9]"));
    let description = limit["description"].as_str().unwrap();
    assert!(description.contains("exact"), "{description}");
    assert!(description.contains("token"), "{description}");
    // The string projection never inherits numeric JSON Schema assertions.
    for banned in [
        "minimum",
        "maximum",
        "exclusiveMinimum",
        "exclusiveMaximum",
        "multipleOf",
    ] {
        assert!(limit[banned].is_null(), "{banned} leaked onto a string");
    }
    assert_eq!(read["properties"]["verbose"]["type"], "boolean");
    assert_eq!(read["properties"]["label"]["type"], "string");
    assert!(read["properties"]["label"]["pattern"].is_null());

    // A body arrives as one projected JSON value, not a filesystem path.
    let create = &schemas["create_widget"];
    assert_eq!(create["required"], serde_json::json!(["widget"]));
    let widget = &create["properties"]["widget"];
    assert_eq!(widget["type"], "object");
    assert_eq!(widget["properties"]["amount"]["type"], "string");
    assert_eq!(widget["properties"]["name"]["type"], "string");
    assert_eq!(widget["properties"]["active"]["type"], "boolean");
    assert_eq!(widget["properties"]["tags"]["items"]["type"], "string");
    let note = widget["properties"]["note"]["anyOf"].as_array().unwrap();
    let kinds: Vec<&str> = note
        .iter()
        .map(|entry| entry["type"].as_str().unwrap())
        .collect();
    assert_eq!(kinds, ["string", "null"]);
    let mut required = widget["required"].as_array().unwrap().clone();
    required.sort_by_key(|value| value.as_str().unwrap().to_owned());
    assert_eq!(required, [Value::from("amount"), Value::from("name")]);
    assert_eq!(widget["additionalProperties"], false);
    // A closed string enumeration keeps its exact declared members.
    let grade = widget["properties"]["grade"]["anyOf"].as_array().unwrap();
    let members: Vec<&str> = grade
        .iter()
        .map(|entry| entry["const"].as_str().unwrap())
        .collect();
    assert_eq!(members, ["low", "high"]);

    // The matching value adapters name the numeric positions explicitly, and
    // conversion always runs through the generated codecs.
    assert!(server.contains("\"kind\":\"number\""));
    assert!(server.contains("Codecs."));
    assert!(!server.contains("JSON.parse("));
    assert!(!server.contains("JSON.stringify("));
    assert!(files["server/runtime.ts"].contains("stringifyJson"));
    assert!(files["server/runtime.ts"].contains("parseJson"));
}

#[test]
fn no_tool_input_schema_mentions_a_credential() {
    let files = files();
    let server = &files["server/main.ts"];
    for (tool, schema) in schemas(server) {
        let text = schema.to_string();
        assert!(!text.contains("WIDGET_API_KEY"), "{tool}");
        assert!(!text.contains("X-Api-Key"), "{tool}");
        assert!(!text.to_lowercase().contains("apikey"), "{tool}");
        assert!(!text.to_lowercase().contains("authorization"), "{tool}");
    }
    // The server never reads a credential itself; the generated SDK factory is
    // the only credential reader, and only the configured base URL variable is
    // read by the application.
    assert!(server.contains("createClient("));
    assert_eq!(server.matches("process.env").count(), 0);
    assert_eq!(files["server/runtime.ts"].matches("process.env").count(), 1);
    assert!(!server.contains("WIDGET_API_KEY"));
}

#[test]
fn emission_is_byte_identical_for_the_same_inputs() {
    let first = emit_server(&plan());
    let second = emit_server(&plan());
    assert_eq!(first, second);
    let again = emit_server(&plan_server(contract(), mapping(), config()).unwrap());
    assert_eq!(first, again);
    assert!(first.len() > 20);
}

#[test]
fn the_surface_manifest_records_names_requiredness_and_representation() {
    let files = files();
    let surface: Value = serde_json::from_str(&files["application-surface.json"]).unwrap();
    assert_eq!(surface["format"], SURFACE_FORMAT);
    assert_eq!(surface["server"]["name"], "widget-control-plane");
    assert_eq!(surface["server"]["version"], "1.4.0");
    assert_eq!(surface["package"]["name"], "widget-mcp-server");
    assert_eq!(surface["transport"], "stdio");
    assert_eq!(surface["numberRepresentation"], "exact_json_number_token");
    assert_eq!(surface["runtime"]["callDeadlineMs"], 30000);
    assert_eq!(surface["runtime"]["maxInputBytes"], 262_144);
    assert_eq!(surface["runtime"]["maxResultBytes"], 1_048_576);
    assert_eq!(surface["runtime"]["logPolicy"], "calls");
    assert_eq!(
        surface["credentialEnv"]["schemes"]["apiKey"],
        "WIDGET_API_KEY"
    );
    assert_eq!(surface["serverUrlEnv"], "WIDGET_SERVER_URL");
    assert_eq!(surface["sdk"]["mcpServer"], MCP_SERVER_VERSION);

    let tools = surface["tools"].as_array().unwrap();
    assert_eq!(tools.len(), 3);
    let read = &tools[0];
    assert_eq!(read["name"], "read_widget");
    assert_eq!(read["operationId"], "getWidget");
    assert_eq!(read["httpMethod"], "GET");
    assert_eq!(read["httpPath"], "/widgets/{id}");
    assert_eq!(read["source"]["document"], "openapi.json");
    assert_eq!(read["annotations"]["readOnlyHint"], true);
    let inputs: BTreeMap<&str, &Value> = read["input"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| (entry["property"].as_str().unwrap(), entry))
        .collect();
    assert_eq!(inputs["id"]["required"], true);
    assert_eq!(inputs["id"]["in"], "path");
    assert_eq!(inputs["id"]["parameter"], "id");
    assert_eq!(inputs["id"]["representation"], "string");
    assert_eq!(inputs["verbose"]["required"], false);
    assert_eq!(inputs["verbose"]["representation"], "boolean");
    assert_eq!(inputs["limit"]["representation"], "exact_number_token");
    assert_eq!(read["responses"][0]["status"], 200);
    assert_eq!(
        read["responses"][0]["representation"],
        "exact_json_document"
    );
    assert_eq!(read["responses"][0]["success"], true);

    let create = &tools[1];
    assert_eq!(create["input"][0]["property"], "widget");
    assert_eq!(create["input"][0]["in"], "body");
    assert_eq!(create["input"][0]["required"], true);
    assert_eq!(create["input"][0]["mediaType"], "application/json");
    assert_eq!(create["input"][0]["representation"], "exact_json_value");

    let purge = &tools[2];
    assert_eq!(purge["annotations"]["destructiveHint"], true);
    let statuses: Vec<&Value> = purge["responses"].as_array().unwrap().iter().collect();
    assert!(
        statuses
            .iter()
            .any(|entry| entry["status"] == 204 && entry["representation"] == "no_content"),
        "{statuses:?}"
    );

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
// generator path. Both application targets share one scope implementation.
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
        r#"{{"format":"suspect.application.mcp.v1","description":"Document scope probe.",
            "tools":[{{"selector":"{selector}","name":"read_thing","title":"Read thing",
              "description":"Reads exactly one thing.",
              "annotations":{{"read_only":true,"destructive":false,
                "idempotent":true,"open_world":true}},
              "input":{{"parameters":{{"id":{{"property":"id",
                "description":"Exact identifier."}}}},"body":{{"kind":"none"}}}}}}]}}"#
    ))
    .unwrap()
}

/// The probe contract declares no security, so the server is anonymous and
/// needs no credential environment policy.
fn scope_config() -> McpTargetConfig {
    let mut config = config();
    config.credential_env = None;
    config
}

#[test]
fn manifest_records_in_tree_documents_relative_to_the_entry_directory() {
    let plan = plan_server(
        scope_contract(),
        scope_mapping("getThingInTree"),
        scope_config(),
    )
    .unwrap();
    let files: BTreeMap<String, String> = emit_server(&plan)
        .into_iter()
        .map(|file| (file.path, file.content))
        .collect();
    let text = &files["application-surface.json"];
    let surface: Value = serde_json::from_str(text).unwrap();
    let tool = &surface["tools"][0];

    // The entry document itself, and a document in one of its subdirectories,
    // both keep a path relative to the entry directory.
    assert_eq!(tool["source"]["document"], "openapi.json", "{surface}");
    assert_eq!(
        tool["input"][0]["schema"]["document"], "nested/identifier.json",
        "{surface}"
    );
    assert_eq!(
        tool["input"][0]["schema"]["pointer"], "/id/schema",
        "{surface}"
    );
    assert!(!text.contains("file://"), "{text}");
    assert!(!text.contains(env!("CARGO_MANIFEST_DIR")), "{text}");
}

#[test]
fn a_document_outside_the_entry_tree_is_refused_during_planning() {
    let errors = plan_server(
        scope_contract(),
        scope_mapping("getThingOutsideTree"),
        scope_config(),
    )
    .unwrap_err();
    assert_eq!(codes(&errors), ["application-document-outside-entry-tree"]);
    // Located at both the caller's own mapping and the offending contract source.
    assert_eq!(errors[0].mapping_pointer, "/tools/0/input/parameters/id");
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

/// Unlike the Go CLI target, this target's canonical SDK plan does record an
/// operation reached through a referenced Path Item, so the mapping binds and
/// the in-tree referenced document relativizes like any other. What this pins
/// is that the shared binding step reaches a decision either way - it must
/// never panic on ordinary input.
#[test]
fn a_referenced_path_item_plans_and_records_its_document_relatively() {
    let plan = plan_server(
        scope_contract_at("api/referenced-path.json"),
        scope_mapping("getThingByReference"),
        scope_config(),
    )
    .unwrap();
    let files: BTreeMap<String, String> = emit_server(&plan)
        .into_iter()
        .map(|file| (file.path, file.content))
        .collect();
    let text = &files["application-surface.json"];
    let surface: Value = serde_json::from_str(text).unwrap();
    let tool = &surface["tools"][0];
    assert_eq!(tool["operationId"], "getThingByReference", "{surface}");
    assert_eq!(tool["source"]["document"], "nested/paths.json", "{surface}");
    assert!(!text.contains("file://"), "{text}");
}

#[test]
fn the_guide_documents_building_running_and_client_configuration() {
    let readme = &files()["README.md"];
    for fragment in [
        "npm ci",
        "npm run build",
        "dist/server/main.js",
        "mcpServers",
        "WIDGET_API_KEY",
        "WIDGET_SERVER_URL",
        "read_widget",
        "standard error",
    ] {
        assert!(readme.contains(fragment), "missing {fragment}");
    }
    // Building and starting must never need a credential.
    assert!(readme.contains("no credential"), "{readme}");
}

// ---------------------------------------------------------------------------
// Portable native acceptance: install and build the emitted package with its
// own pinned toolchain, then drive the real stdio server with the matching
// official MCP client against a loopback HTTP fixture. No upstream API is
// contacted.
// ---------------------------------------------------------------------------

fn npm(root: &Path, args: &[&str]) {
    let output = Command::new("npm")
        .args(args)
        .current_dir(root)
        .env("npm_config_fund", "false")
        .env("npm_config_audit", "false")
        .env("npm_config_update_notifier", "false")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "npm {args:?} in {}\n{}\n{}",
        root.display(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
#[ignore = "requires the native Node toolchain, npm registry access and python3; loopback fixtures only"]
fn native_mcp_server_builds_and_serves_the_official_client() {
    // The whole attempt is retained on failure; a passing run cleans it up.
    let root = tempfile::Builder::new()
        .prefix("suspect-mcp-")
        .tempdir()
        .unwrap()
        .keep();
    println!("generated application root: {}", root.display());
    let emitted = emit_server(&plan());
    let pinned: BTreeMap<&str, &str> = emitted
        .iter()
        .filter(|file| file.path == "package.json" || file.path == "package-lock.json")
        .map(|file| (file.path.as_str(), file.content.as_str()))
        .collect();
    assert_eq!(pinned.len(), 2);
    suspect_codegen::write_files_with_owner(
        &emitted,
        &root,
        "mcp-native-fixture",
        suspect_codegen::Adoption::Refuse,
    )
    .unwrap();

    // The emitted lock must install and build on its own.
    npm(&root, &["ci", "--ignore-scripts"]);
    npm(&root, &["run", "build"]);
    for (path, expected) in &pinned {
        assert_eq!(
            &std::fs::read_to_string(root.join(path)).unwrap(),
            expected,
            "npm rewrote {path}"
        );
    }
    let entry = root.join("dist/server/main.js");
    assert!(entry.is_file(), "missing built entry point");

    // The official client is an acceptance tool: install it fresh, beside the
    // application, never from a preexisting scratch directory.
    let harness = root.join("acceptance");
    std::fs::create_dir_all(&harness).unwrap();
    std::fs::write(
        harness.join("package.json"),
        format!(
            "{{\n  \"name\": \"suspect-mcp-acceptance\",\n  \"version\": \"0.0.0\",\n  \"private\": true,\n  \"type\": \"module\",\n  \"dependencies\": {{ \"@modelcontextprotocol/client\": \"{MCP_CLIENT_VERSION}\" }}\n}}\n"
        ),
    )
    .unwrap();
    std::fs::write(
        harness.join("client.mjs"),
        include_str!("fixtures/mcp-v1/client.mjs"),
    )
    .unwrap();
    npm(
        &harness,
        &["install", "--ignore-scripts", "--no-package-lock"],
    );

    let script = root.join("loopback.py");
    std::fs::write(&script, include_str!("fixtures/mcp-v1/loopback.py")).unwrap();
    let mut api = Command::new("python3")
        .arg(&script)
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut lines = BufReader::new(api.stdout.take().unwrap()).lines();
    let port = lines.next().unwrap().unwrap();

    let session = Command::new("node")
        .arg(harness.join("client.mjs"))
        .arg(&entry)
        .arg(format!("http://127.0.0.1:{port}/v1"))
        .arg("loopback-key")
        .current_dir(&harness)
        .output()
        .unwrap();
    let _ = api.kill();
    let _ = api.wait();
    assert!(
        session.status.success(),
        "acceptance harness failed\n{}\n{}",
        String::from_utf8_lossy(&session.stdout),
        String::from_utf8_lossy(&session.stderr)
    );
    let transcript = String::from_utf8_lossy(&session.stdout).into_owned();
    let observed: BTreeMap<String, Value> = transcript
        .lines()
        .filter(|line| !line.is_empty())
        .map(|line| {
            let value: Value = serde_json::from_str(line)
                .unwrap_or_else(|error| panic!("{error}: {line}\n{transcript}"));
            (value["case"].as_str().unwrap().to_owned(), value)
        })
        .collect();
    println!("{transcript}");
    assert!(
        !observed.contains_key("fatal"),
        "{:?}",
        observed.get("fatal")
    );

    // Discovery is deterministic: the same order and the same schemas.
    let tools = &observed["tools"];
    assert_eq!(
        tools["names"],
        serde_json::json!(["read_widget", "create_widget", "purge_widgets"])
    );
    assert_eq!(tools["names"], tools["repeated"]);
    let listed = tools["tools"].as_array().unwrap();
    assert_eq!(listed[0]["annotations"]["readOnlyHint"], true);
    assert_eq!(
        listed[0]["inputSchema"]["properties"]["limit"]["type"],
        "string"
    );

    // A valid call preserves an exact large integer in both representations.
    let read = &observed["read"];
    assert_eq!(read["isError"], false, "{read}");
    let text = read["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("9007199254740993"), "{text}");
    let document = &read["structuredContent"]["document"];
    assert_eq!(document["amount"], "9007199254740993");
    assert_eq!(document["id"], "w-1");
    assert_eq!(document["active"], false);
    assert_eq!(document["note"], Value::Null);
    assert_eq!(read["structuredContent"]["status"], 200);
    let echoed = document["echo"]["query"].as_str().unwrap();
    let mut pairs: Vec<&str> = echoed.split('&').collect();
    pairs.sort_unstable();
    assert_eq!(pairs, ["label=", "limit=9007199254740993", "verbose=false"]);
    assert_eq!(document["echo"]["trace"], "t-9");
    assert_eq!(document["echo"]["apiKey"], "loopback-key");

    // Omitted optional properties stay omitted; nothing is defaulted.
    let minimal = &observed["read_minimal"];
    assert_eq!(minimal["isError"], false, "{minimal}");
    assert_eq!(
        minimal["structuredContent"]["document"]["echo"]["query"],
        ""
    );
    assert_eq!(
        minimal["structuredContent"]["document"]["echo"]["trace"],
        Value::Null
    );

    // Schema validation failures are tool results, not protocol errors.
    for case in ["bad_input", "bad_extra"] {
        let failure = &observed[case];
        assert_eq!(failure["outcome"], "result", "{case}: {failure}");
        assert_eq!(failure["isError"], true, "{case}: {failure}");
    }

    // A token the projected schema admits but the source schema rejects is
    // refused by the generated codec, in the argument phase, before anything is
    // sent. It must read as the client's argument to correct - never as a
    // condition at the API, which an agent would answer by retrying.
    let codec = &observed["bad_codec"];
    assert_eq!(codec["outcome"], "result", "{codec}");
    assert_eq!(codec["isError"], true, "{codec}");
    assert_eq!(
        codec["structuredContent"]["failure"], "invalid-argument",
        "{codec}"
    );
    let text = codec["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("read_widget"), "{text}");
    // The rejected argument is named, so a client knows what to correct.
    assert!(text.contains("limit"), "{text}");
    assert!(!text.contains("could not complete"), "{text}");
    assert!(!text.contains("upstream"), "{text}");
    assert!(!text.contains("loopback-key"), "{text}");
    // No raw exception dump: no stack frame and no cause chain.
    assert!(!text.contains("    at "), "{text}");
    // The loopback fixture never saw the call: the argument phase ends first.
    assert!(codec["structuredContent"]["status"].is_null(), "{codec}");

    // A declared upstream failure is a sanitized isError result carrying its
    // exact document, never a raw internal exception.
    let upstream = &observed["upstream_failure"];
    assert_eq!(upstream["isError"], true, "{upstream}");
    assert_eq!(upstream["structuredContent"]["status"], 404);
    assert_eq!(
        upstream["structuredContent"]["document"]["message"],
        "no such widget"
    );
    let upstream_text = upstream["content"][0]["text"].as_str().unwrap();
    assert!(!upstream_text.contains("loopback-key"), "{upstream_text}");

    // A finite JSON body reaches the wire through the generated codec with its
    // exact tokens, in the order the caller supplied, and false, null and the
    // empty array all survive as themselves. Nothing is reordered, defaulted or
    // dropped, and the large integer is never routed through a float.
    let created = &observed["create"];
    assert_eq!(created["isError"], false, "{created}");
    assert_eq!(
        created["structuredContent"]["document"]["echo"]["body"],
        "{\"name\":\"alpha\",\"amount\":9007199254740993,\"active\":false,\"note\":null,\"grade\":\"high\",\"tags\":[]}"
    );
    assert_eq!(observed["create_denied"]["isError"], true);
    assert_eq!(
        observed["create_denied"]["structuredContent"]["status"],
        422
    );

    // A declared no-content response still returns a successful tool result.
    let purge = &observed["purge"];
    assert_eq!(purge["isError"], false, "{purge}");
    assert_eq!(purge["structuredContent"]["status"], 204);
    assert!(purge["structuredContent"]["document"].is_null(), "{purge}");

    // Cancellation reaches the running call rather than hanging.
    let cancelled = &observed["cancel"];
    assert!(
        cancelled["outcome"] == "threw" || cancelled["isError"] == true,
        "{cancelled}"
    );

    // Application logs go only to standard error; the protocol session above
    // could not have completed if any of them had reached standard output.
    let stderr = observed["stderr"]["text"].as_str().unwrap();
    assert!(stderr.contains("read_widget"), "{stderr}");
    assert!(stderr.contains("cancelled"), "{stderr}");
    assert!(!stderr.contains("loopback-key"), "{stderr}");
    assert!(!transcript.contains("jsonrpc"), "{transcript}");

    std::fs::remove_dir_all(&root).unwrap();
}
