use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    process::Command,
};
use suspect_codegen::kotlin_sdk::{
    Plan, PlannedHeader,
    models::{Additional, Shape},
};

pub fn java_homes() -> Vec<(String, PathBuf)> {
    if let Some(home) = std::env::var_os("SUSPECT_KOTLIN_JAVA_HOME") {
        return vec![("override".into(), home.into())];
    }
    [
        ("21", "temurin-21.0.12+101.0.LTS"),
        ("25", "temurin-25.0.4+101.0.LTS"),
    ]
    .into_iter()
    .map(|(version, name)| {
        (
            version.into(),
            Path::new("/Users/luke/.local/share/mise/installs/java").join(name),
        )
    })
    .collect()
}

pub fn maven(home: &Path) -> Command {
    let binary = std::env::var_os("SUSPECT_KOTLIN_MAVEN")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            "/Users/luke/.local/share/mise/installs/maven/3.9.16/apache-maven-3.9.16/bin/mvn".into()
        });
    let repo = std::env::var_os("SUSPECT_KOTLIN_MAVEN_REPO")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/sdk-kotlin-maven")
        });
    std::fs::create_dir_all(&repo).unwrap();
    let mut command = Command::new(binary);
    command
        .args(["-B", "--no-transfer-progress"])
        .arg(format!(
            "-Dmaven.repo.local={}",
            repo.canonicalize().unwrap().display()
        ))
        .env("JAVA_HOME", home)
        .env("MAVEN_OPTS", "-Xmx2048m -Dfile.encoding=UTF-8");
    command
}

pub fn installed_quickstart(root: &Path, rich_guide: bool) {
    let readme = std::fs::read_to_string(root.join("kotlin/README.md")).unwrap();
    let snippet = readme
        .split("```kotlin\n")
        .skip(1)
        .map(|part| part.split("\n```").next().unwrap())
        .find(|part| part.contains("public object Quickstart"))
        .expect("fixture requires an executable README");
    assert!(
        !snippet.contains("Codecs."),
        "readable constructors are required"
    );
    std::fs::write(
        root.join("consumer/src/main/kotlin/Quickstart.kt"),
        format!("package consumer\n{snippet}\n"),
    )
    .unwrap();
    if !rich_guide {
        return;
    }
    let docs = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/SDK-KOTLIN.md"),
    )
    .unwrap();
    let snippets = docs
        .split("```kotlin\n")
        .skip(1)
        .map(|p| p.split("\n```").next().unwrap())
        .filter(|p| p.contains("import example.protocol.*"))
        .collect::<Vec<_>>();
    let imports = snippets
        .iter()
        .flat_map(|s| s.lines())
        .filter(|l| l.starts_with("import "))
        .collect::<BTreeSet<_>>();
    let body = snippets
        .iter()
        .flat_map(|s| s.lines())
        .filter(|l| !l.starts_with("import "))
        .collect::<Vec<_>>()
        .join("\n");
    std::fs::write(
        root.join("consumer/src/main/kotlin/ProtocolGuide.kt"),
        format!(
            "package consumer\n{}\n{body}\n",
            imports.into_iter().collect::<Vec<_>>().join("\n")
        ),
    )
    .unwrap();
}

pub fn native_docs(plan: &Plan, root: &Path, version: &str) {
    let mut expected = BTreeSet::new();
    let mut add = |name: String| {
        expected.insert(format!("{}.{name}", plan.config().package_name));
    };
    for name in [
        "Client",
        "Credentials",
        "BasicCredentials",
        "BasicCredentials.username",
        "BasicCredentials.password",
        "CredentialProvider",
        "CredentialProvider.authorization",
        "CredentialContext",
        "CredentialContext.operationId",
        "CredentialContext.scheme",
        "CredentialContext.scopes",
        "CredentialContext.roles",
        "CredentialContext.source",
        "CredentialContext.metadata",
        "CredentialContext.effectiveServer",
        "Upload",
        "Upload.data",
        "Upload.filename",
        "Upload.contentType",
        "LinkMetadata",
        "LinkMetadata.name",
        "LinkMetadata.declaration",
        "Transport",
        "Transport.execute",
        "StreamingTransport",
        "StreamingTransport.open",
        "StreamingTransport.execute",
        "StreamingResponse",
        "StreamingResponse.status",
        "StreamingResponse.headers",
        "StreamingResponse.body",
        "BodyReader",
        "BodyReader.read",
        "BodyReader.close",
        "JdkTransport",
        "JdkTransport.open",
        "ResponseInfo",
        "ResponseInfo.contentType",
        "ResponseInfo.links",
        "ClientOptions",
        "ClientOptions.serverIndex",
        "ClientOptions.serverVariables",
        "ClientOptions.documentUrl",
        "ClientOptions.maxStreamItemBytes",
        "ClientOptions.maxChunkBytes",
        "RequestOptions",
        "RequestOptions.timeout",
        "RequestOptions.serverIndex",
        "RequestOptions.serverVariables",
        "RequestOptions.documentUrl",
        "RequestOptions.securityAlternative",
        "RequestOptions.requestMedia",
        "RequestOptions.responseMedia",
        "JsonNumber",
        "JsonValue",
        "JsonObject",
        "JsonArray",
        "Presence",
        "Presence.Present",
        "Presence.Absent",
        "ModelCodec",
        "CodecLimits",
        "SourceLocation",
        "ValidationException",
        "EvaluationException",
        "SdkException",
        "ApiException",
    ] {
        add(name.into());
    }
    for c in plan.credentials().values() {
        add(format!("Credentials.{}", c.name));
    }
    if plan.credential_env().is_some() {
        for name in [
            "CredentialEnvironment",
            "CredentialEnvironment.value",
            "CredentialEnvironment.Companion.system",
            "Client.Companion.fromEnv",
        ] {
            add(name.into());
        }
    }
    fn headers(name: &str, fields: &[PlannedHeader], add: &mut impl FnMut(String)) {
        add(name.into());
        for h in fields {
            add(format!("{name}.{}", h.name));
        }
    }
    for op in plan.operations() {
        add(format!("Client.{}", op.method_name));
        for name in [&op.input_type, &op.result_type, &op.error_type] {
            add(name.clone());
        }
        for p in &op.parameters {
            add(format!("{}.{}", op.input_type, p.name));
        }
        if let Some(body) = &op.body {
            add(format!("{}.body", op.input_type));
            if let Some(name) = &body.choice_type {
                add(name.clone());
                for media in &body.media {
                    add(format!("{name}.{}", media.name));
                    add(format!("{name}.{}.value", media.name));
                }
            }
        }
        if op.result_data_type.is_some() {
            add(format!("{}.data", op.result_type));
        }
        for r in &op.responses {
            add(r.constructor.clone());
            add(format!("{}.data", r.constructor));
            if let Some(name) = &r.headers_type {
                add(format!("{}.responseHeaders", r.constructor));
                headers(name, &r.headers, &mut add);
            }
        }
        for media in op
            .body
            .iter()
            .flat_map(|b| b.media.iter())
            .chain(op.responses.iter().filter_map(|r| r.media.as_ref()))
        {
            if let Some(form) = &media.form {
                add(form.name.clone());
                for p in &form.fields {
                    add(format!("{}.{}", form.name, p.name));
                }
                if form.additional.is_some() {
                    add(format!("{}.additionalProperties", form.name));
                }
                for p in form
                    .fields
                    .iter()
                    .chain(form.additional.iter().map(|p| p.as_ref()))
                {
                    if let Some(name) = &p.headers_type {
                        headers(name, &p.headers, &mut add);
                    }
                    if let Some(name) = &p.wrapper {
                        add(name.clone());
                        add(format!("{name}.value"));
                        add(format!("{name}.contentType"));
                        if p.headers_type.is_some() {
                            add(format!("{name}.headers"));
                        }
                    }
                }
            }
        }
    }
    for s in plan.models().symbols() {
        add(format!("Codecs.{}", s.codec_name));
        match &s.shape {
            Shape::CheckedJson => {
                add(s.name.clone());
                add(format!("{}.value", s.name));
            }
            Shape::Object { fields, additional } => {
                add(s.name.clone());
                for f in fields {
                    add(format!("{}.{}", s.name, f.name));
                }
                if !matches!(additional, Additional::Closed) {
                    add(format!("{}.additionalProperties", s.name));
                }
            }
            Shape::StringEnum(cases) => {
                add(s.name.clone());
                for (c, _) in cases {
                    add(format!("{}.{c}", s.name));
                }
            }
            Shape::Union { variants, .. } => {
                add(s.name.clone());
                for (c, _) in variants {
                    add(format!("{}.{c}", s.name));
                }
            }
            _ => {}
        }
    }
    let docs = root.join("kotlin/target/dokka");
    let pages: Vec<Value> =
        serde_json::from_slice(&std::fs::read(docs.join("scripts/pages.json")).unwrap()).unwrap();
    let mut locations = Vec::new();
    for symbol in &expected {
        let page = pages
            .iter()
            .find(|p| p["description"].as_str() == Some(symbol))
            .unwrap_or_else(|| panic!("missing Dokka symbol {symbol}: {}", root.display()));
        let location = page["location"].as_str().unwrap();
        let html = std::fs::read_to_string(docs.join(location.split('#').next().unwrap())).unwrap();
        assert!(
            html.contains("id=\"content\"") && html.contains("class=\"paragraph\""),
            "missing rendered KDoc for {symbol}"
        );
        locations.push(json!({"symbol":symbol,"page":location}));
    }
    std::fs::write(
        root.join(format!("dokka-coverage-{version}.json")),
        serde_json::to_string_pretty(
            &json!({"expected":expected.len(),"missing":[],"pages":locations}),
        )
        .unwrap(),
    )
    .unwrap();
}
