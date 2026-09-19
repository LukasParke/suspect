#![cfg(all(feature = "kotlin-sdk", feature = "http-protocol"))]
#![recursion_limit = "512"]
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};
use suspect_codegen::kotlin_sdk::{self, Plan, SdkConfig};
use suspect_ir::contract::Contract;
use suspect_ref::WorkspaceBuilder;
use suspect_source::Uri;

#[path = "kotlin_support/mod.rs"]
mod support;

fn root() -> PathBuf {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/sdk-kotlin-protocol");
    std::fs::create_dir_all(&base).unwrap();
    tempfile::Builder::new()
        .prefix("gate-")
        .tempdir_in(base)
        .unwrap()
        .keep()
        .canonicalize()
        .unwrap()
}
fn response(media: &str, schema: Value) -> Value {
    json!({"description":"Declared fixture response","content":{media:{"schema":schema}}})
}
fn reply() -> Value {
    response(
        "application/json",
        json!({"$ref":"#/components/schemas/Reply"}),
    )
}
fn fixture() -> Value {
    let file_schema = json!({"type":"object","required":["file","title","meta"],"additionalProperties":false,"minProperties":3,"maxProperties":4,
        "properties":{"file":{},"title":{"type":"string","example":"title"},"meta":{"type":"object","required":["flag"],"properties":{"flag":{"type":"boolean","example":true}}},"chunks":{"type":"array","items":{}}}});
    let file_media = json!({"schema":file_schema,"encoding":{"file":{"contentType":"application/octet-stream","headers":{"X-Part":{"required":true,"schema":{"type":"integer","example":7}}}},"meta":{"contentType":"application/json"},"chunks":{"contentType":"application/octet-stream"}}});
    let form_media = json!({"schema":{"type":"object","required":["name","tags","config"],"additionalProperties":{"type":"string"},"properties":{"name":{"type":"string","example":"from wire"},"tags":{"type":"array","items":{"type":"string","example":"tag"}},"config":{"type":"object","required":["ok"],"properties":{"ok":{"type":"boolean","example":true}}}}},"encoding":{"config":{"contentType":"application/json"}}});
    json!({"openapi":"3.2.0","info":{"title":"Kotlin protocol gate","version":"1"},"servers":[{"url":"https://example.test/api"}],"security":[],
        "components":{"securitySchemes":{
            "token":{"type":"http","scheme":"bearer"},"basicAuth":{"type":"http","scheme":"basic"},
            "queryKey":{"type":"apiKey","in":"query","name":"key"},"headerKey":{"type":"apiKey","in":"header","name":"X-Key"},"cookieKey":{"type":"apiKey","in":"cookie","name":"session"},
            "oauth":{"type":"oauth2","flows":{"authorizationCode":{"authorizationUrl":"https://identity.test/authorize","tokenUrl":"https://identity.test/token","scopes":{"read:items":"Read items"}}}},
            "oidc":{"type":"openIdConnect","openIdConnectUrl":"https://identity.test/.well-known/openid-configuration"}},
            "schemas":{"Event":{"type":"object","required":["data"],"additionalProperties":false,"properties":{"data":{"type":"string","example":"send"},"event":{"type":"string"},"id":{"type":"string"},"retry":{"type":"integer","minimum":0}}},"Reply":{"type":"object","required":["name","value"],"properties":{"name":{"type":"string","example":"ok"},"value":{"type":"number","example":1}}},"Input":{"type":"object","required":["name"],"properties":{"name":{"type":"string","example":"input"}}},"Failure":{"type":"object","required":["message"],"properties":{"message":{"type":"string","example":"denied"}}}}},
        "paths":{
            "/anonymous":{"get":{"operationId":"anonymous","responses":{"200":{"description":"Reply","headers":{"X-Count":{"required":true,"schema":{"type":"integer","example":7}},"X-Json":{"required":true,"content":{"application/json":{"schema":{"type":"object","required":["ok"],"properties":{"ok":{"type":"boolean","example":true}}}}}}},"links":{"text":{"operationId":"textValue","parameters":{"value":"$response.body#/name"},"description":"link metadata"}},"content":{"application/json":{"schema":{"$ref":"#/components/schemas/Reply"}}}}}}},
            "/secure":{"get":{"operationId":"secure","security":[{"token":[],"queryKey":[]},{"basicAuth":[]},{}],"responses":{"200":reply()}}},
            "/basic":{"get":{"operationId":"basic","security":[{"basicAuth":[]}],"responses":{"200":reply()}}},
            "/oauth":{"get":{"operationId":"oauthCall","security":[{"oauth":["read:items"]}],"responses":{"200":reply()}}},
            "/oidc":{"get":{"operationId":"oidcCall","security":[{"oidc":[]}],"responses":{"200":reply()}}},
            "/params/{id}":{"get":{"operationId":"parameters","security":[{"headerKey":[],"cookieKey":[]}],"parameters":[
                {"name":"id","in":"path","required":true,"style":"label","explode":true,"schema":{"type":"array","items":{"type":"string"},"example":["one","two"]}},
                {"name":"where","in":"query","style":"deepObject","explode":true,"schema":{"type":"object","required":["name"],"additionalProperties":false,"properties":{"name":{"type":"string","example":"field"},"count":{"type":"integer"}}}},
                {"name":"X-Numbers","in":"header","schema":{"type":"array","items":{"type":"integer"}}},
                {"name":"p","in":"cookie","style":"cookie","schema":{"type":"string"}},
                {"name":"reserved","in":"query","allowReserved":true,"schema":{"type":"string"}},
                {"name":"filter","in":"query","content":{"application/json":{"schema":{"type":"object","required":["count"],"properties":{"count":{"type":"integer","example":2}}}}}}
            ],"responses":{"200":reply()}}},
            "/matrix/{value}":{"get":{"operationId":"matrix","parameters":[{"name":"value","in":"path","required":true,"style":"matrix","explode":true,"schema":{"type":"array","items":{"type":"string"},"example":["a","b"]}},{"name":"pipe","in":"query","style":"pipeDelimited","explode":false,"schema":{"type":"array","items":{"type":"string"}}},{"name":"space","in":"query","style":"spaceDelimited","explode":false,"schema":{"type":"array","items":{"type":"string"}}}],"responses":{"200":reply()}}},
            "/choose":{"post":{"operationId":"choose","parameters":[{"name":"case","in":"query","schema":{"type":"string"}}],"requestBody":{"required":true,"content":{"application/json":{"schema":{"$ref":"#/components/schemas/Input"}},"text/plain":{"schema":{"type":"string","example":"text"}},"application/octet-stream":{}}},"responses":{"200":reply(),"2XX":response("text/plain",json!({"type":"string","example":"range"})),"default":{"description":"Default bytes","content":{"application/octet-stream":{}}}}}},
            "/fallback":{"get":{"operationId":"fallback","parameters":[{"name":"code","in":"query","schema":{"type":"integer"}}],"responses":{"default":reply()}}},
            "/media":{"get":{"operationId":"media","parameters":[{"name":"case","in":"query","schema":{"type":"string"}}],"responses":{"200":{"description":"Media precedence","content":{"application/json; profile=one":{"schema":{"$ref":"#/components/schemas/Reply"}},"application/*":{},"text/*":{},"*/*":{}}}}}},
            "/text":{"post":{"operationId":"textValue","requestBody":{"required":true,"content":{"text/plain":{"schema":{"type":"integer","example":123}}}},"responses":{"200":response("text/plain",json!({"type":"integer","example":123}))}}},
            "/form":{"post":{"operationId":"form","requestBody":{"required":true,"content":{"application/x-www-form-urlencoded":{"schema":{"type":"object","required":["name","config","tags"],"additionalProperties":false,"properties":{"name":{"type":"string","example":"name"},"config":{"type":"object","required":["ok"],"properties":{"ok":{"type":"boolean","example":true}}},"tags":{"type":"array","items":{"type":"string","example":"tag"}}}},"encoding":{"config":{"contentType":"application/json"}}}}},"responses":{"200":reply()}}},
            "/form-reply":{"get":{"operationId":"formReply","responses":{"200":{"description":"form values","content":{"application/x-www-form-urlencoded":form_media}}}}},
            "/typed-headers":{"get":{"operationId":"typedHeaders","responses":{"200":{"description":"typed headers","headers":{"X-List":{"required":true,"schema":{"type":"array","items":{"type":"integer"},"example":[1,2]}},"X-Object":{"required":true,"explode":true,"schema":{"type":"object","required":["active"],"additionalProperties":{"type":"integer"},"properties":{"active":{"type":"boolean"}},"example":{"active":true,"count":2}}},"X-Text":{"required":true,"content":{"text/plain":{"schema":{"type":"integer","example":3}}}}},"content":{"application/json":{"schema":{"$ref":"#/components/schemas/Reply"}}}}}}},
            "/upload":{"post":{"operationId":"upload","requestBody":{"required":true,"content":{"multipart/form-data":file_media}},"responses":{"200":reply()}}},
            "/multipart":{"get":{"operationId":"multipartReply","responses":{"200":{"description":"Multipart reply","content":{"multipart/form-data":file_media}}}}},
            "/head":{"head":{"operationId":"head","responses":{"200":reply()}}},
            "/none":{"get":{"operationId":"noDeclaredBody","responses":{"200":{"description":"Unspecified body"},"204":{"description":"No content"}}}},
            "/events":{"get":{"operationId":"events","responses":{"200":{"description":"SSE","content":{"text/event-stream":{"itemSchema":{"type":"object","required":["data"],"additionalProperties":false,"properties":{"data":{"type":"string","example":"hello"},"event":{"type":"string"},"id":{"type":"string"},"retry":{"type":"integer","minimum":0}}}}}},"400":response("application/json",json!({"$ref":"#/components/schemas/Failure"}))}}},
            "/lines":{"get":{"operationId":"lines","responses":{"200":{"description":"JSON lines","content":{"application/x-ndjson":{"itemSchema":{"$ref":"#/components/schemas/Reply"}}}}}}},
            "/mixed":{"get":{"operationId":"mixed","responses":{"200":{"description":"JSON or streaming items","content":{"application/json":{"schema":{"$ref":"#/components/schemas/Reply"}},"application/x-ndjson":{"itemSchema":{"$ref":"#/components/schemas/Reply"}}}}}}},
            "/undocumented":{"get":{"operationId":"undocumented"}},
            "/wildcard":{"post":{"operationId":"wildcard","requestBody":{"required":true,"content":{"*/*":{},"application/json":{"schema":{"$ref":"#/components/schemas/Input"}}}},"responses":{"200":reply()}}},
            "/post-events":{"post":{"operationId":"postEvents","requestBody":{"required":true,"content":{"text/event-stream":{"itemSchema":{"$ref":"#/components/schemas/Event"}}}},"responses":{"200":reply()}}},
            "/post-lines":{"post":{"operationId":"postLines","requestBody":{"required":true,"content":{"application/x-ndjson":{"itemSchema":{"$ref":"#/components/schemas/Reply"}}}},"responses":{"200":reply()}}},
            "/structured":{"patch":{"operationId":"structured","requestBody":{"required":true,"content":{"application/merge-patch+json":{"schema":{"$ref":"#/components/schemas/Input"}}}},"responses":{"200":response("application/problem+json",json!({"$ref":"#/components/schemas/Reply"}))}}},
            "/schema-free":{"post":{"operationId":"schemaFree","requestBody":{"required":true,"content":{"application/json":{}}},"responses":{"200":{"description":"JSON without schema","content":{"application/json":{}}}}}},
            "/servers":{"get":{"operationId":"servers","servers":[{"url":"https://unused.example/{version}","variables":{"version":{"default":"api"}}},{"url":"../{version}","variables":{"version":{"default":"api","enum":["api","other"]}}}],"responses":{"200":reply()}}},
            "/custom":{"additionalOperations":{"REPORT":{"operationId":"custom","responses":{"200":reply()}}}},
            "/search":{"get":{"operationId":"search","parameters":[{"name":"query","in":"querystring","required":true,"content":{"application/x-www-form-urlencoded":{"schema":{"type":"object","additionalProperties":false,"required":["a","z"],"properties":{"a":{"type":"integer","example":1},"z":{"type":"string","example":"z"}}}}}}],"responses":{"200":reply()}}}
        }
    })
}
fn plan_in(root: &Path) -> Plan {
    let file = root.join("api.json");
    std::fs::write(&file, fixture().to_string()).unwrap();
    let ws = Arc::new(WorkspaceBuilder::new().root(root).build().unwrap());
    let contract =
        Arc::new(Contract::from_workspace(&ws, &Uri::from_path(&file).unwrap()).unwrap());
    let selected = contract
        .operations()
        .map(|op| op.source().clone())
        .collect::<Vec<_>>();
    kotlin_sdk::plan_sdk(
        contract,
        &selected,
        SdkConfig {
            group_id: "test.suspect.kotlin".into(),
            artifact_id: "protocol-sdk".into(),
            version: "0.2.0".into(),
            package_name: "example.protocol".into(),
            credential_env: None,
            sdk_defaults: None,
            attribution: None,
        },
    )
    .unwrap()
}

#[test]
fn plans_rich_descriptors_without_binary_json_standins() {
    let root = root();
    let plan = plan_in(&root);
    assert!(plan.operations().iter().any(|op| op.flow));
    let upload = plan
        .operations()
        .iter()
        .find(|op| op.operation_id == "upload")
        .unwrap();
    let form = upload.body.as_ref().unwrap().media[0]
        .form
        .as_ref()
        .unwrap();
    assert!(plan.models().symbol(&form.source).is_none());
    let file = form
        .fields
        .iter()
        .find(|f| f.wire.name() == Some("file"))
        .unwrap();
    assert!(plan.models().symbol(file.wire.schema().id()).is_none());
    assert!(file.headers_type.is_some());
    assert!(
        plan.render()
            .unwrap()
            .iter()
            .all(|f| f.path.starts_with("kotlin/"))
    );
}

#[test]
#[ignore = "Maven installed Kotlin module, JDK 21/25, Dokka and rich native protocol consumer"]
fn native_rich_protocol_package() {
    let root = root();
    let plan = plan_in(&root);
    native_package(
        &root,
        &plan,
        include_str!("../src/kotlin_sdk/native_protocol.kt"),
        &[
            "val x = ChooseInput(body = byteArrayOf(1))",
            "val x = UploadRequestMultipartBodyFilePart(value = Upload(byteArrayOf(1)))",
            "val x = Credentials(oauth = \"token\")",
            "val x = BasicCredentials(username = \"user\")",
            "val x: kotlinx.coroutines.flow.Flow<Reply> = Client().lines()",
            "val x = FormRequestFormBody(config = FormBodyConfig(true), name = \"x\", tags = \"wrong\")",
            "val x = PostLinesInput(body = listOf(Reply(\"x\", JsonNumber.of(1))))",
            "val x = Event(data = JsonObject(emptyMap()))",
        ],
    );
}

fn native_package(root: &Path, plan: &Plan, consumer_source: &str, negative_cases: &[&str]) {
    for f in plan.render().unwrap() {
        let p = root.join(f.path);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, f.content).unwrap();
    }
    std::fs::write(
        root.join("bindings.txt"),
        format!("{:#?}", plan.operations()),
    )
    .unwrap();
    let consumer = root.join("consumer");
    std::fs::create_dir_all(consumer.join("src/main/kotlin")).unwrap();
    std::fs::write(
        consumer.join("src/main/kotlin/NativeProtocol.kt"),
        consumer_source,
    )
    .unwrap();
    std::fs::write(consumer.join("pom.xml"),format!(r#"<project xmlns="http://maven.apache.org/POM/4.0.0"><modelVersion>4.0.0</modelVersion><groupId>test.suspect</groupId><artifactId>consumer</artifactId><version>1.0.0</version>
<properties><project.build.sourceEncoding>UTF-8</project.build.sourceEncoding><kotlin.compiler.daemon>false</kotlin.compiler.daemon></properties>
<dependencies><dependency><groupId>{}</groupId><artifactId>{}</artifactId><version>{}</version></dependency></dependencies>
<build><sourceDirectory>src/main/kotlin</sourceDirectory><plugins>
<plugin><groupId>org.jetbrains.kotlin</groupId><artifactId>kotlin-maven-plugin</artifactId><version>{}</version><configuration><jvmTarget>21</jvmTarget><args><arg>-Werror</arg></args></configuration><executions><execution><id>compile</id><phase>compile</phase><goals><goal>compile</goal></goals></execution></executions></plugin>
<plugin><groupId>org.codehaus.mojo</groupId><artifactId>exec-maven-plugin</artifactId><version>3.6.3</version><configuration><executable>${{java.home}}/bin/java</executable><arguments><argument>-Xmx512m</argument><argument>-cp</argument><classpath/><argument>consumer.NativeProtocolKt</argument></arguments></configuration></plugin>
</plugins></build></project>"#,plan.config().group_id,plan.config().artifact_id,plan.config().version,kotlin_sdk::KOTLIN_VERSION)).unwrap();
    support::installed_quickstart(root, plan.config().artifact_id == "protocol-sdk");
    for (version, home) in support::java_homes() {
        let output = support::maven(&home)
            .arg("install")
            .current_dir(root.join("kotlin"))
            .output()
            .unwrap();
        let log = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        std::fs::write(root.join(format!("sdk-{version}.log")), &log).unwrap();
        assert!(output.status.success(), "{}\n{log}", root.display());
        support::native_docs(plan, root, &version);
        let output = support::maven(&home)
            .args(["compile", "exec:exec"])
            .current_dir(&consumer)
            .output()
            .unwrap();
        let log = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        std::fs::write(root.join(format!("consumer-{version}.log")), &log).unwrap();
        assert!(output.status.success(), "{}\n{log}", root.display());
        for (i, negative) in negative_cases.iter().enumerate() {
            let file = consumer.join("src/main/kotlin/Negative.kt");
            std::fs::write(
                &file,
                format!("package consumer\nimport example.protocol.*\n{negative}\n"),
            )
            .unwrap();
            let result = support::maven(&home)
                .arg("compile")
                .current_dir(&consumer)
                .output()
                .unwrap();
            let text = format!(
                "{}{}",
                String::from_utf8_lossy(&result.stdout),
                String::from_utf8_lossy(&result.stderr)
            );
            std::fs::write(root.join(format!("negative-{version}-{i}.log")), &text).unwrap();
            assert!(
                !result.status.success()
                    && text.contains("Negative.kt")
                    && !text.contains("Unresolved reference"),
                "uncontrolled negative result: {negative}\n{text}"
            );
            std::fs::remove_file(file).unwrap();
        }
    }
    println!("Kotlin rich package: {}", root.display());
}

fn legacy_fixture() -> Value {
    let marker = json!({"type":"string","format":"binary"});
    let json_marker = json!({"$ref":"#/components/schemas/JsonMarker"});
    json!({"openapi":"3.2.0","info":{"title":"Explicit binary profile","version":"1"},"servers":[{"url":"https://example.test/api"}],"security":[],
        "components":{"schemas":{"JsonMarker":{"type":"object","required":["content"],"additionalProperties":false,"properties":{"content":{"type":"string","format":"binary","minLength":1,"example":"ordinary JSON string"}}}}},
        "paths":{
            "/raw":{"post":{"operationId":"rawData","requestBody":{"required":true,"content":{"application/octet-stream":{"schema":marker}}},"responses":{"200":response("application/octet-stream",marker.clone())}}},
            "/parts":{"post":{"operationId":"parts","requestBody":{"required":true,"content":{"multipart/form-data":{"schema":{"type":"object","required":["file"],"additionalProperties":false,"properties":{"file":marker}},"encoding":{"file":{"contentType":"application/octet-stream"}}}}},"responses":{"200":response("application/json",json!({"type":"string","example":"ok"}))}}},
            "/json":{"post":{"operationId":"jsonMarker","requestBody":{"required":true,"content":{"application/json":{"schema":json_marker}}},"responses":{"200":response("application/json",json_marker.clone())}}}
        }
    })
}

fn legacy_plan(
    root: &Path,
    profiles: &std::collections::BTreeSet<suspect_codegen::http_protocol::CompatibilityProfile>,
) -> Result<Plan, Vec<kotlin_sdk::HttpDiagnostic>> {
    let file = root.join("api.json");
    std::fs::write(&file, legacy_fixture().to_string()).unwrap();
    let workspace = Arc::new(WorkspaceBuilder::new().root(root).build().unwrap());
    let contract =
        Arc::new(Contract::from_workspace(&workspace, &Uri::from_path(&file).unwrap()).unwrap());
    let selected = contract
        .operations()
        .map(|op| op.source().clone())
        .collect::<Vec<_>>();
    kotlin_sdk::plan_sdk_with_profiles(
        contract,
        &selected,
        SdkConfig {
            group_id: "test.suspect.kotlin".into(),
            artifact_id: "legacy-sdk".into(),
            version: "0.2.0".into(),
            package_name: "example.protocol".into(),
            credential_env: None,
            sdk_defaults: None,
            attribution: None,
        },
        profiles,
    )
}

#[test]
fn legacy_binary_is_explicit_and_does_not_reinterpret_json() {
    use suspect_codegen::http_protocol::CompatibilityProfile::LegacyBinaryStringV1;
    let root = root();
    let errors = legacy_plan(&root, &Default::default()).unwrap_err();
    assert!(
        errors.iter().any(|e| e.code == "http-binary-legacy-marker"
            && e.at.end > e.at.start
            && e.source.pointer().ends_with("/format")),
        "{errors:?}"
    );
    let plan = legacy_plan(&root, &[LegacyBinaryStringV1].into()).unwrap();
    assert_eq!(
        plan.protocol().capabilities().profiles(),
        &[LegacyBinaryStringV1].into()
    );
    let raw = plan
        .operations()
        .iter()
        .find(|o| o.operation_id == "rawData")
        .unwrap();
    assert_eq!(raw.body.as_ref().unwrap().kotlin_type, "ByteArray");
    assert!(raw.body.as_ref().unwrap().media[0].schema.is_none());
    let model = plan
        .models()
        .symbols()
        .iter()
        .find(|m| m.name == "JsonMarker")
        .unwrap();
    let kotlin_sdk::models::Shape::Object { fields, .. } = &model.shape else {
        panic!()
    };
    assert_eq!(fields[0].kotlin_type, "String");
}

#[test]
#[ignore = "explicit legacy-binary-string-v1 installed JDK 21/25 byte/JSON witness"]
fn native_legacy_binary_profile() {
    let root = root();
    let plan = legacy_plan(
        &root,
        &[suspect_codegen::http_protocol::CompatibilityProfile::LegacyBinaryStringV1].into(),
    )
    .unwrap();
    native_package(
        &root,
        &plan,
        include_str!("../src/kotlin_sdk/native_protocol_profile.kt"),
        &[
            "val x = RawDataInput(body = \"raw bytes\")",
            "val x = JsonMarker(content = byteArrayOf(1))",
            "val x = PartsRequestMultipartBody(file = byteArrayOf(1))",
        ],
    );
}
