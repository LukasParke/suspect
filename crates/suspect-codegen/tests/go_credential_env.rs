//! Explicit runtime environment policy through retained Go plans and consumers.
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    process::Command,
    sync::Arc,
};
use suspect_codegen::{OutFile, go_http};
use suspect_ir::contract::Contract;
use suspect_ref::{DocumentProvider, ProvidedDocument, WorkspaceBuilder};
use suspect_source::Uri;

fn supplied(uri: &str, document: Value) -> Arc<Contract> {
    let uri = Uri::parse(uri).unwrap();
    let provider = Arc::new(
        DocumentProvider::new([ProvidedDocument::new(
            uri.clone(),
            uri.clone(),
            document.to_string().into_bytes(),
        )
        .unwrap()])
        .unwrap(),
    );
    let workspace = Arc::new(
        WorkspaceBuilder::new()
            .allowed_documents(provider.logical_uris())
            .document_provider(provider)
            .build()
            .unwrap(),
    );
    Arc::new(Contract::from_workspace(&workspace, &uri).unwrap())
}
fn document() -> Value {
    let mut paths = json!({});
    for (name, security) in [
        ("protected", json!([{"token":[]}])),
        ("anonymous", json!([])),
        ("alternative", json!([{"token":[]},{"headerKey":[]}])),
        (
            "conjunctive",
            json!([{"token":[],"headerKey":[],"queryKey":[],"cookieKey":[]}]),
        ),
        ("optional", json!([{}, {"token":[]}])),
    ] {
        paths[format!("/{name}")] = json!({"get":{"operationId":name,"security":security,"responses":{"200":{"content":{"application/json":{"schema":{"type":"string"}}}}}}});
    }
    json!({"openapi":"3.2.0","info":{"title":"Go credential environment","version":"1"},"servers":[{"url":"https://source.example.test/api/v1"}],"paths":paths,"components":{"securitySchemes":{
        "token":{"type":"http","scheme":"bearer"},"headerKey":{"type":"apiKey","in":"header","name":"X-API-Key"},"queryKey":{"type":"apiKey","in":"query","name":"access_key"},"cookieKey":{"type":"apiKey","in":"cookie","name":"session_key"}
    }}})
}
fn plan(c: Arc<Contract>, config: go_http::HttpConfig) -> go_http::HttpPlan {
    let selected = c
        .operations()
        .map(|op| op.source().clone())
        .collect::<Vec<_>>();
    go_http::plan_http(c, &selected, config).unwrap()
}
fn inventory(files: &[OutFile]) -> Value {
    json!(files.iter().map(|file|(file.path.clone(),json!({"bytes":file.content.len(),"sha256":format!("{:x}",Sha256::digest(file.content.as_bytes()))}))).collect::<BTreeMap<_,_>>())
}
fn synthetic_files() -> Vec<OutFile> {
    plan(
        supplied(
            "https://fixtures.example.test/credential-env.json",
            document(),
        ),
        Default::default(),
    )
    .render()
}
fn terraform_files() -> Vec<OutFile> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/terraform-v1");
    let provider = Arc::new(
        DocumentProvider::new(["openapi.json", "schemas.json"].iter().map(|name| {
            let uri =
                Uri::parse(&format!("https://fixtures.example.test/terraform/{name}")).unwrap();
            ProvidedDocument::new(uri.clone(), uri, std::fs::read(root.join(name)).unwrap())
                .unwrap()
        }))
        .unwrap(),
    );
    let workspace = Arc::new(
        WorkspaceBuilder::new()
            .allowed_documents(provider.logical_uris())
            .document_provider(provider)
            .build()
            .unwrap(),
    );
    let c = Arc::new(
        Contract::from_workspace(
            &workspace,
            &Uri::parse("https://fixtures.example.test/terraform/openapi.json").unwrap(),
        )
        .unwrap(),
    );
    let mapping = suspect_codegen::terraform::parse_mapping(
        &std::fs::read_to_string(root.join("mapping.json")).unwrap(),
    )
    .unwrap();
    let target = serde_json::from_slice(&std::fs::read(root.join("target.json")).unwrap()).unwrap();
    let provider = suspect_codegen::terraform::plan_provider(c, mapping, target).unwrap();
    suspect_codegen::terraform::emit_provider(&provider)
}

fn inventory_hash(value: &Value) -> String {
    format!("{:x}", Sha256::digest(serde_json::to_vec(value).unwrap()))
}
#[test]
fn no_policy_retains_pre_change_sdk_and_terraform_bytes() {
    // Captured after the ua/v1 attribution emission (one added go/attribution.go
    // file). This binds every path, length and SHA-256, without depending on a
    // target/ witness at runtime.
    let synthetic = inventory(&synthetic_files());
    assert_eq!(synthetic.as_object().unwrap().len(), 35);
    assert_eq!(
        inventory_hash(&synthetic),
        "74ffce12340c324c4d451738c30dde409e289ef04427a599b7f7eb8833e3c8ca"
    );
    let terraform = inventory(&terraform_files());
    assert_eq!(terraform.as_object().unwrap().len(), 49);
    assert_eq!(
        inventory_hash(&terraform),
        "3ede744f1d03a93d27c04b230c69c291a70889dec4206a79be4ddfc9b7a7e81f"
    );
    let sdk = json!(
        terraform
            .as_object()
            .unwrap()
            .iter()
            .filter(|(path, _)| path.starts_with("go/"))
            .map(|(path, value)| (path.clone(), value.clone()))
            .collect::<BTreeMap<_, _>>()
    );
    assert_eq!(sdk.as_object().unwrap().len(), 35);
    assert_eq!(
        inventory_hash(&sdk),
        "3de9a714710ac82e7a08cb8d29f9b4a47ec7d818af266e720842afb816e6e282"
    );
}

fn policy() -> suspect_codegen::credential_env::CredentialEnv {
    suspect_codegen::credential_env::CredentialEnv::v1(BTreeMap::from([
        ("token".into(), "SUSPECT_GO_ENV_TOKEN".into()),
        ("headerKey".into(), "SUSPECT_GO_ENV_HEADER".into()),
        ("queryKey".into(), "SUSPECT_GO_ENV_QUERY".into()),
        ("cookieKey".into(), "SUSPECT_GO_ENV_COOKIE".into()),
    ]))
}
fn env_plan() -> go_http::HttpPlan {
    plan(
        supplied(
            "https://fixtures.example.test/credential-env.json",
            document(),
        ),
        go_http::HttpConfig {
            credential_env: Some(policy()),
            ..Default::default()
        },
    )
}

#[test]
fn policy_binds_actual_sources_and_allocates_factory_without_changing_explicit_api() {
    let plan = env_plan();
    let bound = plan.credential_env().unwrap();
    assert_eq!(bound.bindings().len(), 4);
    assert_eq!(plan.credential_env_factory(), Some("NewClientFromEnv"));
    let token = bound
        .bindings()
        .iter()
        .find(|b| b.name() == "token")
        .unwrap();
    assert_eq!(token.variable(), "SUSPECT_GO_ENV_TOKEN");
    assert_eq!(
        token.kind(),
        suspect_codegen::credential_env::CredentialEnvKind::Bearer
    );
    assert_eq!(
        token.scheme().use_site().source().pointer(),
        "/components/securitySchemes/token"
    );
    let files = plan.render();
    let metadata: Value = serde_json::from_str(
        &files
            .iter()
            .find(|f| f.path == "go/credential-env.json")
            .unwrap()
            .content,
    )
    .unwrap();
    assert_eq!(
        metadata["semantics"],
        serde_json::to_value(bound.semantic_descriptor()).unwrap()
    );
    assert_eq!(metadata["factory"], "NewClientFromEnv");
    let explicit = &files
        .iter()
        .find(|f| f.path == "go/http_runtime.go")
        .unwrap()
        .content;
    assert_eq!(
        explicit,
        &synthetic_files()
            .iter()
            .find(|f| f.path == "go/http_runtime.go")
            .unwrap()
            .content
    );
    let mut collision = document();
    collision["components"]["schemas"]["NewClientFromEnv"] = json!({"type":"string"});
    collision["paths"]["/anonymous"]["get"]["responses"]["200"]["content"]["application/json"]["schema"] =
        json!({"$ref":"#/components/schemas/NewClientFromEnv"});
    let collision = self::plan(
        supplied("https://fixtures.example.test/collision.json", collision),
        go_http::HttpConfig {
            credential_env: Some(policy()),
            ..Default::default()
        },
    );
    assert_eq!(
        collision.credential_env_factory(),
        Some("NewClientFromEnv2")
    );
}

#[test]
fn unsupported_env_attachment_is_refused_by_shared_binder_before_artifacts() {
    let mut value = document();
    value["components"]["securitySchemes"]["basic"] = json!({"type":"http","scheme":"basic"});
    value["paths"]["/protected"]["get"]["security"] = json!([{"basic":[]}]);
    let c = supplied("https://fixtures.example.test/unsupported.json", value);
    let selected = c
        .operations()
        .map(|o| o.source().clone())
        .collect::<Vec<_>>();
    let config = go_http::HttpConfig {
        credential_env: Some(suspect_codegen::credential_env::CredentialEnv::v1(
            BTreeMap::from([("basic".into(), "BASIC_ENV".into())]),
        )),
        ..Default::default()
    };
    let errors = go_http::plan_http(c, &selected, config).unwrap_err();
    assert!(errors.iter().any(|e| e.code == "sdk-credential-env-kind"
        && e.source.pointer() == "/components/securitySchemes/basic"
        && !e.at.is_empty()));
}

fn tools() -> Vec<String> {
    std::env::var("SUSPECT_GO_TOOLCHAIN")
        .map(|tool| vec![tool])
        .unwrap_or_else(|_| vec!["go1.23.12".into(), "go1.27.1".into()])
}
fn native_command(root: &Path, label: &str, command: &mut Command) -> std::process::Output {
    let output = command.output().unwrap();
    let receipt = json!({
        "program":command.get_program(), "args":command.get_args().collect::<Vec<_>>(),
        "directory":command.get_current_dir(), "exit":output.status.code(),
        "stdout":String::from_utf8_lossy(&output.stdout), "stderr":String::from_utf8_lossy(&output.stderr),
    });
    std::fs::write(
        root.join("receipts").join(format!("{label}.json")),
        serde_json::to_vec_pretty(&receipt).unwrap(),
    )
    .unwrap();
    output
}

fn native(
    case: &str,
    module: &str,
    plan: &go_http::HttpPlan,
    source: &str,
    negative: &[(&str, &str)],
) {
    // Optional immutable outputs are evidence only. Every run starts from the
    // original input documents and this maintained public consumer source.
    let evidence = std::env::var_os("SUSPECT_GO_CREDENTIAL_ENV_EVIDENCE");
    let root = if let Some(base) = &evidence {
        let base = PathBuf::from(base);
        std::fs::create_dir_all(&base).unwrap();
        let path = base.join(case);
        std::fs::create_dir(&path)
            .expect("use a fresh evidence directory; preserve prior receipts");
        path.canonicalize().unwrap()
    } else {
        tempfile::Builder::new()
            .prefix("suspect-go-credential-env-")
            .tempdir()
            .unwrap()
            .keep()
    };
    std::fs::create_dir(root.join("receipts")).unwrap();
    let package = go_http::PackageConfig {
        module_path: module.into(),
        version: "0.1.0".into(),
        ..Default::default()
    };
    let files = go_http::emit_http(plan, &package).unwrap();
    suspect_codegen::write_files(&files, &root).unwrap();
    let consumer = root.join("consumer");
    std::fs::create_dir(&consumer).unwrap();
    let module_file = format!(
        "module example.com/credential-env-consumer\n\ngo 1.23.0\nrequire {module} v0.1.0\nreplace {module} => ../go\n"
    );
    let source = source.replace("example.com/credential-env-sdk", module);
    std::fs::write(consumer.join("go.mod"), &module_file).unwrap();
    std::fs::write(consumer.join("environment_test.go"), &source).unwrap();
    let inputs = json!({
        "case":case, "module":module, "factory":plan.credential_env_factory(),
        "policy":plan.credential_env().unwrap().semantic_descriptor(), "generated":inventory(&files),
        "consumerSourceSha256":format!("{:x}",Sha256::digest(source.as_bytes())),
        "consumerModuleSha256":format!("{:x}",Sha256::digest(module_file.as_bytes())),
    });
    std::fs::write(
        root.join("receipts/inputs.json"),
        serde_json::to_vec_pretty(&inputs).unwrap(),
    )
    .unwrap();
    for tool in tools() {
        let binary = consumer.join(format!("consumer-{tool}.test"));
        for (label, directory, args) in [
            ("version", &consumer, vec!["version".into()]),
            (
                "build-consumer",
                &consumer,
                vec![
                    "test".into(),
                    "-c".into(),
                    "-o".into(),
                    binary.to_string_lossy().into_owned(),
                    ".".into(),
                ],
            ),
            (
                "build-sdk",
                &root.join("go"),
                vec!["test".into(), "./...".into()],
            ),
            (
                "examples",
                &root.join("go"),
                vec!["run".into(), "./examples/validated".into()],
            ),
        ] {
            let output = native_command(
                &root,
                &format!("{tool}-{label}"),
                Command::new("go")
                    .current_dir(directory)
                    .args(&args)
                    .env("GOWORK", "off")
                    .env("GOTOOLCHAIN", &tool),
            );
            assert!(
                output.status.success(),
                "{} {tool} {args:?}\n{}{}",
                root.display(),
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            eprintln!(
                "{case}: {tool} {}: {}",
                args.join(" "),
                String::from_utf8_lossy(&output.stdout).trim()
            );
        }
        let output = native_command(
            &root,
            &format!("{tool}-consumer"),
            Command::new(&binary).current_dir(&consumer).args([
                "-test.count=1",
                "-test.timeout=30s",
                "-test.v",
            ]),
        );
        assert!(
            output.status.success(),
            "{} {tool}\n{}{}",
            root.display(),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        eprintln!(
            "{case}: {tool}: {}",
            String::from_utf8_lossy(&output.stdout).trim()
        );
        let binary_receipt = json!({"binary":binary, "sha256":format!("{:x}", Sha256::digest(std::fs::read(&binary).unwrap()))});
        std::fs::write(
            root.join("receipts").join(format!("{tool}-binary.json")),
            serde_json::to_vec_pretty(&binary_receipt).unwrap(),
        )
        .unwrap();
        for (index, (code, diagnostic)) in negative.iter().enumerate() {
            let path = consumer.join(format!("invalid{index}"));
            std::fs::create_dir_all(&path).unwrap();
            std::fs::write(
                path.join("invalid.go"),
                format!("package invalid\nimport sdk \"{module}\"\n{code}\n"),
            )
            .unwrap();
            let output = native_command(
                &root,
                &format!("{tool}-negative-{index}"),
                Command::new("go")
                    .current_dir(&consumer)
                    .args(["test", "-run=^$", &format!("./invalid{index}")])
                    .env("GOWORK", "off")
                    .env("GOTOOLCHAIN", &tool),
            );
            let stderr = String::from_utf8_lossy(&output.stderr);
            assert!(
                !output.status.success()
                    && stderr.contains("invalid.go:")
                    && stderr.contains(diagnostic),
                "{tool}: invalid boundary did not fail as expected: {stderr}"
            );
        }
        let docs = native_command(
            &root,
            &format!("{tool}-go-doc"),
            Command::new("go")
                .args(["doc", module, plan.credential_env_factory().unwrap()])
                .current_dir(&consumer)
                .env("GOWORK", "off")
                .env("GOTOOLCHAIN", &tool),
        );
        assert!(
            docs.status.success(),
            "{}",
            String::from_utf8_lossy(&docs.stderr)
        );
        eprintln!(
            "{case}: {tool}: {}",
            String::from_utf8_lossy(&docs.stdout).trim()
        );
    }
    if let Some(python) = std::env::var_os("SUSPECT_SPHINX_PYTHON") {
        let output = native_command(
            &root,
            "sphinx",
            Command::new(python)
                .args([
                    "-m",
                    "sphinx",
                    "-W",
                    "--keep-going",
                    "-b",
                    "html",
                    "docs",
                    "docs/_build/html",
                ])
                .current_dir(root.join("go"))
                .env("SUSPECT_GO_TOOLCHAIN", "go1.23.12"),
        );
        assert!(
            output.status.success(),
            "{}\n{}{}",
            root.display(),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        eprintln!("{case}: Sphinx warnings-denied passed");
    }
    if evidence.is_some() {
        eprintln!("credential-env native evidence: {}", root.display());
    } else {
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[test]
#[ignore = "requires native Go 1.23.12/1.27.1"]
fn native_env_factory_snapshots_and_keeps_whole_explicit_credentials_authoritative() {
    let plan = env_plan();
    native(
        "synthetic",
        "example.com/credential-env-sdk",
        &plan,
        ENV_CONSUMER,
        &[
            (
                "var _, _ = sdk.NewClient(nil,sdk.ClientOptions{})",
                "cannot use nil",
            ),
            (
                "var _ = (sdk.Credentials{}).WithBearer(\"token\",nil)",
                "cannot use nil",
            ),
            (
                "var _, _ = sdk.NewClientFromEnv(sdk.Credentials{})",
                "cannot use",
            ),
        ],
    );
}

#[test]
fn generator_environment_values_do_not_enter_configured_or_unconfigured_artifacts() {
    const MARKER: &str = "SUSPECT_GO_CREDENTIAL_ENV_CANARY_CHILD";
    const CANARY: &str = "GENERATING_PROCESS_SECRET_MUST_NEVER_APPEAR_719a4f";
    if std::env::var_os(MARKER).is_none() {
        let output = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "generator_environment_values_do_not_enter_configured_or_unconfigured_artifacts",
                "--nocapture",
            ])
            .env(MARKER, "1")
            .env("SUSPECT_GO_ENV_TOKEN", CANARY)
            .env("SUSPECT_GO_ENV_HEADER", CANARY)
            .env("SUSPECT_GO_ENV_QUERY", CANARY)
            .env("SUSPECT_GO_ENV_COOKIE", CANARY)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }
    let configured = env_plan();
    for file in configured.render().into_iter().chain(synthetic_files()) {
        assert!(
            !file.content.contains(CANARY),
            "generator environment leaked into {}",
            file.path
        );
    }
    assert!(
        !serde_json::to_string(configured.credential_env().unwrap())
            .unwrap()
            .contains(CANARY)
    );
    assert_eq!(
        inventory_hash(&inventory(&synthetic_files())),
        "74ffce12340c324c4d451738c30dde409e289ef04427a599b7f7eb8833e3c8ca"
    );
}

#[test]
#[ignore = "requires native Go 1.23.12/1.27.1"]
fn native_allocated_factory_handles_symbol_collision_and_bounded_env_input() {
    let mut value = document();
    value["components"]["schemas"]["NewClientFromEnv"] = json!({"type":"string"});
    value["paths"]["/anonymous"]["get"]["responses"]["200"]["content"]["application/json"]["schema"] =
        json!({"$ref":"#/components/schemas/NewClientFromEnv"});
    let configured = plan(
        supplied("https://fixtures.example.test/collision.json", value),
        go_http::HttpConfig {
            credential_env: Some(policy()),
            max_request_bytes: 512,
            ..Default::default()
        },
    );
    assert_eq!(
        configured.credential_env_factory(),
        Some("NewClientFromEnv2")
    );
    native(
        "collision",
        "example.com/credential-env-sdk",
        &configured,
        r#"package consumer
import("context";"errors";"io";"net/http";"strings";"testing";sdk "example.com/credential-env-sdk")
type doer func(*http.Request)(*http.Response,error)
func(f doer)Do(r *http.Request)(*http.Response,error){return f(r)}
func TestAllocatedAndBoundedFactory(t *testing.T){var model sdk.NewClientFromEnv="preserved model";_ = model;t.Setenv("SUSPECT_GO_ENV_TOKEN",strings.Repeat("a",513));calls:=0;client,err:=sdk.NewClientFromEnv2(sdk.ClientOptions{Transport:doer(func(*http.Request)(*http.Response,error){calls++;return &http.Response{StatusCode:200,Header:http.Header{"Content-Type":{"application/json"}},Body:io.NopCloser(strings.NewReader(`"ok"`))},nil})});if err!=nil{t.Fatal(err)};if _,err=client.AnonymousData(context.Background());err!=nil{t.Fatal(err)};_,err=client.ProtectedData(context.Background());var failure *sdk.SDKError;if !errors.As(err,&failure)||failure.Kind!="request-validation"||calls!=1{t.Fatal("oversized environment value reached HTTP",err,calls)}}
"#,
        &[],
    );
}

#[test]
#[ignore = "requires OPENROUTER_WEB_ROOT and native Go 1.23.12/1.27.1"]
fn native_actual_openrouter_key_and_credits_use_env_snapshot_and_source_https() {
    let path =
        PathBuf::from(std::env::var_os("OPENROUTER_WEB_ROOT").expect("set OPENROUTER_WEB_ROOT"))
            .join("projects/docs/openapi/openapi.yaml");
    let workspace = Arc::new(
        WorkspaceBuilder::new()
            .root(path.parent().unwrap())
            .build()
            .unwrap(),
    );
    let contract =
        Arc::new(Contract::from_workspace(&workspace, &Uri::from_path(&path).unwrap()).unwrap());
    let selected = contract
        .operations()
        .filter(|op| matches!(op.operation_id(), Some("getCurrentKey" | "getCredits")))
        .map(|op| op.source().clone())
        .collect::<Vec<_>>();
    assert_eq!(selected.len(), 2);
    let policy = suspect_codegen::credential_env::CredentialEnv::v1(BTreeMap::from([(
        "apiKey".into(),
        "OPENROUTER_API_KEY".into(),
    )]));
    let configured = go_http::plan_http(
        contract,
        &selected,
        go_http::HttpConfig {
            credential_env: Some(policy),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(
        configured.credential_env_factory(),
        Some("NewClientFromEnv")
    );
    let binding = &configured.credential_env().unwrap().bindings()[0];
    assert_eq!(
        binding.kind(),
        suspect_codegen::credential_env::CredentialEnvKind::Bearer
    );
    assert_eq!(
        binding.scheme().terminal().source().pointer(),
        "/components/securitySchemes/apiKey"
    );
    native(
        "openrouter",
        "github.com/openrouter/sdk-go",
        &configured,
        r#"package consumer
import("context";"errors";"io";"net/http";"strings";"testing";sdk "example.com/credential-env-sdk")
type doer func(*http.Request)(*http.Response,error)
func(f doer)Do(r *http.Request)(*http.Response,error){return f(r)}
func TestActualSourceDefaultHTTPS(t *testing.T){
 const key=`{"data":{"label":"controlled-key","limit":null,"limit_remaining":null,"limit_reset":null,"usage":25.75000000000000001,"usage_daily":0,"usage_weekly":0,"usage_monthly":0,"byok_usage":0,"byok_usage_daily":0,"byok_usage_weekly":0,"byok_usage_monthly":0,"is_free_tier":false,"is_management_key":false,"is_provisioning_key":false,"include_byok_in_limit":false,"creator_user_id":null,"rate_limit":{"requests":-1,"interval":"not-enforced","note":"controlled fixture"}}}`
 const credits=`{"data":{"total_credits":100.50000000000000001,"total_usage":25.75}}`
 t.Setenv("OPENROUTER_API_KEY","controlled-account-token");var paths []string
 options:=sdk.ClientOptions{Transport:doer(func(r *http.Request)(*http.Response,error){if r.Method!="GET"||r.URL.Scheme!="https"||r.URL.Host!="openrouter.ai"||r.Header.Get("Authorization")!="Bearer controlled-account-token"{t.Fatal("source/default credential attachment changed")};paths=append(paths,r.URL.String());body:=key;if r.URL.Path=="/api/v1/credits"{body=credits}else if r.URL.Path!="/api/v1/key"{t.Fatal(r.URL)};return &http.Response{StatusCode:200,Header:http.Header{"Content-Type":{"application/json"}},Body:io.NopCloser(strings.NewReader(body))},nil})}
 client,err:=sdk.NewClientFromEnv(options);if err!=nil{t.Fatal(err)};t.Setenv("OPENROUTER_API_KEY","")
 result,err:=client.GetCurrentKey(context.Background());if err!=nil{t.Fatal(err)};current:=result.(sdk.GetCurrentKeyStatus200);if current.Status!=200||current.Data.Data.Label!="controlled-key"||current.Data.Data.Usage.String()!="25.75000000000000001"{t.Fatal("actual key response schema/value changed")};_ = result.Close()
 management,err:=client.GetCredits(context.Background());if err!=nil{t.Fatal(err)};actual:=management.(sdk.GetCreditsStatus200);if actual.Status!=200||actual.Data.Data.TotalCredits.String()!="100.50000000000000001"{t.Fatal("actual credits response schema/value changed")};_ = management.Close()
 if strings.Join(paths,",")!="https://openrouter.ai/api/v1/key,https://openrouter.ai/api/v1/credits"{t.Fatal(paths)}
 missing,err:=sdk.NewClientFromEnv(options);if err!=nil{t.Fatal(err)};_,err=missing.GetCurrentKey(context.Background());var failure *sdk.SDKError;if !errors.As(err,&failure)||failure.Kind!="request-validation"||len(paths)!=2{t.Fatal("missing env reached HTTP",err)}
 explicit,err:=sdk.NewClient(sdk.Credentials{},options);if err!=nil{t.Fatal(err)};t.Setenv("OPENROUTER_API_KEY","must-not-fill-explicit-empty");_,err=explicit.GetCurrentKey(context.Background());if !errors.As(err,&failure)||failure.Kind!="request-validation"||len(paths)!=2{t.Fatal(err)}
}
"#,
        &[(
            "var _, _ = sdk.NewClient(nil,sdk.ClientOptions{})",
            "cannot use nil",
        )],
    );
}

const ENV_CONSUMER: &str = r#"package consumer
import("context";"errors";"fmt";"io";"net/http";"os";"strings";"testing";sdk "example.com/credential-env-sdk")
type doer func(*http.Request)(*http.Response,error)
func(f doer)Do(r *http.Request)(*http.Response,error){return f(r)}
func must[T any](value T,err error)T{if err!=nil{panic(err)};return value}
var names=[]string{"SUSPECT_GO_ENV_TOKEN","SUSPECT_GO_ENV_HEADER","SUSPECT_GO_ENV_QUERY","SUSPECT_GO_ENV_COOKIE"}
func unset(t *testing.T){t.Helper();for _,name:=range names{t.Setenv(name,"");if err:=os.Unsetenv(name);err!=nil{t.Fatal(err)}}}
func response()*http.Response{return &http.Response{StatusCode:200,Header:http.Header{"Content-Type":{"application/json"}},Body:io.NopCloser(strings.NewReader(`"ok"`))}}
func failed(t *testing.T,err error,secret string){t.Helper();var failure *sdk.SDKError;if !errors.As(err,&failure)||failure.Kind!="request-validation"{t.Fatalf("missing or invalid credentials must fail before HTTP: %v",err)};for _,text:=range []string{err.Error(),fmt.Sprintf("%+v",err)}{if secret!=""&&strings.Contains(text,secret){t.Fatal("credential leaked through diagnostic")}}}
func TestEnvSnapshotAndSourceServer(t *testing.T){
 unset(t);t.Setenv(names[0],"creation-one");var seen []string
 options:=sdk.ClientOptions{Transport:doer(func(r *http.Request)(*http.Response,error){if r.URL.String()!="https://source.example.test/api/v1/protected"||r.Method!="GET"{t.Fatal(r.URL,r.Method)};seen=append(seen,r.Header.Get("Authorization"));return response(),nil})}
 first:=must(sdk.NewClientFromEnv(options));t.Setenv(names[0],"creation-two");second:=must(sdk.NewClientFromEnv(options));if err:=os.Unsetenv(names[0]);err!=nil{t.Fatal(err)}
 if _,err:=first.ProtectedData(context.Background());err!=nil{t.Fatal(err)};if _,err:=second.ProtectedData(context.Background());err!=nil{t.Fatal(err)};if _,err:=first.ProtectedData(context.Background());err!=nil{t.Fatal(err)}
 if strings.Join(seen,",")!="Bearer creation-one,Bearer creation-two,Bearer creation-one"{t.Fatal("environment was not snapshotted at creation")}
 missing:=must(sdk.NewClientFromEnv(options));_,err:=missing.ProtectedData(context.Background());failed(t,err,"creation-two");if len(seen)!=3{t.Fatal("missing credential reached HTTP")}
 bare:=must(sdk.NewClientFromEnv());bare.CloseIdleConnections() // creation itself makes no HTTP call
 if _,err=sdk.NewClientFromEnv(options,options);err==nil{t.Fatal("multiple options accepted")}
}
func TestMissingEmptyAndUnusableValuesAllowAnonymous(t *testing.T){
 for _,value:=range []string{"","invalid\r\ncredential"}{unset(t);t.Setenv(names[0],value);calls:=0;client:=must(sdk.NewClientFromEnv(sdk.ClientOptions{Transport:doer(func(r *http.Request)(*http.Response,error){calls++;if r.Header.Get("Authorization")!=""{t.Fatal("anonymous operation gained credentials")};return response(),nil})}));if _,err:=client.AnonymousData(context.Background());err!=nil{t.Fatal(err)};_,err:=client.ProtectedData(context.Background());failed(t,err,value);if calls!=1{t.Fatal("invalid credentials reached HTTP",calls)}}
 unset(t);calls:=0;client:=must(sdk.NewClientFromEnv(sdk.ClientOptions{Transport:doer(func(*http.Request)(*http.Response,error){calls++;return response(),nil})}));if _,err:=client.AnonymousData(context.Background());err!=nil{t.Fatal(err)};_,err:=client.ProtectedData(context.Background());failed(t,err,"");if calls!=1{t.Fatal(calls)}
}
func TestWholeExplicitArgumentWins(t *testing.T){
 unset(t);for _,name:=range names{t.Setenv(name,"environment-secret")};calls:=0
 options:=sdk.ClientOptions{Transport:doer(func(r *http.Request)(*http.Response,error){calls++;if r.Header.Get("Authorization")!="Bearer explicit-token"{t.Fatal("explicit credential supplemented or replaced")};return response(),nil})}
 explicit:=must(sdk.NewClient(sdk.Credentials{}.WithBearer("token","explicit-token"),options));if _,err:=explicit.ProtectedData(context.Background());err!=nil{t.Fatal(err)}
 for _,credentials:=range []sdk.Credentials{sdk.Credentials{},sdk.Credentials{}.WithBearer("token",""),sdk.Credentials{}.WithAPIKey("headerKey","explicit-key")}{client:=must(sdk.NewClient(credentials,options));_,err:=client.ProtectedData(context.Background());failed(t,err,"environment-secret")}
 partial:=must(sdk.NewClient(sdk.Credentials{}.WithBearer("token","explicit-token"),options));_,err:=partial.ConjunctiveData(context.Background());failed(t,err,"environment-secret");if calls!=1{t.Fatal("explicit missing members filled from environment",calls)}
}
func TestSourceAlternativesConjunctionAndAnonymousChoice(t *testing.T){
 unset(t);t.Setenv(names[1],"header + value");calls:=0
 options:=sdk.ClientOptions{Transport:doer(func(r *http.Request)(*http.Response,error){calls++;if r.Header.Get("X-API-Key")!="header + value"||r.Header.Get("Authorization")!=""{t.Fatal("OR selection changed")};return response(),nil})}
 client:=must(sdk.NewClientFromEnv(options));if _,err:=client.AlternativeData(context.Background());err!=nil{t.Fatal(err)}
 t.Setenv(names[0],"bad bearer value");client=must(sdk.NewClientFromEnv(options));if _,err:=client.AlternativeData(context.Background());err!=nil{t.Fatal("unusable alternative blocked usable key",err)}
 choice:=0;options.SecurityAlternative=&choice;client=must(sdk.NewClientFromEnv(options));_,err:=client.AlternativeData(context.Background());failed(t,err,"bad bearer value");if calls!=2{t.Fatal(calls)}
 t.Setenv(names[0],"all-token");t.Setenv(names[2],"query +/");t.Setenv(names[3],"cookie +/")
 all:=must(sdk.NewClientFromEnv(sdk.ClientOptions{Transport:doer(func(r *http.Request)(*http.Response,error){calls++;if r.Header.Get("Authorization")!="Bearer all-token"||r.Header.Get("X-API-Key")!="header + value"||r.URL.RawQuery!="access_key=query%20%2B%2F"||r.Header.Get("Cookie")!="session_key=cookie%20%2B%2F"{t.Fatal("AND attachment changed",r.URL,r.Header)};return response(),nil})}));if _,err=all.ConjunctiveData(context.Background());err!=nil{t.Fatal(err)}
 anonymous:=must(sdk.NewClientFromEnv(sdk.ClientOptions{Transport:doer(func(r *http.Request)(*http.Response,error){calls++;if len(r.Header.Values("Authorization"))!=0{t.Fatal("anonymous OR member got authorization")};return response(),nil})}));if _,err=anonymous.OptionalData(context.Background());err!=nil{t.Fatal(err)}
 protectedChoice:=1;protected:=must(sdk.NewClientFromEnv(sdk.ClientOptions{SecurityAlternative:&protectedChoice,Transport:doer(func(r *http.Request)(*http.Response,error){calls++;if r.Header.Get("Authorization")!="Bearer all-token"{t.Fatal(r.Header)};return response(),nil})}));if _,err=protected.OptionalData(context.Background());err!=nil{t.Fatal(err)}
 if calls!=5{t.Fatal("unexpected retry or missing call",calls)}
}
"#;
