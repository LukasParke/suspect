//! Generated program loading scales independently of per-request JSON limits.
#![cfg(feature = "kotlin-sdk")]
use serde_json::{Map, Value, json};
use std::{process::Command, sync::Arc};
use suspect_codegen::kotlin_sdk;
use suspect_ir::contract::Contract;
use suspect_ref::WorkspaceBuilder;
use suspect_schema::OwnedProgram;
use suspect_source::Uri;

fn sdk_plan(operation_count: usize) -> kotlin_sdk::Plan {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("codec-scale.json");
    let properties = (0..80)
        .map(|index| {
            (
                format!("field{index:03}"),
                json!({"type":"string","minLength":1}),
            )
        })
        .collect::<Map<String, Value>>();
    let paths = (0..operation_count).map(|index| (format!("/value-{index}"), json!({"get":{
        "operationId":format!("getValue{index}"),
        "responses":{"200":{"description":"Value","content":{"application/json":{"schema":{
            "type":"object","properties":properties
        }}}}}
    }}))).collect::<Map<String, Value>>();
    std::fs::write(&path, json!({"openapi":"3.2.0","info":{"title":"Codec scale","version":"1"},"servers":[{"url":"https://example.test"}],"paths":paths}).to_string()).unwrap();
    let workspace = Arc::new(
        WorkspaceBuilder::new()
            .root(directory.path())
            .build()
            .unwrap(),
    );
    let contract =
        Arc::new(Contract::from_workspace(&workspace, &Uri::from_path(&path).unwrap()).unwrap());
    let selected = contract
        .operations()
        .map(|operation| operation.source().clone())
        .collect::<Vec<_>>();
    kotlin_sdk::plan_sdk(
        contract,
        &selected,
        kotlin_sdk::SdkConfig {
            package_name: "example.sdk".into(),
            ..Default::default()
        },
    )
    .unwrap()
}

#[test]
fn codec_groups_preserve_every_source_bound_public_member() {
    let plan = sdk_plan(3);
    let files = plan.render().unwrap();
    let groups = files
        .iter()
        .filter(|file| file.content.contains("public sealed class CodecGroup"))
        .collect::<Vec<_>>();
    assert!(groups.len() > 1);
    assert_eq!(
        groups
            .iter()
            .map(|file| file.content.matches("public val ").count())
            .sum::<usize>(),
        plan.models().symbols().len()
    );
    let facade = files
        .iter()
        .find(|file| file.path.ends_with("/Codecs.kt"))
        .unwrap();
    assert!(facade.content.contains("public object Codecs : CodecGroup"));
}

#[test]
#[ignore = "requires Kotlin Maven toolchain and cached dependencies"]
fn native_thousands_of_codecs_compile_and_validate_through_the_public_facade() {
    let plan = sdk_plan(64);
    assert!(plan.models().symbols().len() > 5_000);
    let directory = tempfile::tempdir().unwrap();
    suspect_codegen::write_files(&plan.render().unwrap(), directory.path()).unwrap();
    let root = directory.path().join("kotlin");
    let scalars = plan
        .models()
        .symbols()
        .iter()
        .filter(|symbol| symbol.kotlin_type == "String")
        .collect::<Vec<_>>();
    let mut source = String::from(
        "package example.sdk\npublic object CodecScale { @JvmStatic public fun main(args: Array<String>) {\n",
    );
    for symbol in [
        scalars[0],
        scalars[scalars.len() / 2],
        scalars[scalars.len() - 1],
    ] {
        source.push_str(&format!("check(Codecs.{}.decode(\"\\\"valid\\\"\") == \"valid\")\ntry {{ Codecs.{}.encode(\"\"); error(\"constraint lost\") }} catch (_: ValidationException) {{}}\n",symbol.codec_name,symbol.codec_name));
    }
    source.push_str("println(\"partitioned codecs retain inherited native access and source assertions\")\n} }\n");
    std::fs::write(
        root.join("src/test/kotlin/example/sdk/CodecScale.kt"),
        source,
    )
    .unwrap();
    let mut command =
        Command::new(std::env::var_os("SUSPECT_KOTLIN_MAVEN").unwrap_or_else(|| "mvn".into()));
    command
        .args([
            "-B",
            "--no-transfer-progress",
            "-o",
            "test",
            "exec:java",
            "-Dexec.classpathScope=test",
            "-Dexec.mainClass=example.sdk.CodecScale",
        ])
        .env("MAVEN_OPTS", "-Xmx4096m -Dfile.encoding=UTF-8")
        .current_dir(&root);
    if let Some(repository) = std::env::var_os("SUSPECT_KOTLIN_MAVEN_REPO") {
        command.arg(format!(
            "-Dmaven.repo.local={}",
            repository.to_string_lossy()
        ));
    }
    if let Some(home) = std::env::var_os("SUSPECT_KOTLIN_JAVA_HOME") {
        command.env("JAVA_HOME", home);
    }
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn program() -> OwnedProgram {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("large-api.json");
    let schemas = (0..6_000)
        .map(|index| {
            (
                format!("AnIndependentSourceBoundModelWithItsOwnValidationIdentity{index}"),
                json!({"type":"string","minLength":1,"maxLength":64}),
            )
        })
        .collect::<Map<String, Value>>();
    std::fs::write(&path, json!({"openapi":"3.2.0","info":{"title":"Large graph","version":"1"},"paths":{},"components":{"schemas":schemas}}).to_string()).unwrap();
    let workspace = Arc::new(
        WorkspaceBuilder::new()
            .root(directory.path())
            .build()
            .unwrap(),
    );
    let contract =
        Arc::new(Contract::from_workspace(&workspace, &Uri::from_path(&path).unwrap()).unwrap());
    kotlin_sdk::validation::plan_validation(
        contract.clone(),
        contract.schema_roots(),
        suspect_schema::Config {
            max_depth: 128,
            ..Default::default()
        },
    )
    .unwrap()
}

#[test]
fn large_program_metadata_retains_per_value_evaluation_limits() {
    let program = program();
    assert_eq!(program.nodes.len(), 6_000);
    assert!(serde_json::to_vec(&program).unwrap().len() > 4 * 1024 * 1024);
    assert_eq!(program.limits.max_evaluation_steps, 100_000);
    assert_eq!(program.limits.max_depth, 128);
    assert!(kotlin_sdk::validation::emit_validation(&program, "example.sdk").is_ok());
}

#[test]
#[ignore = "requires Kotlin Maven toolchain and cached dependencies"]
fn native_large_program_loads_while_payload_and_program_bounds_remain_distinct() {
    let program = program();
    let directory = tempfile::tempdir().unwrap();
    let files = kotlin_sdk::validation::emit_validation(&program, "example.sdk").unwrap();
    suspect_codegen::write_files(&files, directory.path()).unwrap();
    let root = directory.path().join("kotlin");
    let mut pom = include_str!("../src/kotlin_sdk/pom.xml").to_owned();
    for (key, value) in [
        ("GROUP", "example"),
        ("ARTIFACT", "program-scale"),
        ("VERSION", "0.1.0"),
        ("PACKAGE", "example.sdk"),
        ("KOTLIN", kotlin_sdk::KOTLIN_VERSION),
        ("COROUTINES", kotlin_sdk::COROUTINES_VERSION),
        ("DOKKA", kotlin_sdk::DOKKA_VERSION),
    ] {
        pom = pom.replace(&format!("__{key}__"), value);
    }
    std::fs::write(root.join("pom.xml"), pom).unwrap();
    std::fs::write(
        root.join("src/main/kotlin/example/sdk/ProgramScale.kt"),
        r#"package example.sdk
public object ProgramScale {
    @JvmStatic public fun main(args: Array<String>) {
        check(ValidationProgram.nodes.size == 6_000)
        val target = ValidationProgram.roots.values.max()
        ValidationSession(CodecLimits()) {}.validate(target, JsonString("valid"), "")
        try {
            ValidationSession(CodecLimits()) {}.validate(target, JsonString(""), "")
            error("large program lost its assertions")
        } catch (_: ValidationException) {}
        try {
            Json.parse(ByteArray(Json.MAX_BYTES + 1))
            error("payload limit widened")
        } catch (error: JsonException) { check(error.kind == JsonErrorKind.RESOURCE_LIMIT) }
        try {
            Json.parseProgram(ByteArray(Json.MAX_PROGRAM_BYTES + 1))
            error("program limit missing")
        } catch (error: JsonException) { check(error.kind == JsonErrorKind.RESOURCE_LIMIT) }
        println("large program loaded and independent byte/validation limits enforced")
    }
}
"#,
    )
    .unwrap();
    let mut command =
        Command::new(std::env::var_os("SUSPECT_KOTLIN_MAVEN").unwrap_or_else(|| "mvn".into()));
    command
        .args([
            "-B",
            "--no-transfer-progress",
            "-o",
            "compile",
            "exec:java",
            "-Dexec.mainClass=example.sdk.ProgramScale",
        ])
        .current_dir(&root);
    if let Some(repository) = std::env::var_os("SUSPECT_KOTLIN_MAVEN_REPO") {
        command.arg(format!(
            "-Dmaven.repo.local={}",
            repository.to_string_lossy()
        ));
    }
    if let Some(home) = std::env::var_os("SUSPECT_KOTLIN_JAVA_HOME") {
        command.env("JAVA_HOME", home);
    }
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
