#![cfg(feature = "csharp-sdk")]
//! Finite positional multipart: actual source codec roots, typed native values,
//! independent MIME bytes and installed .NET floor/current consumers.
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::Arc,
};
use suspect_codegen::{
    backend::{Backend, TargetConfig},
    compatibility,
    csharp_sdk::{self, SdkConfig, SdkPlan},
    http_protocol::{self as wire, Representation},
};
use suspect_ir::contract::Contract;
use suspect_ref::WorkspaceBuilder;
use suspect_source::Uri;

fn fixture() -> Value {
    let sequence = json!({"schema":{"type":"array","minItems":1,"maxItems":4,"prefixItems":[{"type":"string","minLength":1},{"$ref":"#/components/schemas/Payload"}],"items":{"maxLength":16}},
        "prefixEncoding":[{"contentType":"text/plain","headers":{"X-Ordinal":{"required":true,"schema":{"type":"integer"},"example":1}}},{"contentType":"application/json","headers":{"X-Trace":{"schema":{"type":"string"},"example":"trace"}}}],
        "itemEncoding":{"contentType":"application/octet-stream","headers":{"X-Index":{"required":true,"schema":{"type":"integer"},"example":2}}}});
    let optional = json!({"schema":{"type":"array","prefixItems":[{"type":"string"}],"items":false,"maxItems":1},"prefixEncoding":[]});
    let barrier = json!({"schema":{"type":"array","prefixItems":[{"type":"string"},false,{"type":"number"}],"items":{"type":"integer"},"maxItems":9},"prefixEncoding":[{"contentType":"text/plain"},{"contentType":"application/octet-stream"},{"contentType":"application/json"}]});
    let empty_barrier = json!({"schema":{"type":"array","prefixItems":[false,{"type":"string"}],"items":{"type":"integer"}},"prefixEncoding":[]});
    let repeated = json!({"schema":{"type":"array","minItems":1,"maxItems":2,"items":{"$ref":"#/components/schemas/Payload"}},"itemEncoding":{"contentType":"application/json","headers":{"X-Item":{"schema":{"type":"string"},"example":"item"}}}});
    let fallback = json!({"schema":{"type":"array","prefixItems":[{"type":"string"}],"items":{"type":"integer"},"maxItems":3},"prefixEncoding":[{}, {"contentType":"text/plain","headers":{"X-Number":{"required":true,"schema":{"type":"integer"},"example":2}}}],"itemEncoding":{"contentType":"text/plain"}});
    let disposition = json!({"schema":{"type":"array","prefixItems":[{"type":"string"}],"items":false,"minItems":1,"maxItems":1},"prefixEncoding":[{"headers":{"Content-Disposition":{"required":true,"schema":{"type":"string"},"example":"form-data; name=\"first\""}}}]});
    let tiny = json!({"schema":{"type":"array","prefixItems":[{"maxLength":3}],"items":false,"minItems":1},"prefixEncoding":[{"contentType":"application/octet-stream"}]});
    let mut value = json!({"openapi":"3.2.0","info":{"title":"Positional C# witnesses","version":"1"},"servers":[{"url":"https://parts.example.test/api"}],
        "components":{"schemas":{"Payload":{"type":"object","required":["amount"],"properties":{"amount":{"type":"number"}},"additionalProperties":false}}},"paths":{}});
    for (path, name, media, body, required) in [
        (
            "sequence",
            "sendSequence",
            "multipart/mixed",
            sequence,
            true,
        ),
        (
            "optional",
            "sendOptional",
            "multipart/mixed",
            optional,
            false,
        ),
        ("barrier", "sendBarrier", "multipart/mixed", barrier, true),
        ("empty", "sendEmpty", "multipart/mixed", empty_barrier, true),
        ("repeat", "sendRepeated", "multipart/mixed", repeated, true),
        (
            "fallback",
            "sendFallback",
            "multipart/mixed",
            fallback,
            true,
        ),
        (
            "disposition",
            "sendDisposition",
            "multipart/form-data",
            disposition,
            true,
        ),
        ("tiny", "sendTiny", "multipart/mixed", tiny, true),
    ] {
        value["paths"][format!("/{path}")] = json!({"post":{"operationId":name,"requestBody":{"required":required,"content":{media:body.clone()}},"responses":{"200":{"content":{media:body}}}}});
    }
    value
}
fn directory(prefix: &str) -> PathBuf {
    let parent = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/sdk-csharp-positional");
    fs::create_dir_all(&parent).unwrap();
    tempfile::Builder::new()
        .prefix(prefix)
        .tempdir_in(fs::canonicalize(parent).unwrap())
        .unwrap()
        .keep()
}
fn load(path: &Path) -> Arc<Contract> {
    let ws = Arc::new(
        WorkspaceBuilder::new()
            .root(path.parent().unwrap())
            .build()
            .unwrap(),
    );
    Arc::new(Contract::from_workspace(&ws, &Uri::from_path(path).unwrap()).unwrap())
}
fn plan_in(root: &Path, value: &Value) -> SdkPlan {
    let path = root.join("source.openapi.json");
    fs::write(&path, serde_json::to_vec_pretty(value).unwrap()).unwrap();
    let contract = load(&path);
    let selected = contract
        .operations()
        .map(|op| op.source().clone())
        .collect::<Vec<_>>();
    csharp_sdk::plan_sdk(
        contract,
        &selected,
        SdkConfig {
            name: "Positional.Csharp".into(),
            version: "1.0.0".into(),
            namespace: "Positional.Csharp".into(),
        },
    )
    .unwrap()
}

#[test]
fn positional_projection_uses_only_reachable_part_and_header_codecs() {
    let root = directory("projection-");
    let plan = plan_in(&root, &fixture());
    assert!(
        plan.protocol()
            .capabilities()
            .supports(wire::Capability::PositionalMultipart)
    );
    let sequence = plan
        .operations()
        .iter()
        .find(|o| o.operation_id == "sendSequence")
        .unwrap()
        .body
        .as_ref()
        .unwrap()
        .media[0]
        .positional
        .as_ref()
        .unwrap();
    assert_eq!(sequence.prefix.len(), 2);
    assert!(sequence.prefix[0].wire.required());
    assert!(!sequence.prefix[1].wire.required());
    assert_eq!(sequence.items.as_ref().unwrap().value_type, "byte[]");
    assert!(!plan.protocol().codec_roots().contains(sequence.schema.id()));
    assert!(
        !plan
            .protocol()
            .codec_roots()
            .contains(sequence.items.as_ref().unwrap().wire.schema().id())
    );
    let barrier = plan
        .operations()
        .iter()
        .find(|o| o.operation_id == "sendBarrier")
        .unwrap()
        .body
        .as_ref()
        .unwrap()
        .media[0]
        .positional
        .as_ref()
        .unwrap();
    assert_eq!(barrier.prefix.len(), 1);
    assert!(barrier.items.is_none());
    assert!(
        !plan
            .protocol()
            .codec_roots()
            .iter()
            .any(|id| id.pointer().contains("~1barrier/")
                && (id.pointer().ends_with("prefixItems/2") || id.pointer().ends_with("/items")))
    );
    let empty = plan
        .operations()
        .iter()
        .find(|o| o.operation_id == "sendEmpty")
        .unwrap()
        .body
        .as_ref()
        .unwrap()
        .media[0]
        .positional
        .as_ref()
        .unwrap();
    assert!(empty.prefix.is_empty() && empty.items.is_none());
    let fallback = plan
        .operations()
        .iter()
        .find(|o| o.operation_id == "sendFallback")
        .unwrap()
        .body
        .as_ref()
        .unwrap()
        .media[0]
        .positional
        .as_ref()
        .unwrap();
    assert_eq!(fallback.prefix.len(), 2);
    assert_eq!(
        fallback.prefix[1].wire.schema().id(),
        fallback.items.as_ref().unwrap().wire.schema().id()
    );
    assert_eq!(fallback.prefix[1].headers.len(), 1);
    assert!(fallback.items.as_ref().unwrap().headers.is_empty());
    let target = TargetConfig {
        backend: Backend::CsharpHttp,
        package_name: "Positional.Csharp".into(),
        package_version: "1.0.0".into(),
        import_name: Some("Positional.Csharp".into()),
    };
    let before =
        compatibility::snapshot(plan.contract().clone(), &[], std::slice::from_ref(&target))
            .unwrap();
    assert!(before.native[0].findings.is_empty());
    let recorded = before.native[0]
        .models
        .iter()
        .find(|m| m.role == "positional-parts" && m.name.ends_with(&barrier.native_type))
        .unwrap();
    assert_eq!(
        recorded.descriptor.as_ref().unwrap()["closedAfterPrefix"],
        true
    );
    assert_eq!(recorded.descriptor.as_ref().unwrap()["prefixLength"], 1);
    let mut changed = fixture();
    changed["paths"]["/sequence"]["post"]["requestBody"]["content"]["multipart/mixed"]["schema"]
        ["minItems"] = json!(2);
    let after = plan_in(&root, &changed);
    let after = compatibility::snapshot(after.contract().clone(), &[], &[target]).unwrap();
    let report = compatibility::compare_snapshots(&before, &after);
    assert!(
        report.native[0]
            .changes
            .iter()
            .any(|c| c.code == "native-model-shape-changed"
                && c.subject.contains("SendSequenceRequestPositional"))
    );
    assert!(!report.wire.is_empty());
}

#[test]
fn undefined_positional_styles_are_located_refusals() {
    let root = directory("decline-");
    let mut value = fixture();
    // RFC6570 fields are active for form-data, but ignored for multipart/mixed.
    // An active positional style still lacks the source field name it requires.
    let content = value["paths"]["/optional"]["post"]["requestBody"]["content"]
        .as_object_mut()
        .unwrap();
    let mut media = content.remove("multipart/mixed").unwrap();
    media["prefixEncoding"] = json!([{"style":"form","explode":false,"contentType":"application/json",
        "headers":{"Content-Disposition":{"required":true,"schema":{"type":"string"},"example":"form-data; name=\"value\""}}}]);
    content.insert("multipart/form-data".into(), media);
    let path = root.join("source.openapi.json");
    fs::write(&path, value.to_string()).unwrap();
    let contract = load(&path);
    let selected = contract
        .operations()
        .filter(|op| op.operation_id() == Some("sendOptional"))
        .map(|op| op.source().clone())
        .collect::<Vec<_>>();
    let protocol = wire::plan(&contract, &selected, csharp_sdk::protocol::capabilities());
    assert!(protocol.is_admitted(), "{:?}", protocol.diagnostics());
    let Representation::Multipart {
        multipart: wire::MultipartPlan::Positional { prefix, .. },
    } = protocol.operations()[0].body().unwrap().media()[0].representation()
    else {
        panic!("active positional form-data")
    };
    assert!(matches!(
        prefix[0].representation(),
        wire::PartRepresentation::Style { .. }
    ));
    assert!(
        prefix[0].content_types().is_empty(),
        "active style ignores contentType"
    );
    let source = prefix[0]
        .encoding_source()
        .unwrap()
        .terminal()
        .source()
        .clone();
    let errors = csharp_sdk::plan_sdk(contract.clone(), &selected, Default::default()).unwrap_err();
    assert!(errors.iter().any(|e| {
        e.code == "csharp-positional-style-unsupported"
            && e.source == source
            && e.source
                .pointer()
                .ends_with("multipart~1form-data/prefixEncoding/0")
            && contract.source_span(&e.source) == Some(e.at.clone())
            && e.at.end > e.at.start
    }));
}

fn ignored_encoding_fixture() -> Value {
    let fixture: Value =
        serde_json::from_str(include_str!("fixtures/http-protocol-v1.json")).unwrap();
    fixture["ignoredMultipartEncoding"].clone()
}

#[test]
fn ignored_positional_styles_preserve_native_content_profile() {
    let root = directory("ignored-content-");
    let witness = ignored_encoding_fixture();
    let styled = plan_in(&root, &witness["document"]);
    let operation = &styled.operations()[0];
    for media in [
        &operation.body.as_ref().unwrap().media[0],
        &operation.responses[0].media[0],
    ] {
        let parts = media.positional.as_ref().unwrap();
        assert_eq!(parts.prefix.len(), 4);
        for (part, expected) in parts
            .prefix
            .iter()
            .zip(witness["prefixMedia"].as_array().unwrap())
        {
            assert_eq!(
                part.wire.content_types()[0].declared(),
                expected.as_str().unwrap()
            );
            assert_eq!(part.wire.multiplicity(), wire::PartMultiplicity::One);
        }
        assert!(matches!(
            parts.prefix[0].wire.representation(),
            wire::PartRepresentation::Json {
                outer_encoding: wire::PercentEncoding::None,
                ..
            }
        ));
        assert_eq!(parts.prefix[0].value_type, "string");
        assert!(parts.prefix[0].headers[0].wire.required());
        assert!(matches!(
            parts.prefix[1].wire.representation(),
            wire::PartRepresentation::Text {
                scalar: wire::ScalarType::Integer,
                outer_encoding: wire::PercentEncoding::None,
                ..
            }
        ));
        assert_eq!(parts.prefix[1].value_type, "JsonInteger");
        assert!(matches!(
            parts.prefix[2].wire.representation(),
            wire::PartRepresentation::Json { .. }
        ));
        assert_eq!(
            parts.prefix[2].value_type,
            "global::System.Collections.Generic.List<string>"
        );
        assert!(matches!(
            parts.prefix[3].wire.representation(),
            wire::PartRepresentation::Binary { .. }
        ));
        assert_eq!(parts.prefix[3].value_type, "byte[]");
        let tail = parts.items.as_ref().unwrap();
        assert!(matches!(
            tail.wire.representation(),
            wire::PartRepresentation::Json { .. }
        ));
        assert_eq!(
            tail.wire.content_types()[0].declared(),
            witness["itemMedia"].as_str().unwrap()
        );
        assert!(!styled.protocol().codec_roots().contains(parts.schema.id()));
        assert!(
            !styled
                .protocol()
                .codec_roots()
                .contains(parts.prefix[3].wire.schema().id())
        );
    }
    assert_eq!(
        styled
            .protocol()
            .codec_roots()
            .iter()
            .map(|id| id.pointer())
            .collect::<Vec<_>>(),
        witness["codecRoots"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect::<Vec<_>>()
    );
    for suffix in [
        "prefixEncoding/0/style",
        "prefixEncoding/0/explode",
        "prefixEncoding/1/style",
        "prefixEncoding/2/explode",
        "prefixEncoding/3/allowReserved",
        "itemEncoding/style",
        "itemEncoding/explode",
        "itemEncoding/allowReserved",
    ] {
        let expected = format!("/components/mediaTypes/Mixed/{suffix}");
        assert!(
            styled.protocol().diagnostics().iter().any(|d| d.code()
                == "http-encoding-style-ignored"
                && d.severity() == wire::Severity::Warning
                && d.source().source().pointer() == expected
                && styled.contract().source_span(d.source().source()) == Some(d.source().span())
                && !d.source().span().is_empty()),
            "missing physical ignored-field warning: {expected}"
        );
    }
    assert!(
        !styled
            .protocol()
            .diagnostics()
            .iter()
            .any(|d| d.code() == "http-encoding-content-type-ignored")
    );
    // The ignored metadata must produce the same native models, codecs, HTTP
    // constructors and runtime source as the corresponding content-only source.
    let mut content_only = witness["document"].clone();
    let media = &mut content_only["components"]["mediaTypes"]["Mixed"];
    for encoding in media["prefixEncoding"].as_array_mut().unwrap() {
        for name in ["style", "explode", "allowReserved"] {
            encoding.as_object_mut().unwrap().remove(name);
        }
    }
    for name in ["style", "explode", "allowReserved"] {
        media["itemEncoding"].as_object_mut().unwrap().remove(name);
    }
    let plain = plan_in(&root, &content_only);
    assert_eq!(
        styled.protocol().codec_roots(),
        plain.protocol().codec_roots()
    );
    assert_eq!(
        serde_json::to_vec(styled.program()).unwrap(),
        serde_json::to_vec(plain.program()).unwrap()
    );
    let styled_files = styled.render().unwrap();
    let plain_files = plain.render().unwrap();
    let native = |files: &[suspect_codegen::OutFile]| {
        files
            .iter()
            .filter(|f| f.path.ends_with(".cs"))
            .map(|f| (f.path.clone(), f.content.clone()))
            .collect::<Vec<_>>()
    };
    suspect_codegen::write_files(&styled_files, &root.join("ignored-fields")).unwrap();
    suspect_codegen::write_files(&plain_files, &root.join("content-only")).unwrap();
    let styled_native = native(&styled_files);
    let plain_native = native(&plain_files);
    assert_eq!(styled_native.len(), plain_native.len());
    for ((styled_path, styled_source), (plain_path, plain_source)) in
        styled_native.iter().zip(&plain_native)
    {
        assert_eq!(styled_path, plain_path);
        assert!(
            styled_source == plain_source,
            "ignored encoding changed native source: {styled_path}"
        );
    }
    fs::write(
        root.join("ignored-source.json"),
        serde_json::to_vec_pretty(&witness["document"]).unwrap(),
    )
    .unwrap();
    fs::write(root.join("PASS.json"),json!({"result":"passed","gate":"ignored-positional-content-native-profile","nativeSourceFiles":native(&styled_files).len(),"programBytesEqual":true,"codecRoots":styled.protocol().codec_roots().len()}).to_string()).unwrap();
    println!("C# ignored encoding profile evidence: {}", root.display());
}

#[test]
fn ignored_positional_styles_cannot_hide_invalid_content_or_metadata() {
    let witness = ignored_encoding_fixture();
    for (field, value, schema, code, suffix) in [
        (
            "contentType",
            json!("text/plain"),
            Some(json!({"type":"object","properties":{"x":{"type":"string"}}})),
            "http-scalar-shape-unsupported",
            "schema/prefixItems/0",
        ),
        (
            "contentType",
            json!("application/json, text/plain"),
            None,
            "http-part-mixed-representations",
            "prefixEncoding/0",
        ),
        (
            "contentType",
            json!("application/octet-stream"),
            None,
            "http-binary-schema-type",
            "schema/prefixItems/0/type",
        ),
        (
            "contentType",
            json!("not a media type"),
            None,
            "http-media-type-invalid",
            "prefixEncoding/0/contentType",
        ),
        (
            "explode",
            json!("false"),
            None,
            "http-metadata-boolean",
            "prefixEncoding/0/explode",
        ),
        (
            "style",
            json!(false),
            None,
            "http-metadata-string",
            "prefixEncoding/0/style",
        ),
    ] {
        let root = directory("ignored-invalid-");
        let mut document = witness["document"].clone();
        let media = &mut document["components"]["mediaTypes"]["Mixed"];
        media["prefixEncoding"][0][field] = value;
        if let Some(schema) = schema {
            media["schema"]["prefixItems"][0] = schema;
        }
        let path = root.join("source.openapi.json");
        fs::write(&path, serde_json::to_vec_pretty(&document).unwrap()).unwrap();
        let contract = load(&path);
        let selected = contract
            .operations()
            .map(|op| op.source().clone())
            .collect::<Vec<_>>();
        let errors =
            csharp_sdk::plan_sdk(contract.clone(), &selected, Default::default()).unwrap_err();
        let expected = format!("/components/mediaTypes/Mixed/{suffix}");
        assert!(
            errors.iter().any(|e| e.code == code
                && e.source.document() == contract.entry()
                && e.source.pointer() == expected
                && contract.source_span(&e.source) == Some(e.at.clone())
                && e.at.end > e.at.start),
            "{code}: {errors:#?}"
        );
        fs::write(root.join("findings.txt"), format!("{errors:#?}")).unwrap();
    }
}

#[test]
fn documented_httpclient_method_case_refusal_is_source_located() {
    let root = directory("method-case-");
    let value = json!({"openapi":"3.2.0","info":{"title":"Case witness","version":"1"},"paths":{"/case":{"additionalOperations":{"hEaD":{"operationId":"caseWitness","responses":{"200":{"content":{"application/octet-stream":{}}}}}}}}});
    let path = root.join("source.openapi.json");
    fs::write(&path, value.to_string()).unwrap();
    let contract = load(&path);
    let selected = contract
        .operations()
        .map(|op| op.source().clone())
        .collect::<Vec<_>>();
    let errors = csharp_sdk::plan_sdk(contract, &selected, Default::default()).unwrap_err();
    assert!(
        errors
            .iter()
            .any(|e| e.code == "csharp-method-case-unsupported"
                && e.source.pointer().ends_with("additionalOperations/hEaD")
                && e.at.end > e.at.start)
    );
}

#[test]
fn canonical_generation_options_are_identical_in_generation_and_snapshot_capture() {
    use suspect_codegen::backend::{self, GenerationOptions};
    let root = directory("options-");
    let mut value = fixture();
    value["paths"]["/tiny"]["post"]["requestBody"]["content"]["multipart/mixed"]["schema"]["prefixItems"]
        [0] = json!({"type":"string","format":"binary"});
    let path = root.join("source.openapi.json");
    fs::write(&path, value.to_string()).unwrap();
    let contract = load(&path);
    let selected = contract
        .operations()
        .filter(|o| o.operation_id() == Some("sendTiny"))
        .map(|o| o.source().clone())
        .collect::<Vec<_>>();
    let target = TargetConfig {
        backend: Backend::CsharpHttp,
        package_name: "Positional.Csharp".into(),
        package_version: "1.0.0".into(),
        import_name: Some("Positional.Csharp".into()),
    };
    assert!(backend::generate(contract.clone(), &selected, &target).is_err());
    let declined = compatibility::snapshot(
        contract.clone(),
        &["sendTiny".into()],
        std::slice::from_ref(&target),
    )
    .unwrap();
    assert_eq!(
        declined.native[0].status,
        compatibility::PlanStatus::Unavailable
    );
    let generation = GenerationOptions {
        compatibility_profiles: [wire::CompatibilityProfile::LegacyBinaryStringV1]
            .into_iter()
            .collect(),
        ..Default::default()
    };
    let files =
        backend::generate_with_options(contract.clone(), &selected, &target, &generation).unwrap();
    assert!(files.iter().all(|f| f.path.starts_with("csharp/")));
    let snapshot = compatibility::snapshot_with_options(
        contract,
        &["sendTiny".into()],
        &[target],
        &generation,
    )
    .unwrap();
    let native = &snapshot.native[0];
    assert_eq!(native.generation, generation);
    assert_eq!(native.status, compatibility::PlanStatus::Planned);
    assert!(native.findings.is_empty());
    let part = native
        .models
        .iter()
        .find(|m| m.role == "positional-part" && m.name.contains("SendTinyRequest"))
        .unwrap();
    assert_eq!(
        part.descriptor.as_ref().unwrap()["fields"][0]["type"],
        json!({"kind":"array","type":{"kind":"primitive","name":"byte"}})
    );
}

#[test]
#[ignore = "focused .NET 8/10 installed positional multipart MIME/type/docs/cleanup witness"]
fn native_positional_multipart_matrix() {
    let root = directory("native-");
    let plan = plan_in(&root, &fixture());
    let dotnet = std::env::var_os("SUSPECT_DOTNET_BIN")
        .unwrap_or_else(|| "/Users/luke/.local/share/mise/dotnet-root/dotnet".into());
    for (sdk, framework) in [("8.0.424", "net8.0"), ("10.0.400", "net10.0")] {
        let case = root.join(sdk);
        fs::create_dir_all(case.join("feed")).unwrap();
        fs::write(
            case.join("global.json"),
            json!({"sdk":{"version":sdk,"rollForward":"disable"}}).to_string(),
        )
        .unwrap();
        fs::write(case.join("NuGet.Config"),"<configuration><packageSources><clear/><add key=\"local\" value=\"feed\"/></packageSources></configuration>").unwrap();
        suspect_codegen::write_files(&plan.render().unwrap(), &case).unwrap();
        let run = |dir: &str, label: &str, args: &[&str]| {
            let mut command = Command::new(&dotnet);
            command
                .args(args)
                .current_dir(case.join(dir))
                .env("DOTNET_CLI_TELEMETRY_OPTOUT", "1")
                .env("DOTNET_NOLOGO", "1")
                .env("DOTNET_CLI_HOME", case.join("dotnet-home"))
                .env("NUGET_PACKAGES", case.join("nuget-cache"));
            let output = command.output().unwrap();
            fs::write(
                case.join(format!("{label}.log")),
                [output.stdout.as_slice(), output.stderr.as_slice()].concat(),
            )
            .unwrap();
            fs::write(
                case.join(format!("{label}.command.json")),
                json!({"command":format!("{command:?}"),"status":output.status.code(),"sdk":sdk})
                    .to_string(),
            )
            .unwrap();
            output
        };
        let checked = |dir: &str, label: &str, args: &[&str]| {
            let output = run(dir, label, args);
            assert!(
                output.status.success(),
                "{label}: {}\n{}{}",
                case.display(),
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            output
        };
        assert_eq!(
            String::from_utf8_lossy(&checked(".", "sdk-version", &["--version"]).stdout).trim(),
            sdk
        );
        checked(
            "csharp",
            "restore",
            &["restore", "--configfile", "../NuGet.Config"],
        );
        checked(
            "csharp",
            "pack",
            &["pack", "-c", "Release", "--no-restore", "-o", "../feed"],
        );
        fs::create_dir(case.join("consumer")).unwrap();
        fs::write(case.join("consumer/Consumer.csproj"),format!("<Project Sdk=\"Microsoft.NET.Sdk\"><PropertyGroup><TargetFramework>{framework}</TargetFramework><OutputType>Exe</OutputType><LangVersion>12.0</LangVersion><Nullable>enable</Nullable><ImplicitUsings>enable</ImplicitUsings><TreatWarningsAsErrors>true</TreatWarningsAsErrors></PropertyGroup><ItemGroup><PackageReference Include=\"Positional.Csharp\" Version=\"[1.0.0]\" /></ItemGroup></Project>")).unwrap();
        fs::write(case.join("consumer/Program.cs"), consumer(&plan)).unwrap();
        checked(
            "consumer",
            "consumer-restore",
            &["restore", "--configfile", "../NuGet.Config"],
        );
        checked(
            "consumer",
            "consumer-run",
            &[
                "run",
                "-c",
                "Release",
                "--no-restore",
                "--",
                "../feed/Positional.Csharp.1.0.0.nupkg",
            ],
        );
        fs::create_dir(case.join("negative")).unwrap();
        fs::write(case.join("negative/Negative.csproj"),format!("<Project Sdk=\"Microsoft.NET.Sdk\"><PropertyGroup><TargetFramework>{framework}</TargetFramework><Nullable>enable</Nullable><ImplicitUsings>enable</ImplicitUsings><TreatWarningsAsErrors>true</TreatWarningsAsErrors></PropertyGroup><ItemGroup><PackageReference Include=\"Positional.Csharp\" Version=\"[1.0.0]\" /></ItemGroup></Project>")).unwrap();
        let body = |op: &str| {
            plan.operations()
                .iter()
                .find(|o| o.operation_id == op)
                .unwrap()
                .body
                .as_ref()
                .unwrap()
                .media[0]
                .positional
                .as_ref()
                .unwrap()
        };
        fs::write(case.join("negative/Negative.cs"),format!("using Positional.Csharp; public static class Negative {{ public static object WrongPayload() => new {} {{ Value = \"not bytes\", Headers = new {} {{ XIndex = 1 }} }}; public static object MissingPrefix() => new {}(); public static object CrossBarrier() => new {} {{ Item2 = default }}; public static object MissingHeader() => new {} {{ Value = \"x\" }}; }}",body("sendSequence").items.as_ref().unwrap().wrapper_type.as_ref().unwrap(),body("sendSequence").items.as_ref().unwrap().header_type.as_ref().unwrap(),body("sendSequence").native_type,body("sendBarrier").native_type,body("sendSequence").prefix[0].wrapper_type.as_ref().unwrap())).unwrap();
        checked(
            "negative",
            "negative-restore",
            &["restore", "--configfile", "../NuGet.Config"],
        );
        let negative = run(
            "negative",
            "negative-types",
            &["build", "-c", "Release", "--no-restore"],
        );
        let text = String::from_utf8_lossy(&negative.stdout);
        assert!(
            !negative.status.success()
                && ["CS0029", "CS9035", "CS0117"]
                    .iter()
                    .all(|c| text.contains(c)),
            "{text}"
        );
        checked(
            "csharp/examples",
            "examples-restore",
            &["restore", "--configfile", "../../NuGet.Config"],
        );
        checked(
            "csharp/examples",
            "examples-run",
            &["run", "-c", "Release", "--no-restore"],
        );
        fs::write(case.join("PASS.json"),json!({"sdk":sdk,"framework":framework,"gate":"finite-positional-multipart","result":"passed"}).to_string()).unwrap();
    }
    println!("C# positional evidence: {}", root.display());
}
fn consumer(plan: &SdkPlan) -> String {
    let mut source = include_str!("../src/csharp_sdk/testdata/PositionalConsumer.cs").to_owned();
    for (operation, stem) in [
        ("sendSequence", "SEQUENCE"),
        ("sendOptional", "OPTIONAL"),
        ("sendBarrier", "BARRIER"),
        ("sendEmpty", "EMPTY"),
        ("sendRepeated", "REPEATED"),
        ("sendFallback", "FALLBACK"),
        ("sendDisposition", "DISPOSITION"),
        ("sendTiny", "TINY"),
    ] {
        let op = plan
            .operations()
            .iter()
            .find(|o| o.operation_id == operation)
            .unwrap();
        let parts = op.body.as_ref().unwrap().media[0]
            .positional
            .as_ref()
            .unwrap();
        source = source.replace(&format!("__{stem}__"), &parts.native_type);
        for (i, part) in parts.prefix.iter().enumerate() {
            source = source.replace(
                &format!("__{stem}_PART{}__", i + 1),
                part.wrapper_type.as_ref().unwrap(),
            );
            if let Some(h) = &part.header_type {
                source = source.replace(&format!("__{stem}_HEADER{}__", i + 1), h);
            }
        }
        if let Some(items) = &parts.items {
            source = source.replace(
                &format!("__{stem}_ITEM__"),
                items.wrapper_type.as_ref().unwrap(),
            );
            if let Some(h) = &items.header_type {
                source = source.replace(&format!("__{stem}_ITEM_HEADERS__"), h);
            }
        }
        assert!(matches!(
            op.body.as_ref().unwrap().media[0].wire.representation(),
            Representation::Multipart {
                multipart: wire::MultipartPlan::Positional { .. }
            }
        ));
    }
    source
}
