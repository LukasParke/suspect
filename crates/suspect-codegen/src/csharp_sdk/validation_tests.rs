//! Source-driven scoped validation and installed native SDK adoption witnesses.
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
    sync::Arc,
};
use suspect_ir::contract::Contract;
use suspect_ref::WorkspaceBuilder;
use suspect_schema::{Config, OwnedCompiler};
use suspect_source::Uri;

const TIERS: [(&str, &str); 2] = [("8.0.424", "net8.0"), ("10.0.400", "net10.0")];
const VECTORS: &str =
    include_str!("../../../suspect-schema/tests/fixtures/owned-applicators-v2.json");

fn directory(prefix: &str) -> PathBuf {
    let parent = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/sdk-csharp-schema-v2");
    fs::create_dir_all(&parent).unwrap();
    tempfile::Builder::new()
        .prefix(prefix)
        .tempdir_in(fs::canonicalize(parent).unwrap())
        .unwrap()
        .keep()
}
fn load(path: &Path) -> Arc<Contract> {
    let workspace = Arc::new(
        WorkspaceBuilder::new()
            .root(path.parent().unwrap())
            .build()
            .unwrap(),
    );
    Arc::new(Contract::from_workspace(&workspace, &Uri::from_path(path).unwrap()).unwrap())
}
pub(super) fn write(path: &Path, value: impl AsRef<[u8]>) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, value).unwrap();
}

pub(super) struct Native {
    pub(super) root: PathBuf,
    sdk: String,
    framework: String,
}
impl Native {
    pub(super) fn new(root: &Path, sdk: &str, framework: &str) -> Self {
        let root = root.join(sdk);
        write(
            &root.join("global.json"),
            json!({"sdk":{"version":sdk,"rollForward":"disable"}}).to_string(),
        );
        write(
            &root.join("NuGet.Config"),
            "<configuration><packageSources><clear/><add key=\"local\" value=\"feed\"/></packageSources></configuration>",
        );
        fs::create_dir_all(root.join("feed")).unwrap();
        let native = Self {
            root,
            sdk: sdk.into(),
            framework: framework.into(),
        };
        let version = native.checked(".", "sdk-version", &["--version"]);
        assert_eq!(String::from_utf8_lossy(&version.stdout).trim(), sdk);
        native
    }
    pub(super) fn run(&self, dir: &str, label: &str, args: &[&str]) -> Output {
        let executable = std::env::var_os("SUSPECT_DOTNET_BIN")
            .unwrap_or_else(|| "/Users/luke/.local/share/mise/dotnet-root/dotnet".into());
        let mut command = Command::new(&executable);
        command
            .args(args)
            .current_dir(self.root.join(dir))
            .env("DOTNET_CLI_TELEMETRY_OPTOUT", "1")
            .env("DOTNET_NOLOGO", "1")
            .env("DOTNET_CLI_HOME", self.root.join("dotnet-home"))
            .env("NUGET_PACKAGES", self.root.join("nuget-cache"));
        // Record the exact selected executable before launch, even if launch fails.
        let record = self.root.join(format!("{label}.command.json"));
        assert!(
            !record.exists(),
            "native command labels must not overwrite attempts"
        );
        write(
            &record,
            json!({"command":format!("{command:?}"),"sdk":self.sdk,"status":"started"}).to_string(),
        );
        let output = command.output().unwrap();
        write(
            &self.root.join(format!("{label}.log")),
            [output.stdout.as_slice(), output.stderr.as_slice()].concat(),
        );
        write(&record,json!({"command":format!("{command:?}"),"sdk":self.sdk,"status":output.status.code(),"selector":executable}).to_string());
        output
    }
    pub(super) fn checked(&self, dir: &str, label: &str, args: &[&str]) -> Output {
        let output = self.run(dir, label, args);
        assert!(
            output.status.success(),
            "{label}: {}\n{}{}",
            self.root.display(),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        output
    }
    pub(super) fn project(&self, dir: &str, package: Option<&str>, executable: bool) {
        let dependency = package.map_or(String::new(), |name| {
            format!(
                "<ItemGroup><PackageReference Include=\"{name}\" Version=\"[1.0.0]\" /></ItemGroup>"
            )
        });
        write(
            &self.root.join(dir).join("Consumer.csproj"),
            format!(
                "<Project Sdk=\"Microsoft.NET.Sdk\"><PropertyGroup><TargetFramework>{}</TargetFramework><OutputType>{}</OutputType><LangVersion>12.0</LangVersion><Nullable>enable</Nullable><ImplicitUsings>enable</ImplicitUsings><TreatWarningsAsErrors>true</TreatWarningsAsErrors></PropertyGroup>{dependency}</Project>",
                self.framework,
                if executable { "Exe" } else { "Library" }
            ),
        );
    }
    pub(super) fn finish(&self, gate: &str) {
        write(
            &self.root.join("PASS.json"),
            json!({"sdk":self.sdk,"framework":self.framework,"gate":gate,"result":"passed"})
                .to_string(),
        );
    }
}

// Independent instance pointers supplement the maintained source/outcome oracle.
// They are not copied from the shared evaluator's output.
fn instance_pointer(id: &str) -> &'static str {
    match id {
        "contains-zero-does-not-mark-unmatched" | "contains-exact-integrality" => "/0",
        "contains-failure-after-exceeded-maximum" => "/1",
        "pattern-overlap-rejects" | "named-and-pattern-both-apply" => "/x",
        "property-names-checks-key-not-value" => "/long",
        "property-names-does-not-annotate-values" => "/ok",
        "failed-anyof-branch-does-not-leak"
        | "allof-cousins-have-independent-scopes"
        | "not-discards-annotations"
        | "required-is-not-an-evaluation" => "/a",
        "nested-members-do-not-mark-parent" => "/inner",
        "prefix-and-contains-leave-unmatched-item" => "/2",
        _ => "",
    }
}

fn compiled_vectors(root: &Path) -> Value {
    let fixture: Value = serde_json::from_str(VECTORS).unwrap();
    assert_eq!(fixture["cases"].as_array().unwrap().len(), 32);
    compile_cases(root, &fixture)
}
fn compile_cases(root: &Path, fixture: &Value) -> Value {
    let mut result = Vec::new();
    for case in fixture["cases"].as_array().unwrap() {
        let id = case["id"].as_str().unwrap();
        let source = root.join("sources").join(format!("{id}.openapi.json"));
        let schema: Value = serde_json::from_str(case["schemaJson"].as_str().unwrap()).unwrap();
        let document = json!({"openapi":"3.2.0","info":{"title":id,"version":"1"},"paths":{},"components":{"schemas":{"Root":schema}}});
        write(&source, serde_json::to_vec_pretty(&document).unwrap());
        let contract = load(&source);
        let mut config = Config {
            max_depth: 128,
            ..Default::default()
        };
        if let Some(limits) = case["limits"].as_object() {
            for (key, value) in limits {
                let n = value.as_u64().unwrap() as usize;
                match key.as_str() {
                    "maxNumberBytes" => config.max_number_bytes = n,
                    "maxEvaluationSteps" => config.max_evaluation_steps = n,
                    "maxEqualitySteps" => config.max_equality_steps = n,
                    "maxDepth" => config.max_depth = n,
                    "maxErrors" => config.max_errors = n,
                    _ => panic!("unhandled source fixture limit: {key}"),
                }
            }
        }
        let compiled = OwnedCompiler::new(config)
            .compile_v2(contract.clone(), contract.schema_roots())
            .unwrap();
        let program = compiled.program();
        assert_eq!(
            program.version,
            suspect_schema::OwnedProgram::V2_VERSION,
            "the scoped fixture must contain an executable v2 instruction: {id}"
        );
        program.check().unwrap();
        result.push(json!({"id":id,"program":program,"instanceJson":case["instanceJson"],"expected":case["expected"],
            "source":case["source"],"document":Uri::from_path(&source).unwrap().to_string(),"instancePointer":case["instancePointer"].as_str().unwrap_or_else(||instance_pointer(id))}));
    }
    json!({"format":"suspect-csharp-scoped-source-vectors-v1","oracle":fixture["oracle"],"expectedCount":result.len(),"cases":result})
}

#[test]
#[ignore = "source-driven 32-case native v2 runtime proof on exact .NET 8/10 tiers"]
fn native_scoped_source_vectors() {
    let root = directory("vectors-");
    let vectors = compiled_vectors(&root);
    run_vectors(&root, &vectors);
}

#[test]
fn source_scoped_operations_retain_typed_fields_and_checked_extras() {
    let root = directory("planning-");
    let path = root.join("source.openapi.json");
    write(&path, include_str!("testdata/scoped.openapi.json"));
    let contract = load(&path);
    let selected = contract
        .operations()
        .map(|op| op.source().clone())
        .collect::<Vec<_>>();
    let plan = super::plan_sdk(
        contract,
        &selected,
        super::SdkConfig {
            name: "Scoped.Csharp".into(),
            version: "1.0.0".into(),
            namespace: "Scoped.Csharp".into(),
        },
    )
    .unwrap();
    assert_eq!(
        plan.program().version,
        suspect_schema::OwnedProgram::V2_VERSION
    );
    let invoice = plan
        .models()
        .declarations()
        .iter()
        .find(|(key, _)| key.0.pointer() == "/components/schemas/Invoice")
        .unwrap();
    let super::models::CsDecl::Record { fields, extras } = invoice.1 else {
        panic!("declared object fields must stay typed")
    };
    assert_eq!(fields.len(), 7);
    assert!(matches!(
        fields.iter().find(|f| f.wire == "amount").unwrap().ty,
        super::models::CsType::Number
    ));
    assert!(
        matches!(extras, Some(super::models::CsType::Json)),
        "patterned extras survive additionalProperties:false"
    );
    assert_eq!(plan.models().native_type(&invoice.0.0), "Invoice");
    assert_eq!(plan.examples().operations().len(), 8);
    assert!(
        plan.examples().diagnostics().is_empty(),
        "{:?}",
        plan.examples().diagnostics()
    );
    assert!(
        plan.render()
            .unwrap()
            .iter()
            .any(|f| f.path == "csharp/src/ScopedValidationRuntime.cs")
    );
}

#[test]
fn inactive_scoped_compiler_keeps_v1_package_bytes() {
    let root = directory("v1-bytes-");
    let source = root.join("source.openapi.json");
    let document = json!({"openapi":"3.2.0","info":{"title":"Frozen base closure","version":"1"},"servers":[{"url":"https://base.example.test"}],"components":{"schemas":{"Root":{
        "type":"object","required":["id"],"properties":{"id":{"type":"string","pattern":"^[A-Za-z]+$"},"count":{"type":"integer","minimum":1,"multipleOf":1},"note":{"type":["string","null"]}},"additionalProperties":{"type":"number"},"examples":[{"id":"base","count":1,"note":null}]}}},
        "paths":{"/base":{"post":{"operationId":"sendBase","requestBody":{"required":true,"content":{"application/json":{"schema":{"$ref":"#/components/schemas/Root"}}}},"responses":{"200":{"content":{"application/json":{"schema":{"$ref":"#/components/schemas/Root"}}}}}}}}});
    write(&source, serde_json::to_vec_pretty(&document).unwrap());
    let contract = load(&source);
    let selected = contract
        .operations()
        .map(|op| op.source().clone())
        .collect::<Vec<_>>();
    let plan = |scoped| {
        super::protocol::plan_with_validation(
            contract.clone(),
            &selected,
            Default::default(),
            Default::default(),
            scoped,
        )
        .unwrap()
    };
    let old = plan(false);
    let new = plan(true);
    assert_eq!(
        old.program().version,
        suspect_schema::OwnedProgram::V1_VERSION
    );
    assert_eq!(
        serde_json::to_vec(old.program()).unwrap(),
        serde_json::to_vec(new.program()).unwrap()
    );
    let before = old.render().unwrap();
    let after = new.render().unwrap();
    assert_eq!(before.len(), after.len());
    for (before, after) in before.iter().zip(&after) {
        assert_eq!(before.path, after.path);
        assert_eq!(
            before.content, after.content,
            "unchanged v1 artifact {}",
            before.path
        );
    }
    assert!(
        after
            .iter()
            .any(|file| file.path == "csharp/src/ValidationRuntime.cs")
    );
    assert!(
        !after
            .iter()
            .any(|file| file.path.ends_with("ScopedValidationRuntime.cs")
                || file.path.ends_with("ValidationProgramGuard.cs"))
    );
    write(
        &root.join("PASS.json"),
        json!({"program":"byte-identical-v1","packageFiles":after.len(),"result":"passed"})
            .to_string(),
    );
}

#[test]
#[ignore = "focused v2 scope, exact budgets, native program admission and ownership controls"]
fn native_scoped_limits_and_admission() {
    let root = directory("controls-");
    let fixture: Value =
        serde_json::from_str(include_str!("testdata/scoped-controls.json")).unwrap();
    assert_eq!(fixture["cases"].as_array().unwrap().len(), 28);
    let mut vectors = compile_cases(&root, &fixture);
    vectors["admissionPrograms"] = compiled_vectors(&root.join("admission"))["cases"].clone();
    run_vectors(&root, &vectors);
}
fn run_vectors(root: &Path, vectors: &Value) {
    write(
        &root.join("source-programs.json"),
        serde_json::to_vec_pretty(&vectors).unwrap(),
    );
    for (sdk, framework) in TIERS {
        let native = Native::new(root, sdk, framework);
        native.project("validation", None, true);
        for (name, asset) in [
            ("JsonRuntime.cs", include_str!("JsonRuntime.cs")),
            (
                "ScopedValidationRuntime.cs",
                include_str!("ScopedValidationRuntime.cs"),
            ),
            (
                "ValidationProgramGuard.cs",
                include_str!("ValidationProgramGuard.cs"),
            ),
        ] {
            write(
                &native.root.join("validation").join(name),
                asset.replace("__NAMESPACE__", "Scoped.Csharp"),
            );
        }
        write(
            &native.root.join("validation/Program.cs"),
            include_str!("testdata/ScopedValidationConsumer.cs"),
        );
        write(
            &native.root.join("validation/Controls.cs"),
            include_str!("testdata/ScopedValidationControls.cs"),
        );
        write(
            &native.root.join("validation/vectors.json"),
            serde_json::to_vec_pretty(&vectors).unwrap(),
        );
        native.checked(
            "validation",
            "restore",
            &["restore", "--configfile", "../NuGet.Config"],
        );
        native.checked(
            "validation",
            "source-vectors",
            &["run", "-c", "Release", "--no-restore"],
        );
        native.finish("source-driven-scoped-v2");
    }
    println!("C# scoped runtime evidence: {}", root.display());
}

fn scoped_plan(root: &Path) -> super::SdkPlan {
    let path = root.join("source.openapi.json");
    write(&path, include_str!("testdata/scoped.openapi.json"));
    let contract = load(&path);
    let selected = contract
        .operations()
        .map(|op| op.source().clone())
        .collect::<Vec<_>>();
    super::plan_sdk(
        contract,
        &selected,
        super::SdkConfig {
            name: "Scoped.Csharp".into(),
            version: "1.0.0".into(),
            namespace: "Scoped.Csharp".into(),
        },
    )
    .unwrap()
}
fn native_name<'a>(plan: &'a super::SdkPlan, pointer: &str) -> &'a str {
    plan.models()
        .names()
        .iter()
        .find(|(key, _)| key.0.pointer() == pointer)
        .unwrap()
        .1
}
fn sdk_consumer(plan: &super::SdkPlan) -> String {
    let mut source = include_str!("testdata/ScopedSdkConsumer.cs").to_owned();
    for (token, pointer) in [
        ("INVOICE", "Invoice"),
        ("INVOICE_KIND", "Invoice/properties/kind"),
        ("SETTINGS", "Settings"),
        ("SETTINGS_MODE", "Settings/properties/mode"),
        ("LABELS", "Labels"),
        ("NODE", "Node"),
        ("MAYBE", "Maybe"),
    ] {
        source = source.replace(
            &format!("__{token}__"),
            native_name(plan, &format!("/components/schemas/{pointer}")),
        );
    }
    for (token, schema) in [
        ("INVOICE", "Invoice"),
        ("SETTINGS", "Settings"),
        ("SEQUENCE", "Sequence"),
        ("LABELS", "Labels"),
        ("VALUE", "ScopedValue"),
        ("NODE", "Node"),
        ("MAYBE", "Maybe"),
        ("REFERENCE", "ScopedReference"),
    ] {
        source = source.replace(
            &format!("__{token}_CODEC__"),
            native_name(plan, &format!("/components/schemas/{schema}")),
        );
    }
    let error = &plan
        .operations()
        .iter()
        .find(|op| op.operation_id == "createInvoice")
        .unwrap()
        .responses
        .iter()
        .find(|r| r.wire.status_key() == "422")
        .unwrap()
        .error_type_name;
    source.replace("__INVOICE_ERROR__", error)
}

#[test]
#[ignore = "installed NuGet v2 models, codec mutation, SDK operations, CLR types and executable native docs"]
fn native_scoped_sdk_packages() {
    let root = directory("sdk-");
    let plan = scoped_plan(&root);
    assert_eq!(
        plan.program().version,
        suspect_schema::OwnedProgram::V2_VERSION
    );
    assert!(
        plan.examples().diagnostics().is_empty(),
        "{:?}",
        plan.examples().diagnostics()
    );
    assert_eq!(
        plan.examples()
            .operations()
            .iter()
            .map(|op| op.entries.len())
            .sum::<usize>(),
        30
    );
    for operation in plan.examples().operations() {
        for entry in &operation.entries {
            assert_eq!(
                plan.contract()
                    .source(entry.declared_source.as_ref().unwrap()),
                Some(&entry.value)
            );
        }
    }
    let files = plan.render().unwrap();
    for (sdk, framework) in TIERS {
        let native = Native::new(&root, sdk, framework);
        crate::write_files(&files, &native.root).unwrap();
        native.checked(
            "csharp",
            "package-restore",
            &["restore", "--configfile", "../NuGet.Config"],
        );
        native.checked(
            "csharp",
            "package-pack",
            &["pack", "-c", "Release", "--no-restore", "-o", "../feed"],
        );
        native.project("consumer", Some("Scoped.Csharp"), true);
        write(
            &native.root.join("consumer/Program.cs"),
            sdk_consumer(&plan),
        );
        native.checked(
            "consumer",
            "consumer-restore",
            &["restore", "--configfile", "../NuGet.Config"],
        );
        native.checked(
            "consumer",
            "consumer-run",
            &[
                "run",
                "-c",
                "Release",
                "--no-restore",
                "--",
                "../feed/Scoped.Csharp.1.0.0.nupkg",
            ],
        );
        native.project("negative", Some("Scoped.Csharp"), false);
        let invoice = native_name(&plan, "/components/schemas/Invoice");
        let kind = native_name(&plan, "/components/schemas/Invoice/properties/kind");
        let labels = native_name(&plan, "/components/schemas/Labels");
        write(
            &native.root.join("negative/Negative.cs"),
            format!(
                "using Scoped.Csharp; using System.Text.Json; public static class Negative {{ public static object MissingFields() => new {invoice}(); public static object WrongNumber() => new {invoice} {{ Id=\"id\",Kind={kind}.Business,Amount=0.1 }}; public static object WrongExtra() => new {labels} {{ Title=\"x\",Extra=new Dictionary<string,string>() }}; public static object WrongSequence() => new ReplaceSequenceInput {{ Body=new List<string>() }}; public static object WrongCarrier() => new EvaluateValueInput {{ Body=\"{{}}\" }}; }}"
            ),
        );
        native.checked(
            "negative",
            "negative-restore",
            &["restore", "--configfile", "../NuGet.Config"],
        );
        let negative = native.run(
            "negative",
            "negative-build",
            &["build", "-c", "Release", "--no-restore"],
        );
        let text = String::from_utf8_lossy(&negative.stdout);
        assert!(
            !negative.status.success()
                && ["CS9035", "CS0029"].iter().all(|code| text.contains(code)),
            "{text}"
        );
        for phrase in [
            "double",
            "JsonNumber",
            "Dictionary<string, string>",
            "List<string>",
            "JsonElement",
        ] {
            assert!(
                text.contains(phrase),
                "missing negative type diagnostic {phrase}: {text}"
            );
        }
        native.checked(
            "csharp/examples",
            "examples-restore",
            &["restore", "--configfile", "../../NuGet.Config"],
        );
        native.checked(
            "csharp/examples",
            "examples-run",
            &["run", "-c", "Release", "--no-restore"],
        );
        native.finish("installed-scoped-v2-sdk");
    }
    println!("C# scoped SDK evidence: {}", root.display());
}

#[test]
fn canonical_scoped_capture_preserves_native_codec_obligations() {
    use crate::{
        backend::{Backend, TargetConfig},
        compatibility,
    };
    let root = directory("capture-");
    let before_path = root.join("before/source.openapi.json");
    write(&before_path, include_str!("testdata/scoped.openapi.json"));
    let contract = load(&before_path);
    let target = TargetConfig {
        backend: Backend::CsharpHttp,
        package_name: "Scoped.Csharp".into(),
        package_version: "1.0.0".into(),
        import_name: Some("Scoped.Csharp".into()),
    };
    let before = compatibility::snapshot(contract, &[], std::slice::from_ref(&target)).unwrap();
    assert_eq!(
        before.native[0].status,
        compatibility::PlanStatus::Planned,
        "{:?}",
        before.native[0].findings
    );
    let invoice = before.native[0]
        .models
        .iter()
        .find(|m| m.source.pointer == "/components/schemas/Invoice" && m.role == "model")
        .unwrap()
        .descriptor
        .as_ref()
        .unwrap();
    assert_eq!(
        invoice["sourceValidation"]["profile"],
        suspect_schema::OwnedProgram::V2_PROFILE
    );
    assert_eq!(
        invoice["sourceValidation"]["encode"],
        "current-mutable-value"
    );
    assert_eq!(
        invoice["extraValidation"],
        "all-matching-patterns-and-unmatched-additional-properties"
    );
    let extra = invoice["fields"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["name"] == "Extra")
        .unwrap();
    assert_eq!(
        extra["type"]["arguments"][1]["name"],
        "System.Text.Json.JsonElement"
    );
    let carrier = before.native[0]
        .models
        .iter()
        .find(|m| m.source.pointer == "/components/schemas/ScopedValue" && m.role == "erased-alias")
        .unwrap()
        .descriptor
        .as_ref()
        .unwrap();
    assert_eq!(carrier["sourceValidation"]["jsonCarrier"], true);
    assert_eq!(carrier["type"]["name"], "System.Text.Json.JsonElement");
    let mut changed: Value =
        serde_json::from_str(include_str!("testdata/scoped.openapi.json")).unwrap();
    changed["components"]["schemas"]["Labels"]
        .as_object_mut()
        .unwrap()
        .remove("patternProperties");
    let after_path = root.join("after/source.openapi.json");
    write(&after_path, serde_json::to_vec_pretty(&changed).unwrap());
    let after = compatibility::snapshot(load(&after_path), &[], &[target]).unwrap();
    assert_eq!(
        after.native[0].status,
        compatibility::PlanStatus::Planned,
        "{:?}",
        after.native[0].findings
    );
    let labels = after.native[0]
        .models
        .iter()
        .find(|m| m.source.pointer == "/components/schemas/Labels" && m.role == "model")
        .unwrap()
        .descriptor
        .as_ref()
        .unwrap();
    let extra = labels["fields"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["name"] == "Extra")
        .unwrap();
    assert_eq!(extra["type"]["arguments"][1]["name"], "string");
    let report = compatibility::compare_snapshots(&before, &after);
    assert!(
        report.native[0]
            .changes
            .iter()
            .any(|c| c.code == "native-model-shape-changed" && c.subject.contains("Labels"))
    );
    write(
        &root.join("before.json"),
        serde_json::to_vec_pretty(&json!({"metadata":before.metadata,"native":before.native}))
            .unwrap(),
    );
    write(
        &root.join("after.json"),
        serde_json::to_vec_pretty(&json!({"metadata":after.metadata,"native":after.native}))
            .unwrap(),
    );
    write(
        &root.join("comparison.json"),
        serde_json::to_vec_pretty(&report).unwrap(),
    );
    write(
        &root.join("PASS.json"),
        json!({"result":"passed","gate":"canonical-v2-capture"}).to_string(),
    );
}

#[test]
fn beyond_static_v2_remains_a_located_refusal() {
    let root = directory("refusals-");
    for (name, schema, keyword) in [
        (
            "dynamic",
            json!({"type":"object","$dynamicAnchor":"node","properties":{"next":{"$dynamicRef":"#node"}}}),
            "$dynamic",
        ),
        (
            "pattern",
            json!({"type":"object","patternProperties":{"(?=x)":{"type":"integer"}}}),
            "patternProperties",
        ),
    ] {
        let document = json!({"openapi":"3.2.0","info":{"title":name,"version":"1"},"servers":[{"url":"https://boundary.example.test"}],"paths":{"/boundary":{"post":{"operationId":"checkBoundary","requestBody":{"required":true,"content":{"application/json":{"schema":schema}}},"responses":{"204":{"description":"done"}}}}}});
        let source = root.join(format!("{name}.openapi.json"));
        write(&source, serde_json::to_vec_pretty(&document).unwrap());
        let contract = load(&source);
        let selected = contract
            .operations()
            .map(|op| op.source().clone())
            .collect::<Vec<_>>();
        let capabilities = super::protocol::capabilities();
        let static_capabilities = crate::http_protocol::Capabilities::for_adapter(
            "csharp-static-profile-fence",
            capabilities.enabled().iter().copied().filter(|c| {
                !matches!(
                    c,
                    crate::http_protocol::Capability::SchemaResources
                        | crate::http_protocol::Capability::DynamicSchemaReferences
                )
            }),
        )
        .with_limits(capabilities.limits());
        let errors = super::protocol::plan_with_capabilities(
            contract.clone(),
            &selected,
            Default::default(),
            Default::default(),
            true,
            static_capabilities,
        )
        .unwrap_err();
        assert!(
            errors
                .iter()
                .any(|error| error.source.pointer().contains(keyword)
                    && error.source.document() == contract.entry()
                    && error.at.end > error.at.start),
            "{errors:?}"
        );
    }
}
