//! Source-driven native adoption gates for the verified v2 profile and explicit v3 fence.
use super::*;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fmt::Write as _,
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::Command,
};
use suspect_ref::WorkspaceBuilder;
use suspect_source::Uri;

pub(super) fn repo() -> PathBuf {
    std::env::var_os("SUSPECT_REPO")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../..")
                .canonicalize()
                .unwrap()
        })
}
pub(super) fn php() -> PathBuf {
    std::env::var_os("SUSPECT_PHP_BIN")
        .map(PathBuf::from)
        .unwrap_or_else(|| repo().join("target/sdk-php-tools/php-8.3.32/php"))
}
pub(super) fn composer() -> PathBuf {
    std::env::var_os("SUSPECT_COMPOSER_PHAR")
        .map(PathBuf::from)
        .unwrap_or_else(|| repo().join("target/sdk-php-tools/composer-2.10.3.phar"))
}
pub(super) fn phpstan() -> PathBuf {
    std::env::var_os("SUSPECT_PHPSTAN_PHAR")
        .map(PathBuf::from)
        .unwrap_or_else(|| repo().join("target/sdk-php-tools/phpstan-2.2.13.phar"))
}
fn root(label: &str) -> PathBuf {
    let path = repo().join("target/sdk-php-validation-v2");
    fs::create_dir_all(&path).unwrap();
    tempfile::Builder::new()
        .prefix(label)
        .tempdir_in(path)
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
pub(super) fn literal(value: &str) -> String {
    format!("'{}'", value.replace('\\', "\\\\").replace('\'', "\\'"))
}
pub(super) fn run(command: &mut Command, root: &Path, label: &str) {
    let output = command.output().unwrap();
    fs::write(
        root.join(format!("{label}.log")),
        [output.stdout.as_slice(), output.stderr.as_slice()].concat(),
    )
    .unwrap();
    writeln!(
        fs::OpenOptions::new()
            .append(true)
            .create(true)
            .open(root.join("commands.jsonl"))
            .unwrap(),
        "{}",
        json!({"command":format!("{command:?}"),"label":label,"exit":output.status.code()})
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
        let text = String::from_utf8_lossy(&output.stdout);
        for word in ["Warning:", "Deprecated:", "Notice:", "Fatal error:"] {
            assert!(!text.contains(word), "{text}");
        }
        assert!(
            output.stderr.is_empty(),
            "native stderr: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
pub(super) fn install(root: &Path, files: &[OutFile], name: &str) -> PathBuf {
    let package = root.join("package");
    fs::create_dir(&package).unwrap();
    for file in files {
        let path = package.join(file.path.strip_prefix("php/").unwrap_or(&file.path));
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, &file.content).unwrap();
    }
    let mut manifest = json!({"name":format!("fixture/{name}"),"version":"0.1.0","type":"library","license":"proprietary","require":{"php":"^8.3","ext-json":"*"},"autoload":{"classmap":["src/"]},"archive":{"exclude":["/vendor","/build","/composer.lock"]}});
    if !package.join("composer.json").exists() {
        fs::write(package.join("composer.json"), manifest.to_string()).unwrap();
    } else {
        manifest =
            serde_json::from_slice(&fs::read(package.join("composer.json")).unwrap()).unwrap();
    }
    let tools = repo().join("target/sdk-php-tools");
    run(
        Command::new(php())
            .arg("-n")
            .arg(composer())
            .args([
                "archive",
                "--format=zip",
                "--dir=build",
                "--file=package",
                "--no-plugins",
            ])
            .env("COMPOSER_HOME", root.join("composer-home"))
            .env("COMPOSER_CACHE_DIR", tools.join("composer-cache"))
            .current_dir(&package),
        root,
        "composer-archive",
    );
    manifest["dist"] = json!({"type":"zip","url":Uri::from_path(&package.join("build/package.zip")).unwrap().to_string()});
    let consumer = root.join("consumer");
    fs::create_dir(&consumer).unwrap();
    fs::write(consumer.join("composer.json"),json!({"name":"fixture/consumer","require":{manifest["name"].as_str().unwrap():"0.1.0"},"repositories":[{"type":"package","package":manifest},{"packagist.org":false}],"config":{"allow-plugins":false}}).to_string()).unwrap();
    run(
        Command::new(php())
            .arg("-n")
            .arg(composer())
            .args([
                "install",
                "--no-dev",
                "--no-scripts",
                "--no-plugins",
                "--no-progress",
            ])
            .env("COMPOSER_HOME", root.join("composer-home"))
            .env("COMPOSER_CACHE_DIR", tools.join("composer-cache"))
            .current_dir(&consumer),
        root,
        "composer-install",
    );
    let installed = consumer
        .join("vendor")
        .join(manifest["name"].as_str().unwrap());
    for file in files {
        assert_eq!(
            fs::read(installed.join(file.path.strip_prefix("php/").unwrap_or(&file.path))).unwrap(),
            file.content.as_bytes(),
            "installed artifact drift: {}",
            file.path
        );
    }
    fs::write(root.join("emitted-manifest.json"),serde_json::to_string_pretty(&files.iter().map(|f|json!({"path":f.path,"sha256":format!("{:x}",Sha256::digest(f.content.as_bytes()))})).collect::<Vec<_>>()).unwrap()).unwrap();
    consumer
}
pub(super) fn typecheck(root: &Path, consumer: &Path, paths: &[&str], label: &str) {
    run(
        Command::new(php())
            .arg("-n")
            .arg(phpstan())
            .args([
                "analyse",
                "--no-progress",
                "--memory-limit=2G",
                "--level=max",
                "--autoload-file=vendor/autoload.php",
            ])
            .args(paths)
            .current_dir(consumer),
        root,
        label,
    );
}
fn instance_path(id: &str) -> &'static str {
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

#[test]
#[ignore = "source-driven 32-case v2 corpus; Composer-installed PHP and PHPStan required"]
fn native_v2_source_vectors() {
    let root = root("vectors-");
    let source = repo().join("crates/suspect-schema/tests/fixtures/owned-applicators-v2.json");
    let bytes = fs::read(&source).unwrap();
    let vectors: Value = serde_json::from_slice(&bytes).unwrap();
    let cases = vectors["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 32);
    execute_vectors(
        &root,
        cases,
        json!({"fixture":source,"fixtureSha256":format!("{:x}",Sha256::digest(bytes))}),
        None,
    );
}

fn execute_vectors(root: &Path, cases: &[Value], provenance: Value, extra: Option<&str>) {
    let mut files = Vec::new();
    let mut script = String::from(
        "<?php\ndeclare(strict_types=1);\nrequire __DIR__.'/vendor/autoload.php';\nfunction check(bool $ok,string $id):void{if(!$ok){throw new RuntimeException($id);}}\n$results=[];\n",
    );
    let mut records = Vec::new();
    for (index, case) in cases.iter().enumerate() {
        let directory = root.join(format!("source-{index:02}"));
        fs::create_dir(&directory).unwrap();
        let path = directory.join("api.json");
        fs::write(&path,format!("{{\"openapi\":\"3.1.0\",\"info\":{{\"title\":\"V2 native source vectors\",\"version\":\"1\"}},\"components\":{{\"schemas\":{{\"Root\":{}}}}}}}",case["schemaJson"].as_str().unwrap())).unwrap();
        let contract = load(&path);
        let id = SourceId::new(contract.entry().clone(), Default::default())
            .child("components")
            .child("schemas")
            .child("Root");
        let mut config = PhpConfig {
            namespace: format!("NativeV2\\C{index}"),
            ..Default::default()
        };
        if let Some(limit) = case["limits"]["maxNumberBytes"].as_u64() {
            config.validation.max_number_bytes = limit as usize;
        }
        if let Some(limit) = case["limits"]["maxEvaluationSteps"].as_u64() {
            config.validation.max_evaluation_steps = limit as usize;
        }
        if let Some(limit) = case["limits"]["maxDepth"].as_u64() {
            config.validation.max_depth = limit as usize;
        }
        if let Some(limit) = case["limits"]["maxEqualitySteps"].as_u64() {
            config.validation.max_equality_steps = limit as usize;
        }
        let program = suspect_schema::OwnedCompiler::new(config.validation.clone())
            .compile_v2(contract, std::slice::from_ref(&id))
            .unwrap()
            .program();
        assert_eq!(program.check(), Ok(()));
        for mut file in emit_validation(&program, &config).unwrap() {
            file.path = format!(
                "php/src/C{index}/{}",
                file.path.strip_prefix("php/src/").unwrap()
            );
            files.push(file);
        }
        let ns = &config.namespace;
        let root_index = program
            .roots
            .iter()
            .find(|r| r.source.pointer == id.pointer())
            .unwrap()
            .target;
        let name = case["id"].as_str().unwrap();
        writeln!(script,"$actual='Valid';$source='';$path='';\ntry{{{ns}\\Validator::validate({root_index},{ns}\\JsonValue::parse({}));}}catch({ns}\\ValidationError $error){{$actual=$error->kind==='invalid'?'Invalid':'EvaluationFailure';$source=substr($error->source,strrpos($error->source,'#')+1);$path=$error->instancePath;}}\ncheck($actual==={},{} . ': outcome ' . $actual);",literal(case["instanceJson"].as_str().unwrap()),literal(case["expected"].as_str().unwrap()),literal(name)).unwrap();
        if let Some(source) = case["source"].as_str() {
            writeln!(script,"check($source==={},{} . ': source ' . $source);check($path==={},{} . ': path ' . $path);",literal(source),literal(name),literal(case["instancePath"].as_str().unwrap_or_else(||instance_path(name))),literal(name)).unwrap();
        }
        writeln!(
            script,
            "$results[]=[{},$actual,$source,$path];",
            literal(name)
        )
        .unwrap();
        records.push(json!({"id":name,"schemaSource":id.document().as_str(),"schemaPointer":id.pointer(),"program":program,"instanceJson":case["instanceJson"],"expected":case["expected"],"source":case["source"],"instancePath":case["instancePath"].as_str().unwrap_or_else(||instance_path(name))}));
    }
    files.push(OutFile{path:"php/phpstan.neon".into(),content:"parameters:\n    level: max\n    phpVersion: 80300\n    treatPhpDocTypesAsCertain: false\n    paths: [src]\n    tmpDir: build/phpstan\n".into()});
    if let Some(extra) = extra {
        script.push_str(
            &extra
                .replace("__NS__", &format!("NativeV2\\C{}", cases.len() - 1))
                .replace(
                    "__ROOT__",
                    &records.last().unwrap()["program"]["roots"][0]["target"].to_string(),
                ),
        );
    }
    writeln!(script,"echo json_encode($results,JSON_THROW_ON_ERROR|JSON_UNESCAPED_UNICODE),PHP_EOL;\necho '{} source-driven scoped native vectors passed',PHP_EOL;",cases.len()).unwrap();
    fs::write(
        root.join("source-programs.json"),
        serde_json::to_string_pretty(&json!({"provenance":provenance,"cases":records})).unwrap(),
    )
    .unwrap();
    let consumer = install(root, &files, "php-validation-v2");
    fs::write(consumer.join("vectors.php"), script).unwrap();
    run(
        Command::new(php())
            .args(["-n", "-d", "error_reporting=-1", "vectors.php"])
            .current_dir(&consumer),
        root,
        "native-vectors",
    );
    run(
        Command::new(php())
            .arg("-n")
            .arg(phpstan())
            .args([
                "analyse",
                "--no-progress",
                "--memory-limit=2G",
                "--autoload-file",
            ])
            .arg(consumer.join("vendor/autoload.php"))
            .current_dir(consumer.join("vendor/fixture/php-validation-v2")),
        root,
        "package-types",
    );
    typecheck(root, &consumer, &["vectors.php"], "consumer-types");
    println!("PHP v2 corpus evidence: {}", root.display());
}

#[test]
#[ignore = "native scoped merge costs, key identity, recursion, cancellation and ownership"]
fn native_v2_scope_resource_controls() {
    let root = root("controls-");
    let merge = r#"{"allOf":[{"properties":{"a":true}},{"properties":{"a":true}}],"unevaluatedProperties":false}"#;
    let one = r#"{"oneOf":[{"properties":{"a":true}},{"properties":{"a":true}}],"unevaluatedProperties":false}"#;
    let recursive = r##"{"type":"object","properties":{"next":{"$ref":"#/components/schemas/Root"}},"unevaluatedProperties":false}"##;
    let mut nested = json!({});
    for _ in 0..20 {
        nested = json!({"next":nested});
    }
    let cases = vec![
        json!({"id":"merge-includes-duplicate-candidates","schemaJson":merge,"instanceJson":"{\"a\":0}","limits":{"maxEvaluationSteps":21},"expected":"Valid"}),
        json!({"id":"merge-exact-last-visit-failure","schemaJson":merge,"instanceJson":"{\"a\":0}","limits":{"maxEvaluationSteps":20},"expected":"EvaluationFailure","source":"/components/schemas/Root/unevaluatedProperties","instancePath":""}),
        json!({"id":"common-merge-is-charged","schemaJson":merge,"instanceJson":"{\"a\":0}","limits":{"maxEvaluationSteps":18},"expected":"EvaluationFailure","source":"/components/schemas/Root/allOf","instancePath":""}),
        json!({"id":"oneof-overlap-skips-union-work","schemaJson":one,"instanceJson":"{\"a\":0}","limits":{"maxEvaluationSteps":20},"expected":"Invalid","source":"/components/schemas/Root/oneOf","instancePath":""}),
        json!({"id":"mismatch-does-not-hide-late-failure","schemaJson":"{\"allOf\":[false,{\"contains\":{\"type\":\"integer\"}}]}","instanceJson":"[1,12345]","limits":{"maxNumberBytes":3},"expected":"EvaluationFailure","source":"/components/schemas/Root/allOf/1/contains/type","instancePath":"/1"}),
        json!({"id":"numeric-and-unicode-key-identity","schemaJson":"{\"properties\":{\"10\":true,\"2\":true,\"é\":true,\"雪\":true},\"unevaluatedProperties\":false}","instanceJson":"{\"雪\":null,\"é\":1,\"2\":2,\"10\":10}","expected":"Valid"}),
        json!({"id":"decoded-key-instance-path","schemaJson":"{\"propertyNames\":{\"maxLength\":1},\"unevaluatedProperties\":true}","instanceJson":"{\"~/a\":null}","expected":"Invalid","source":"/components/schemas/Root/propertyNames/maxLength","instancePath":"/~0~1a"}),
        json!({"id":"unicode-key-scalar-count","schemaJson":"{\"propertyNames\":{\"maxLength\":1},\"unevaluatedProperties\":true}","instanceJson":"{\"雪\":null}","expected":"Valid"}),
        json!({"id":"nonproductive-if-ref","schemaJson":"{\"if\":{\"$ref\":\"#/components/schemas/Root\"},\"then\":true}","instanceJson":"{}","expected":"EvaluationFailure","source":"/components/schemas/Root","instancePath":""}),
        json!({"id":"normal-stack-depth-failure","schemaJson":recursive,"instanceJson":nested.to_string(),"limits":{"maxDepth":12},"expected":"EvaluationFailure","source":"/components/schemas/Root","instancePath":"/next/next/next/next/next/next"}),
        json!({"id":"shared-equality-failure","schemaJson":"{\"if\":{\"enum\":[{\"a\":1}]},\"then\":true,\"else\":true}","instanceJson":"{\"a\":1}","limits":{"maxEqualitySteps":1},"expected":"EvaluationFailure","source":"/components/schemas/Root/if/enum","instancePath":""}),
        json!({"id":"ownership-control","schemaJson":"{\"properties\":{\"a\":true},\"unevaluatedProperties\":false}","instanceJson":"{\"a\":\"ok\"}","expected":"Valid"}),
    ];
    execute_vectors(
        &root,
        &cases,
        json!({"oracle":"independent PHP ownership/key/control vectors and exact SDK-SCHEMA-APPLICATORS.md visit counts"}),
        Some(CONTROL_CONSUMER),
    );
}
const CONTROL_CONSUMER: &str = r#"
final class StopValidation extends RuntimeException {}
$state=new class {public bool $stop=true;public int $checks=0;};
$control=new __NS__\CallControl(static function()use($state):void{if($state->stop&&++$state->checks===4){throw new StopValidation('cooperative stop');}});
$session=new __NS__\ValidationSession($control);$value=__NS__\JsonValue::parse('{"a":"ok"}');
try{$session->check(__ROOT__,$value);throw new RuntimeException('cancellation checkpoint missed');}catch(StopValidation $abort){check($state->checks===4,'cancel checkpoint count');}unset($abort);
$state->stop=false;$session->check(__ROOT__,$value);$session->check(__ROOT__,$value);
$weak=WeakReference::create($value);unset($value);check($weak->get()===null,'validation session retained instance ownership');
echo 'cooperative stop, failure cleanup, fresh scopes and released ownership passed',PHP_EOL;
"#;

#[test]
fn v1_program_bytes_and_explicit_profile_guards_are_preserved() {
    let root = root("admission-");
    let path = root.join("api.json");
    fs::write(&path,json!({"openapi":"3.1.0","info":{"title":"Admission","version":"1"},"components":{"schemas":{"Root":{"type":"object","required":["a"],"properties":{"a":{"type":"integer"}},"additionalProperties":false}}}}).to_string()).unwrap();
    let contract = load(&path);
    let id = SourceId::new(contract.entry().clone(), Default::default())
        .child("components")
        .child("schemas")
        .child("Root");
    let config = PhpConfig::default();
    let compiler = suspect_schema::OwnedCompiler::new(config.validation.clone());
    let old = compiler
        .compile(contract.clone(), std::slice::from_ref(&id))
        .unwrap()
        .program();
    let new = compiler
        .compile_v2(contract, std::slice::from_ref(&id))
        .unwrap()
        .program();
    assert_eq!(
        serde_json::to_vec(&old).unwrap(),
        serde_json::to_vec(&new).unwrap()
    );
    assert_eq!(
        emit::validation(&old, &config),
        emit::validation(&new, &config)
    );
    let mut mismatched = new.clone();
    mismatched.profile = "oas31-jsonschema202012-static-applicators";
    assert!(native_program_check_profile(&mismatched, true).is_err());
    let mut unknown = new;
    unknown.version = "future";
    assert!(native_program_check_profile(&unknown, true).is_err());
    fs::write(&path,json!({"openapi":"3.1.0","info":{"title":"Dynamic refusal","version":"1"},"components":{"schemas":{"Root":{"$dynamicRef":"#/components/schemas/Root"}}}}).to_string()).unwrap();
    let contract = load(&path);
    let dynamic = compiler
        .compile_v3(contract, std::slice::from_ref(&id))
        .unwrap()
        .program();
    assert_eq!(dynamic.check(), Ok(()));
    let finding = native_program_check_profile(&dynamic, true).unwrap_err();
    assert_eq!(
        finding.source.as_ref().unwrap().pointer,
        "/components/schemas/Root/$dynamicRef"
    );
    assert!(emit_validation(&dynamic, &config).is_ok());
    let mut malformed = dynamic;
    malformed
        .resource_context
        .as_mut()
        .unwrap()
        .node_scopes
        .pop();
    assert!(emit_validation(&malformed, &config).is_err());
    fs::write(root.join("source-fence.json"),serde_json::to_string_pretty(&json!({"v1ByteIdentity":true,"v3Finding":{"source":finding.source,"message":finding.message}})).unwrap()).unwrap();
}

#[test]
#[ignore = "real scoped SDK operations; installed Composer package, native types and docs"]
#[cfg(feature = "http-protocol")]
fn native_v2_sdk_packages_models_codecs() {
    let root = root("sdk-");
    let record = json!({"type":"object","required":["kind","name","nullable"],"properties":{
        "kind":{"type":"string","const":"record"},"name":{"type":"string","minLength":1},"nullable":{"type":["string","null"]},
        "enabled":{"type":"boolean"},"peer":{"type":"string"},"card":{"type":["string","null"]},"billing":{"type":"string"},"next":{"$ref":"#/components/schemas/RichRecord"}
    },"patternProperties":{"^x_":{"type":"integer"},"_positive$":{"minimum":0}},"additionalProperties":false,
    "dependentRequired":{"card":["billing"]},"dependentSchemas":{"enabled":{"required":["peer"],"properties":{"peer":{"minLength":2}}}},
    "if":{"properties":{"enabled":{"const":true}},"required":["enabled"]},"then":{"required":["peer"]},"else":{"properties":{"name":{"maxLength":20}}},
    "propertyNames":{"pattern":"^(kind|name|nullable|enabled|peer|card|billing|next|x_[a-z_]+)$"},
    "examples":[{"kind":"record","name":"missing nullable"},{"kind":"record","name":"example","nullable":null,"x_positive":1}]});
    let items = json!({"type":"array","prefixItems":[{"type":"string"}],"contains":{"type":"integer"},"minContains":1,"maxContains":2,"unevaluatedItems":false,"examples":[["tag",1]]});
    let envelope = json!({"if":{"type":"object"},"then":{"allOf":[{"properties":{"a":true}},{"properties":{"b":true}}]},"else":{"type":"string","minLength":1},"unevaluatedProperties":false,"examples":[{"a":1,"b":null},"text"]});
    let mut value = json!({"openapi":"3.1.0","info":{"title":"PHP native scoped operations","version":"1"},"servers":[{"url":"https://fixture.test"}],"components":{"schemas":{"RichRecord":record,"ScopedItems":items,"ConditionalJson":envelope}},"paths":{}});
    for (path, id, schema) in [
        ("/records", "upsertRecord", "RichRecord"),
        ("/batch", "checkBatch", "ScopedItems"),
        ("/opaque", "checkEnvelope", "ConditionalJson"),
    ] {
        let schema = json!({"$ref":format!("#/components/schemas/{schema}")});
        value["paths"][path] = json!({"post":{"operationId":id,"security":[],"requestBody":{"required":true,"content":{"application/json":{"schema":schema}}},"responses":{"200":{"description":"Source-validated echo","content":{"application/json":{"schema":schema}}}}}});
    }
    let path = root.join("api.json");
    fs::write(&path, value.to_string()).unwrap();
    let contract = load(&path);
    let selected = contract
        .operations()
        .map(|o| o.source().clone())
        .collect::<Vec<_>>();
    let config = PhpConfig {
        namespace: "ScopedSdk".into(),
        package_name: "fixture/php-v2-sdk".into(),
        package_version: "0.1.0".into(),
        ..Default::default()
    };
    let plan = protocol::plan_sdk(contract, &selected, config, protocol::capabilities()).unwrap();
    assert_eq!(plan.program().version, "suspect.validation.experimental.v2");
    let record = plan
        .models()
        .nodes
        .values()
        .find(|n| n.name == "RichRecord")
        .unwrap();
    assert!(matches!(
        &record.shape,
        models::Shape::Object {
            extras: models::Extras::Patterned(_),
            ..
        }
    ));
    assert_eq!(plan.examples().operations().len(), 3);
    assert!(
        plan.examples()
            .operations()
            .iter()
            .all(|o| !o.entries.is_empty()),
        "scoped examples must use the canonical v2 planner"
    );
    assert!(
        plan.examples()
            .diagnostics()
            .iter()
            .any(|d| d.source.pointer().contains("/examples/0")),
        "invalid declared scoped examples remain located findings"
    );
    let files = plan.render();
    let consumer = install(&root, &files, "php-v2-sdk");
    fs::write(
        root.join("checked-program.json"),
        serde_json::to_string_pretty(plan.program()).unwrap(),
    )
    .unwrap();
    let installed = consumer.join("vendor/fixture/php-v2-sdk");
    run(
        Command::new(php())
            .arg("-n")
            .arg(phpstan())
            .args([
                "analyse",
                "--no-progress",
                "--memory-limit=1G",
                "--autoload-file",
            ])
            .arg(consumer.join("vendor/autoload.php"))
            .current_dir(&installed),
        &root,
        "package-types",
    );
    for name in ["codecs", "client", "quickstart"] {
        run(
            Command::new(php())
                .args(["-n", "-d", "error_reporting=-1"])
                .arg(installed.join(format!("examples/{name}.php")))
                .env("SUSPECT_SDK_AUTOLOAD", consumer.join("vendor/autoload.php"))
                .current_dir(&consumer),
            &root,
            &format!("native-example-{name}"),
        );
    }
    fs::write(consumer.join("positive.php"), SDK_CONSUMER).unwrap();
    typecheck(&root, &consumer, &["positive.php"], "consumer-types");
    run(
        Command::new(php())
            .args(["-n", "-d", "error_reporting=-1", "positive.php"])
            .current_dir(&consumer),
        &root,
        "native-sdk",
    );
    fs::write(
        consumer.join("docs.php"),
        include_str!("native_protocol_docs.php"),
    )
    .unwrap();
    run(
        Command::new(php())
            .args(["-n", "-d", "error_reporting=-1", "docs.php"])
            .arg(&installed)
            .current_dir(&consumer),
        &root,
        "native-docs",
    );
    fs::write(consumer.join("negative.php"), SDK_NEGATIVE).unwrap();
    let output = Command::new(php())
        .arg("-n")
        .arg(phpstan())
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
    fs::write(root.join("negative-types.stderr"), &output.stderr).unwrap();
    assert_eq!(output.status.code(), Some(1));
    let errors: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(
        errors["totals"]["file_errors"].as_u64().unwrap() >= 4,
        "{errors}"
    );
    println!("PHP v2 SDK evidence: {}", root.display());
}
const SDK_CONSUMER: &str = r#"<?php
declare(strict_types=1);
require __DIR__.'/vendor/autoload.php';
use ScopedSdk as S;
function check(bool $ok):void{if(!$ok){throw new RuntimeException('scoped SDK mismatch');}}
$transport=new class implements S\Transport {
    public int $calls=0;public ?string $override=null;public ?string $last=null;
    public function send(S\HttpRequest $request):S\HttpResponse{++$this->calls;$this->last=$request->body;return new S\HttpResponse(200,['Content-Type'=>'application/json'],$this->override??$request->body??throw new RuntimeException('missing request bytes'));}
};
$client=new S\Client(new S\Credentials([]),$transport);
$record=new S\RichRecord(name:'native',nullable:null,extra:['x_positive'=>S\JsonValue::fromNumber(S\JsonNumber::fromString('9007199254740993'))]);
$response=$client->upsertRecord(new S\UpsertRecordInput(body:$record));check($response->body->nullable===null&&$response->body->card===S\Absent::Value);check($response->body->extra['x_positive']->asNumber()->token==='9007199254740993');check(str_contains($record->toJson(),'"kind":"record"'));check((new ReflectionProperty(S\RichRecord::class,'kind'))->isReadOnly());
$record->card=null;
try{$client->upsertRecord(new S\UpsertRecordInput(body:$record));throw new RuntimeException('null presence was lost');}catch(S\SdkError $error){check($error->kind==='request_validation');}check($transport->calls===1);
$record->billing='present';$client->upsertRecord(new S\UpsertRecordInput(body:$record));check(str_contains($transport->last??'', '"card":null'));
$record->enabled=true;
try{$client->upsertRecord(new S\UpsertRecordInput(body:$record));throw new RuntimeException('dependent schema did not see whole object');}catch(S\SdkError $error){check($error->kind==='request_validation');}
$record->peer='ok';$client->upsertRecord(new S\UpsertRecordInput(body:$record));
$record->extra['x_positive']=S\JsonValue::fromNumber(S\JsonNumber::fromInt(-1));
try{$record->toJson();throw new RuntimeException('overlapping pattern missed');}catch(S\ValidationError $error){check($error->kind==='invalid'&&str_contains($error->source,'patternProperties'));}
$record->extra['x_positive']=S\JsonValue::fromString('not an integer');
try{$record->toJson();throw new RuntimeException('pattern extra type missed');}catch(S\ValidationError $error){check($error->kind==='invalid');}
$record->extra=['unknown'=>S\JsonValue::null()];try{$record->toJson();throw new RuntimeException('additional member accepted');}catch(S\ValidationError $error){check($error->kind==='invalid');}
$record->extra=[];$record->next=$record;try{$record->toJson();throw new RuntimeException('native object cycle accepted');}catch(S\JsonError $error){check($error->kind==='conversion');}$record->next=S\Absent::Value;
$client->upsertRecord(new S\UpsertRecordInput(body:$record));
$list=[S\JsonValue::fromString('tag'),S\JsonValue::fromNumber(S\JsonNumber::fromString('1e0'))];$batch=$client->checkBatch(new S\CheckBatchInput(body:$list));check($batch->body[1]->asNumber()->token==='1e0');
$list[]=S\JsonValue::fromNumber(S\JsonNumber::fromString('1e-400'));try{$client->checkBatch(new S\CheckBatchInput(body:$list));throw new RuntimeException('unmatched item was marked');}catch(S\SdkError $error){check($error->kind==='request_validation');}
$object=S\JsonValue::fromObject(['a'=>S\JsonValue::fromNumber(S\JsonNumber::fromInt(1)),'b'=>S\JsonValue::null()]);check($client->checkEnvelope(new S\CheckEnvelopeInput(body:$object))->body->toJson()===$object->toJson());
check($client->checkEnvelope(new S\CheckEnvelopeInput(body:S\JsonValue::fromString('text')))->body->asString()==='text');
try{$client->checkEnvelope(new S\CheckEnvelopeInput(body:S\JsonValue::fromObject(['extra'=>S\JsonValue::null()])));throw new RuntimeException('unevaluated member leaked');}catch(S\SdkError $error){check($error->kind==='request_validation');}
$transport->override='{"kind":"record","name":"bad","nullable":null,"card":null}';
try{$client->upsertRecord(new S\UpsertRecordInput(body:$record));throw new RuntimeException('invalid response decoded');}catch(S\SdkError $error){check($error->kind==='response_validation');}
echo 'native scoped operations, typed fields, pattern extras, null/omission and revalidation passed',PHP_EOL;
"#;
const SDK_NEGATIVE: &str = r#"<?php
declare(strict_types=1);
require __DIR__.'/vendor/autoload.php';
use ScopedSdk as S;
new S\RichRecord(name:1,nullable:null);
new S\RichRecord(name:'missing nullable');
new S\RichRecord(name:'bad extras',nullable:null,extra:['x_positive'=>1]);
new S\CheckBatchInput(body:['tag',1]);
new S\CheckEnvelopeInput(body:new stdClass());
"#;
