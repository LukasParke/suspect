//! Explicit runtime environment defaults through source-bound PHP plans and installed packages.
#![cfg(feature = "php-sdk")]

use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::Command,
    sync::Arc,
};
use suspect_codegen::{
    credential_env::{CredentialEnv, CredentialEnvKind},
    php_sdk::{self, PhpConfig, Plan},
};
use suspect_ir::contract::Contract;
use suspect_ref::WorkspaceBuilder;
use suspect_source::Uri;

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}
fn root(label: &str) -> PathBuf {
    let parent = repo().join("target/sdk-php-credential-env");
    fs::create_dir_all(&parent).unwrap();
    tempfile::Builder::new()
        .prefix(label)
        .tempdir_in(parent)
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
fn php() -> PathBuf {
    std::env::var_os("SUSPECT_PHP_BIN")
        .map(PathBuf::from)
        .unwrap_or_else(|| repo().join("target/sdk-php-tools/php-8.3.32/php"))
}
fn tool(variable: &str, name: &str) -> PathBuf {
    std::env::var_os(variable)
        .map(PathBuf::from)
        .unwrap_or_else(|| repo().join("target/sdk-php-tools").join(name))
}
fn config(policy: Option<CredentialEnv>) -> PhpConfig {
    PhpConfig {
        package_name: "openrouter/sdk".into(),
        package_version: "0.1.0".into(),
        namespace: "OpenRouter".into(),
        credential_env: policy,
        ..Default::default()
    }
}
fn policy() -> CredentialEnv {
    CredentialEnv::v1(
        [
            ("apiKey".into(), "SUSPECT_PHP_ENV_TOKEN".into()),
            ("headerKey".into(), "SUSPECT_PHP_ENV_HEADER".into()),
            ("queryKey".into(), "SUSPECT_PHP_ENV_QUERY".into()),
            ("cookieKey".into(), "SUSPECT_PHP_ENV_COOKIE".into()),
        ]
        .into_iter()
        .collect(),
    )
}
fn source() -> Value {
    let mut value = json!({"openapi":"3.1.0","info":{"title":"Explicit runtime credential defaults","version":"1"},"servers":[{"url":"https://openrouter.ai/api/v1"}],
        "components":{"securitySchemes":{"apiKey":{"type":"http","scheme":"bearer"},"headerKey":{"type":"apiKey","in":"header","name":"X-Api-Key"},"queryKey":{"type":"apiKey","in":"query","name":"key"},"cookieKey":{"type":"apiKey","in":"cookie","name":"session"}}},"paths":{}});
    for (path, id, security) in [
        ("/anonymous", "anonymous", json!([])),
        ("/bearer", "bearer", json!([{"apiKey":[]}])),
        (
            "/either",
            "either",
            json!([{"headerKey":[]},{"queryKey":[]}]),
        ),
        (
            "/all",
            "allKeys",
            json!([{"headerKey":[],"queryKey":[],"cookieKey":[]}]),
        ),
        ("/optional", "optional", json!([{}, {"apiKey":[]}])),
        ("/factory-name", "fromEnv", json!([])),
    ] {
        value["paths"][path] = json!({"get":{"operationId":id,"security":security,"responses":{"204":{"description":"Checked request"}}}});
    }
    value
}
fn fixture(value: &Value) -> (PathBuf, Arc<Contract>) {
    let root = root("case-");
    let path = root.join("api.json");
    fs::write(&path, value.to_string()).unwrap();
    (root, load(&path))
}
fn plan(contract: Arc<Contract>, config: PhpConfig) -> Result<Plan, Vec<php_sdk::HttpDiagnostic>> {
    let selected = contract
        .operations()
        .map(|op| op.source().clone())
        .collect::<Vec<_>>();
    php_sdk::plan_sdk(contract, &selected, config)
}
fn run(command: &mut Command, root: &Path, label: &str) {
    let output = command.output().unwrap();
    fs::write(
        root.join(format!("{label}.log")),
        [output.stdout.as_slice(), output.stderr.as_slice()].concat(),
    )
    .unwrap();
    writeln!(
        fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(root.join("commands.jsonl"))
            .unwrap(),
        "{}",
        json!({"label":label,"command":format!("{command:?}"),"exit":output.status.code()})
    )
    .unwrap();
    assert!(
        output.status.success(),
        "{}\n{command:?}\n{}{}",
        root.display(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    if label.starts_with("native") {
        assert!(output.stderr.is_empty());
        for word in ["Warning:", "Notice:", "Deprecated:", "Fatal error:"] {
            assert!(!String::from_utf8_lossy(&output.stdout).contains(word));
        }
    }
}
fn installed(root: &Path, plan: &Plan) -> PathBuf {
    let files = plan.render();
    let package = root.join("package");
    fs::create_dir(&package).unwrap();
    for file in &files {
        let path = package.join(file.path.strip_prefix("php/").unwrap());
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, &file.content).unwrap();
    }
    run(
        Command::new(php())
            .arg("-n")
            .arg(tool("SUSPECT_COMPOSER_PHAR", "composer-2.10.3.phar"))
            .args([
                "archive",
                "--format=zip",
                "--dir=build",
                "--file=package",
                "--no-plugins",
            ])
            .env("COMPOSER_HOME", root.join("composer-home"))
            .current_dir(&package),
        root,
        "archive",
    );
    let mut manifest: Value =
        serde_json::from_slice(&fs::read(package.join("composer.json")).unwrap()).unwrap();
    manifest["dist"] = json!({"type":"zip","url":Uri::from_path(&package.join("build/package.zip")).unwrap().to_string()});
    let consumer = root.join("consumer");
    fs::create_dir(&consumer).unwrap();
    fs::write(consumer.join("composer.json"),json!({"name":"fixture/credential-consumer","require":{"openrouter/sdk":"0.1.0"},"repositories":[{"type":"package","package":manifest},{"packagist.org":false}],"config":{"allow-plugins":false}}).to_string()).unwrap();
    run(
        Command::new(php())
            .arg("-n")
            .arg(tool("SUSPECT_COMPOSER_PHAR", "composer-2.10.3.phar"))
            .args([
                "install",
                "--no-dev",
                "--no-plugins",
                "--no-scripts",
                "--no-progress",
            ])
            .env("COMPOSER_HOME", root.join("composer-home"))
            .current_dir(&consumer),
        root,
        "install",
    );
    let installed = consumer.join("vendor/openrouter/sdk");
    for file in &files {
        assert_eq!(
            fs::read(installed.join(file.path.strip_prefix("php/").unwrap())).unwrap(),
            file.content.as_bytes()
        );
    }
    fs::write(root.join("emitted-manifest.json"),serde_json::to_string_pretty(&files.iter().map(|file|json!({"path":file.path,"sha256":format!("{:x}",Sha256::digest(file.content.as_bytes()))})).collect::<Vec<_>>()).unwrap()).unwrap();
    run(
        Command::new(php())
            .arg("-n")
            .arg(tool("SUSPECT_PHPSTAN_PHAR", "phpstan-2.2.13.phar"))
            .args([
                "analyse",
                "--no-progress",
                "--memory-limit=1G",
                "--autoload-file",
            ])
            .arg(consumer.join("vendor/autoload.php"))
            .current_dir(&installed),
        root,
        "package-types",
    );
    consumer
}

#[test]
fn bound_policy_and_helper_allocation_preserve_native_source_identity() {
    let (_, contract) = fixture(&source());
    let ordinary = plan(contract.clone(), config(None)).unwrap();
    assert!(ordinary.credential_env().is_none());
    assert_eq!(
        ordinary
            .operations()
            .iter()
            .find(|op| op.id == "fromEnv")
            .unwrap()
            .method,
        "fromEnv"
    );
    let configured = plan(contract, config(Some(policy()))).unwrap();
    let bound = configured.credential_env().unwrap();
    assert_eq!(bound.bindings().len(), 4);
    let bearer = bound
        .bindings()
        .iter()
        .find(|binding| binding.name() == "apiKey")
        .unwrap();
    assert_eq!(bearer.kind(), CredentialEnvKind::Bearer);
    assert_eq!(bearer.variable(), "SUSPECT_PHP_ENV_TOKEN");
    assert_eq!(
        bearer.scheme().use_site().source().pointer(),
        "/components/securitySchemes/apiKey"
    );
    assert_eq!(
        configured
            .operations()
            .iter()
            .find(|op| op.id == "fromEnv")
            .unwrap()
            .method,
        "fromEnv2"
    );
    let (_, relocated) = fixture(&source());
    let relocated = plan(relocated, config(Some(policy()))).unwrap();
    assert_eq!(
        bound.semantic_descriptor(),
        relocated.credential_env().unwrap().semantic_descriptor()
    );
    let descriptor = serde_json::to_value(bound.semantic_descriptor()).unwrap();
    assert!(
        descriptor["bindings"]
            .as_array()
            .unwrap()
            .iter()
            .all(|binding| binding
                .as_object()
                .unwrap()
                .keys()
                .all(|key| ["name", "variable", "kind"].contains(&key.as_str())))
    );
}

#[test]
fn canonical_generation_capture_and_policy_reverts_preserve_php_semantics() {
    use suspect_codegen::{
        backend::{self, Backend, GenerationOptions, TargetConfig},
        compatibility::{self, PlanStatus},
    };

    let (root, contract) = fixture(&source());
    let selected = contract
        .operations()
        .map(|op| op.source().clone())
        .collect::<Vec<_>>();
    let target = TargetConfig {
        backend: Backend::PhpHttp,
        package_name: "openrouter/sdk".into(),
        package_version: "0.1.0".into(),
        import_name: Some("OpenRouter".into()),
    };
    let options = GenerationOptions {
        credential_env: Some(policy()),
        ..Default::default()
    };
    let direct = plan(contract.clone(), config(Some(policy()))).unwrap();
    let generated =
        backend::generate_with_options(contract.clone(), &selected, &target, &options).unwrap();
    assert_eq!(
        generated,
        direct.render(),
        "canonical forwarding must use the admitted native policy"
    );
    let capture = |options: &GenerationOptions| {
        compatibility::snapshot_with_options(
            contract.clone(),
            &[],
            std::slice::from_ref(&target),
            options,
        )
        .unwrap()
    };
    let before = capture(&options);
    let native = &before.native[0];
    assert_eq!(native.status, PlanStatus::Planned, "{:?}", native.findings);
    assert_eq!(
        native.credential_env,
        direct.credential_env().map(|env| env.semantic_descriptor())
    );
    let factory = native
        .models
        .iter()
        .find(|model| model.role == "credential-env-factory")
        .unwrap();
    assert_eq!(factory.name, "OpenRouter\\Client::fromEnv");
    let descriptor = factory.descriptor.as_ref().unwrap();
    assert_eq!(descriptor["kind"], "static-method");
    assert_eq!(descriptor["environmentRead"], "client-factory-time");
    assert_eq!(descriptor["explicitConstructorUsesEnvironment"], false);
    assert_eq!(
        descriptor["parameters"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p["name"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["transport", "options"]
    );
    assert_eq!(
        native
            .operations
            .iter()
            .find(|op| op.operation_id == "fromEnv")
            .unwrap()
            .symbols["method"],
        "fromEnv2"
    );

    let (_, relocated) = fixture(&source());
    let relocated = compatibility::snapshot_with_options(
        relocated,
        &[],
        std::slice::from_ref(&target),
        &options,
    )
    .unwrap();
    assert_eq!(
        native.credential_env, relocated.native[0].credential_env,
        "physical scheme provenance is not semantic policy identity"
    );
    assert!(compatibility::compare_snapshots(&before, &relocated).is_proven_compatible());

    let mut edited = options.clone();
    edited
        .credential_env
        .as_mut()
        .unwrap()
        .schemes
        .insert("apiKey".into(), "SUSPECT_PHP_ENV_TOKEN_B".into());
    let after = capture(&edited);
    let edited_files =
        backend::generate_with_options(contract.clone(), &selected, &target, &edited).unwrap();
    assert_ne!(
        generated, edited_files,
        "the configured variable name is part of emitted factory behavior"
    );
    let changed = compatibility::compare_snapshots(&before, &after);
    assert!(
        changed.wire.is_empty(),
        "an environment-default edit must not change source wire interpretation"
    );
    assert!(
        changed.native[0]
            .changes
            .iter()
            .any(|change| change.code == "native-credential-env-changed")
    );
    let restored = capture(&options);
    assert_eq!(native.credential_env, restored.native[0].credential_env);
    assert!(compatibility::compare_snapshots(&before, &restored).is_proven_compatible());
    assert_eq!(
        generated,
        backend::generate_with_options(contract.clone(), &selected, &target, &options).unwrap()
    );

    let ordinary = GenerationOptions::default();
    let unconfigured = capture(&ordinary);
    assert!(unconfigured.native[0].credential_env.is_none());
    assert!(
        !unconfigured.native[0]
            .models
            .iter()
            .any(|model| model.role == "credential-env-factory")
    );
    assert_eq!(
        unconfigured.native[0]
            .operations
            .iter()
            .find(|op| op.operation_id == "fromEnv")
            .unwrap()
            .symbols["method"],
        "fromEnv"
    );
    assert_eq!(
        backend::generate_with_options(contract, &selected, &target, &ordinary).unwrap(),
        plan(before.contract().clone(), config(None))
            .unwrap()
            .render()
    );
    fs::write(
        root.join("canonical-capture.json"),
        serde_json::to_string_pretty(&json!({
            "configured": native, "edited": after.native[0], "restored": restored.native[0],
            "unconfigured": unconfigured.native[0], "policyChange": changed,
            "canonicalMatchesDirect": true, "restoredArtifactsEqual": true,
        }))
        .unwrap(),
    )
    .unwrap();
    println!("PHP canonical credential env evidence: {}", root.display());
}

#[test]
fn generation_does_not_capture_environment_values() {
    const CANARY: &str = "generator-private-canary-php-env-20260911";
    if std::env::var_os("SUSPECT_PHP_ENV_GENERATION_CHILD").is_some() {
        assert_eq!(std::env::var("SUSPECT_PHP_ENV_TOKEN").unwrap(), CANARY);
        let (_, contract) = fixture(&source());
        let generated = plan(contract, config(Some(policy()))).unwrap();
        assert!(
            generated
                .render()
                .iter()
                .all(|file| !file.content.contains(CANARY))
        );
        assert!(
            !serde_json::to_string(generated.credential_env().unwrap())
                .unwrap()
                .contains(CANARY)
        );
        return;
    }
    let root = root("canary-");
    run(
        Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "generation_does_not_capture_environment_values",
                "--test-threads=1",
            ])
            .env("SUSPECT_PHP_ENV_GENERATION_CHILD", "1")
            .env("SUSPECT_PHP_ENV_TOKEN", CANARY),
        &root,
        "generator-canary",
    );
}

#[test]
fn php_admission_keeps_shared_unsupported_credential_findings_located() {
    let mut value = source();
    value["components"]["securitySchemes"]["basic"] = json!({"type":"http","scheme":"basic"});
    value["paths"]["/basic"] = json!({"get":{"operationId":"basic","security":[{"basic":[]}],"responses":{"204":{"description":"Basic"}}}});
    let (_, contract) = fixture(&value);
    let error = plan(
        contract,
        config(Some(CredentialEnv::v1(
            [("basic".into(), "BASIC_SECRET".into())]
                .into_iter()
                .collect(),
        ))),
    )
    .unwrap_err();
    assert!(error.iter().any(|e| e.code == "sdk-credential-env-kind"
        && e.source.pointer() == "/components/securitySchemes/basic"
        && !e.at.is_empty()));
}

#[test]
#[ignore = "installed Composer/PHPStan runtime env gate; PHP 8.3/8.5 selector SUSPECT_PHP_BIN"]
fn native_env_snapshot_explicit_auth_and_unavailable_platform() {
    let (root, contract) = fixture(&source());
    let planned = plan(contract, config(Some(policy()))).unwrap();
    let consumer = installed(&root, &planned);
    fs::write(consumer.join("positive.php"), ENV_CONSUMER).unwrap();
    run(
        Command::new(php())
            .arg("-n")
            .arg(tool("SUSPECT_PHPSTAN_PHAR", "phpstan-2.2.13.phar"))
            .args([
                "analyse",
                "--no-progress",
                "--level=max",
                "--autoload-file=vendor/autoload.php",
                "positive.php",
            ])
            .current_dir(&consumer),
        &root,
        "consumer-types",
    );
    run(
        Command::new(php())
            .args(["-n", "-d", "error_reporting=-1", "positive.php"])
            .env("SUSPECT_PHP_ENV_TOKEN", "before-import-token")
            .current_dir(&consumer),
        &root,
        "native-env",
    );
    fs::write(consumer.join("unavailable.php"), UNAVAILABLE_CONSUMER).unwrap();
    run(
        Command::new(php())
            .args([
                "-n",
                "-d",
                "disable_functions=getenv",
                "-d",
                "error_reporting=-1",
                "unavailable.php",
            ])
            .env("SUSPECT_PHP_ENV_TOKEN", "inaccessible-token")
            .current_dir(&consumer),
        &root,
        "native-unavailable",
    );
    fs::write(consumer.join("negative.php"),"<?php\nrequire __DIR__.'/vendor/autoload.php';\nnew OpenRouter\\Client(null);\nnew OpenRouter\\Client(OpenRouter\\Absent::Value);\nOpenRouter\\Client::fromEnv(transport: null);\n").unwrap();
    let output = Command::new(php())
        .arg("-n")
        .arg(tool("SUSPECT_PHPSTAN_PHAR", "phpstan-2.2.13.phar"))
        .args([
            "analyse",
            "--no-progress",
            "--level=max",
            "--error-format=json",
            "--autoload-file=vendor/autoload.php",
            "negative.php",
        ])
        .current_dir(&consumer)
        .output()
        .unwrap();
    fs::write(root.join("negative-types.json"), &output.stdout).unwrap();
    assert_eq!(output.status.code(), Some(1));
    let errors: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(errors["totals"]["file_errors"], 3);
    println!("PHP credential env evidence: {}", root.display());
}
const ENV_CONSUMER: &str = r#"<?php
declare(strict_types=1);
require __DIR__.'/vendor/autoload.php';
use OpenRouter as S;
function check(bool $ok,string $label):void{if(!$ok){throw new RuntimeException($label);}}
final class Recorder implements S\Transport {
    /** @var list<S\HttpRequest> */ public array $requests=[];
    public function send(S\HttpRequest $request):S\HttpResponse{$request->check();$this->requests[]=$request;return new S\HttpResponse(204,[],'');}
    public function last():S\HttpRequest{$key=array_key_last($this->requests)??throw new RuntimeException('no request');return $this->requests[$key];}
}
function missing(Closure $call,Recorder $transport,string $label):void{
    $before=count($transport->requests);
    try{$call();throw new RuntimeException($label.' did not fail');}catch(S\SdkError $error){check($error->kind==='credentials',$label.' classification');check(!str_contains((string)$error,'private-'),$label.' secrecy');}
    check(count($transport->requests)===$before,$label.' must fail before HTTP');
}
foreach(['SUSPECT_PHP_ENV_TOKEN','SUSPECT_PHP_ENV_HEADER','SUSPECT_PHP_ENV_QUERY','SUSPECT_PHP_ENV_COOKIE'] as$name){putenv($name);}
$transport=new Recorder();$empty=S\Client::fromEnv(transport:$transport);
$empty->anonymous();$empty->fromEnv2();missing(static fn()=> $empty->bearer(),$transport,'missing env');
check($transport->last()->url==='https://openrouter.ai/api/v1/factory-name','source default HTTPS server');
putenv('SUSPECT_PHP_ENV_TOKEN=');$blank=S\Client::fromEnv(transport:$transport);$blank->anonymous();missing(static fn()=> $blank->bearer(),$transport,'empty env');
putenv('SUSPECT_PHP_ENV_TOKEN=private-first-token');$snapshot=S\Client::fromEnv(transport:$transport);
putenv('SUSPECT_PHP_ENV_TOKEN=private-second-token');$snapshot->bearer();check($transport->last()->headers['authorization']==='Bearer private-first-token','creation-time snapshot');
$new=S\Client::fromEnv(transport:$transport);$new->bearer();check($transport->last()->headers['authorization']==='Bearer private-second-token','new client observes new env');
putenv('SUSPECT_PHP_ENV_TOKEN');$snapshot->bearer();check($transport->last()->headers['authorization']==='Bearer private-first-token','no per-request lookup');
putenv('SUSPECT_PHP_ENV_TOKEN=private-fallback-token');
$explicit=new S\Client(new S\Credentials(['apiKey'=>'private-explicit-token']),$transport);$explicit->bearer();check($transport->last()->headers['authorization']==='Bearer private-explicit-token','explicit value wins');
$explicitEmpty=new S\Client(new S\Credentials([]),$transport);$explicitEmpty->anonymous();missing(static fn()=> $explicitEmpty->bearer(),$transport,'whole explicit empty map');
$partial=new S\Client(new S\Credentials(['headerKey'=>new S\ApiKeyCredential('explicit-header')]),$transport);missing(static fn()=> $partial->bearer(),$transport,'explicit missing member');
/** @var list<mixed> $invalid */$invalid=[null,S\Absent::Value];
foreach($invalid as$value){$before=count($transport->requests);try{(new ReflectionClass(S\Client::class))->newInstanceArgs([$value,$transport]);throw new RuntimeException('explicit invalid argument accepted');}catch(TypeError $error){check(count($transport->requests)===$before,'explicit null/Absent never use env');}}
try{new S\Client(new S\Credentials(['apiKey'=>'']),$transport);throw new RuntimeException('explicit empty token accepted');}catch(S\SdkError $error){check($error->kind==='credentials','explicit empty token keeps old validation');}
putenv('SUSPECT_PHP_ENV_QUERY=private-query/a');$or=S\Client::fromEnv(transport:$transport);$or->either();check($transport->last()->url==='https://openrouter.ai/api/v1/either?key=private-query%2Fa','available OR alternative');
missing(static fn()=> $or->either(options:new S\RequestOptions(securityAlternative:0)),$transport,'explicit unsatisfied alternative');
putenv('SUSPECT_PHP_ENV_HEADER=private-header');putenv('SUSPECT_PHP_ENV_COOKIE=private-cookie');$and=S\Client::fromEnv(transport:$transport);$and->allKeys();check($transport->last()->headers['x-api-key']==='private-header'&&$transport->last()->headers['cookie']==='session=private-cookie','AND header/cookie values');
check($transport->last()->url==='https://openrouter.ai/api/v1/all?key=private-query%2Fa','AND query value');
missing(static fn()=> $partial->allKeys(),$transport,'explicit AND partial map is not filled');
putenv('SUSPECT_PHP_ENV_COOKIE=');$incomplete=S\Client::fromEnv(transport:$transport);missing(static fn()=> $incomplete->allKeys(),$transport,'missing AND member');
$and->allKeys();check($transport->last()->headers['cookie']==='session=private-cookie','AND snapshot retained');
$and->optional();check(!isset($transport->last()->headers['authorization']),'anonymous alternative remains first');
$and->optional(options:new S\RequestOptions(securityAlternative:1));check($transport->last()->headers['authorization']==='Bearer private-fallback-token','explicit protected alternative');
putenv('SUSPECT_PHP_ENV_TOKEN=private invalid bearer');$unusable=S\Client::fromEnv(transport:$transport);$unusable->anonymous();missing(static fn()=> $unusable->bearer(),$transport,'unusable env token');
putenv('SUSPECT_PHP_ENV_TOKEN='.str_repeat('x',8193));$oversized=S\Client::fromEnv(transport:$transport);$oversized->anonymous();missing(static fn()=> $oversized->bearer(),$transport,'bounded token');
check((new ReflectionMethod(S\Client::class,'fromEnv'))->isStatic(),'native static env factory');
echo 'runtime env snapshots, explicit precedence, OR/AND/anonymous and source HTTPS checks passed',PHP_EOL;
"#;
const UNAVAILABLE_CONSUMER: &str = r#"<?php
declare(strict_types=1);
require __DIR__.'/vendor/autoload.php';
use OpenRouter as S;
if(function_exists('getenv')){throw new RuntimeException('environment API must be unavailable in this fixture');}
$transport=new class implements S\Transport {public int $calls=0;public function send(S\HttpRequest $request):S\HttpResponse{++$this->calls;return new S\HttpResponse(204,[],'');}};
$client=S\Client::fromEnv(transport:$transport);$client->anonymous();
try{$client->bearer();throw new RuntimeException('missing protected credential accepted');}catch(S\SdkError $error){if($error->kind!=='credentials'||str_contains((string)$error,'inaccessible-token')){throw new RuntimeException('bad missing-credential error');}}
if($transport->calls!==1){throw new RuntimeException('protected request reached transport');}
(new S\Client(new S\Credentials(['apiKey'=>'explicit-token']),$transport))->bearer();
echo 'unavailable getenv preserves anonymous and explicit-credential operations',PHP_EOL;
"#;

#[test]
#[ignore = "actual read-only OpenRouter source, installed branded SDK, controlled transport; PHP 8.3/8.5"]
fn native_openrouter_current_key_factory_uses_source_https() {
    let source = PathBuf::from(
        std::env::var_os("OPENROUTER_WEB_ROOT")
            .expect("OPENROUTER_WEB_ROOT must identify the read-only original source"),
    )
    .join("projects/docs/openapi/openapi.yaml");
    let before = fs::read(&source).unwrap();
    let contract = load(&source);
    let wanted = ["getCurrentKey", "getCredits"];
    let selected = contract
        .operations()
        .filter(|op| wanted.contains(&op.operation_id().unwrap_or_default()))
        .map(|op| op.source().clone())
        .collect::<Vec<_>>();
    assert_eq!(selected.len(), 2);
    let policy = CredentialEnv::v1(
        [("apiKey".into(), "OPENROUTER_API_KEY".into())]
            .into_iter()
            .collect(),
    );
    let planned = php_sdk::plan_sdk(contract, &selected, config(Some(policy))).unwrap();
    let root = root("openrouter-");
    let consumer = installed(&root, &planned);
    for id in wanted {
        let examples = planned
            .examples()
            .operations()
            .iter()
            .find(|op| op.operation_id == id)
            .unwrap();
        let value = &examples
            .entries
            .iter()
            .find(|entry| {
                matches!(
                    entry.role,
                    suspect_codegen::examples::ExampleRole::Response { status: 200 }
                )
            })
            .unwrap()
            .value;
        fs::write(consumer.join(format!("{id}.json")), value.to_string()).unwrap();
    }
    fs::write(consumer.join("positive.php"), OPENROUTER_CONSUMER).unwrap();
    run(
        Command::new(php())
            .arg("-n")
            .arg(tool("SUSPECT_PHPSTAN_PHAR", "phpstan-2.2.13.phar"))
            .args([
                "analyse",
                "--no-progress",
                "--level=max",
                "--autoload-file=vendor/autoload.php",
                "positive.php",
            ])
            .current_dir(&consumer),
        &root,
        "consumer-types",
    );
    run(
        Command::new(php())
            .args(["-n", "-d", "error_reporting=-1", "positive.php"])
            .env("OPENROUTER_API_KEY", "controlled-user-token")
            .current_dir(&consumer),
        &root,
        "native-current-key",
    );
    assert_eq!(fs::read(&source).unwrap(), before);
    fs::write(root.join("source-provenance.json"),serde_json::to_string_pretty(&json!({"path":source,"sha256":format!("{:x}",Sha256::digest(&before)),"operations":wanted,"packageName":"openrouter/sdk","namespace":"OpenRouter","transport":"controlled, source-default HTTPS, no real account request"})).unwrap()).unwrap();
    println!("PHP OpenRouter credential env evidence: {}", root.display());
}
const OPENROUTER_CONSUMER: &str = r#"<?php
declare(strict_types=1);
require __DIR__.'/vendor/autoload.php';
use OpenRouter as S;
function check(bool $ok):void{if(!$ok){throw new RuntimeException('OpenRouter env factory mismatch');}}
$transport=new class implements S\Transport {
    /** @var list<S\HttpRequest> */public array $requests=[];
    public function send(S\HttpRequest $request):S\HttpResponse{
        $request->check();$this->requests[]=$request;
        $file=match($request->url){'https://openrouter.ai/api/v1/key'=>'getCurrentKey.json','https://openrouter.ai/api/v1/credits'=>'getCredits.json',default=>throw new RuntimeException('source HTTPS server or path changed')};
        $json=file_get_contents(__DIR__.'/'.$file);if($json===false){throw new RuntimeException('source response fixture missing');}return new S\HttpResponse(200,['Content-Type'=>'application/json'],$json);
    }
};
$client=S\Client::fromEnv(transport:$transport);
$key=$client->getCurrentKey();check($key->response->status===200);check($key->body->data->isManagementKey===false);check($key->body->data->usage->toDecimalString(maxBytes:128)!=='');
putenv('OPENROUTER_API_KEY=controlled-management-token');$client->getCurrentKey();
$management=S\Client::fromEnv(transport:$transport);$credits=$management->getCredits();check($credits->response->status===200&&$credits->body->data->totalCredits->token!=='');
check(count($transport->requests)===3);check($transport->requests[0]->method==='GET'&&$transport->requests[0]->headers['authorization']==='Bearer controlled-user-token');check($transport->requests[1]->headers['authorization']==='Bearer controlled-user-token');check($transport->requests[2]->headers['authorization']==='Bearer controlled-management-token');
putenv('OPENROUTER_API_KEY');$missing=S\Client::fromEnv(transport:$transport);try{$missing->getCurrentKey();throw new RuntimeException('missing key reached transport');}catch(S\SdkError $error){check($error->kind==='credentials');}check(count($transport->requests)===3);
echo 'OpenRouter getCurrentKey/default HTTPS and explicit getCredits mode passed with controlled transport',PHP_EOL;
"#;
