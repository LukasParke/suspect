//! Public Contract -> Go plan -> independently installed native consumers.
//! Literal normative vectors are shared with the core tests, not derived from
//! emitted Go or from a serializer under test.
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    process::Command,
    sync::Arc,
};
use suspect_codegen::go_http::{self, HttpConfig, HttpPlan, PlannedOperation};
use suspect_codegen::http_protocol::{
    Capability, CompatibilityProfile, Representation, ResponseStatus,
};
use suspect_ir::contract::Contract;
use suspect_ref::WorkspaceBuilder;
use suspect_source::Uri;

const CORE: &str = include_str!("fixtures/http-protocol-v1.json");
fn contract(document: &Value) -> Arc<Contract> {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("api.json");
    std::fs::write(&path, document.to_string()).unwrap();
    load(&path)
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
fn plan(document: Value) -> HttpPlan {
    let contract = contract(&document);
    let selected = contract
        .operations()
        .map(|o| o.source().clone())
        .collect::<Vec<_>>();
    go_http::plan_http(contract, &selected, Default::default()).unwrap()
}
fn base() -> Value {
    json!({"openapi":"3.2.0","info":{"title":"Independent Go protocol witnesses","version":"1"},"servers":[{"url":"https://example.test/v1"}],"paths":{}})
}
fn op<'a>(plan: &'a HttpPlan, name: &str) -> &'a PlannedOperation {
    plan.operations()
        .iter()
        .find(|o| o.operation_id == name)
        .unwrap()
}
fn q(s: &str) -> String {
    serde_json::to_string(s).unwrap()
}
fn tools() -> Vec<String> {
    std::env::var("SUSPECT_GO_TOOLCHAIN")
        .map(|t| vec![t])
        .unwrap_or_else(|_| vec!["go1.23.12".into(), "go1.27.1".into()])
}
fn go(dir: &Path, tool: &str, args: &[&str]) -> std::process::Output {
    Command::new("go")
        .args(args)
        .current_dir(dir)
        .env("GOWORK", "off")
        .env("GOTOOLCHAIN", tool)
        .output()
        .unwrap()
}
fn success(dir: &Path, tool: &str, args: &[&str]) {
    let out = go(dir, tool, args);
    assert!(
        out.status.success(),
        "{} {tool} {args:?}\n{}{}",
        dir.display(),
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    eprintln!(
        "{tool} {}: {}",
        args.join(" "),
        String::from_utf8_lossy(&out.stdout).trim()
    );
}
fn native(plan: &HttpPlan, source: &str, negative: &[&str]) {
    let root = tempfile::Builder::new()
        .prefix("suspect-go-protocol-")
        .tempdir()
        .unwrap()
        .keep();
    suspect_codegen::write_files(&plan.render(), &root).unwrap();
    let consumer = root.join("consumer");
    std::fs::create_dir(&consumer).unwrap();
    std::fs::write(consumer.join("go.mod"),"module example.com/protocol-consumer\n\ngo 1.23.0\nrequire example.com/generated-sdk v0.0.0\nreplace example.com/generated-sdk => ../go\n").unwrap();
    std::fs::write(consumer.join("protocol_test.go"), source).unwrap();
    for tool in tools() {
        success(&root.join("go"), &tool, &["test", "./..."]);
        success(&root.join("go"), &tool, &["run", "./examples/validated"]);
        success(
            &consumer,
            &tool,
            &["test", "-count=1", "-timeout=40s", "-v", "."],
        );
        let docs = go(
            &consumer,
            &tool,
            &["doc", "-all", "example.com/generated-sdk"],
        );
        assert!(
            docs.status.success(),
            "{}",
            String::from_utf8_lossy(&docs.stderr)
        );
        for (i, bad) in negative.iter().enumerate() {
            let dir = consumer.join(format!("negative{i}"));
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(
                dir.join("invalid.go"),
                format!("package invalid\nimport sdk \"example.com/generated-sdk\"\n{bad}\n"),
            )
            .unwrap();
            let out = go(
                &consumer,
                &tool,
                &["test", "-run=^$", &format!("./negative{i}")],
            );
            assert!(
                !out.status.success()
                    && String::from_utf8_lossy(&out.stderr).contains("invalid.go:"),
                "invalid consumer compiled or failed outside its source: {}",
                String::from_utf8_lossy(&out.stderr)
            );
        }
    }
    std::fs::remove_dir_all(root).unwrap();
}
const PRELUDE: &str = r#"package consumer
import("bytes";"context";"errors";"fmt";"io";"net/http";"net/http/httptest";"reflect";"strings";"sync/atomic";"testing";"time";sdk "example.com/generated-sdk")
var _=bytes.Equal;var _=context.Background;var _=errors.As;var _=fmt.Sprint;var _=io.EOF;var _=http.MethodGet;var _=httptest.NewServer;var _=reflect.DeepEqual;var _=strings.Contains;var _ atomic.Int32;var _=time.Second
type doer func(*http.Request)(*http.Response,error)
func(f doer)Do(r *http.Request)(*http.Response,error){return f(r)}
func reply(status int,media string,body []byte)*http.Response{headers:=make(http.Header);if media!=""{headers.Set("Content-Type",media)};return &http.Response{StatusCode:status,Header:headers,Body:io.NopCloser(bytes.NewReader(body))}}
func must[T any](value T,err error)T{if err!=nil{panic(err)};return value}
func sdkFailure(t *testing.T,err error,kind string)*sdk.SDKError{t.Helper();var e *sdk.SDKError;if !errors.As(err,&e)||e.Kind!=kind{t.Fatalf("expected SDK %s, got %#v / %v",kind,e,err)};return e}
"#;

#[test]
fn shared_descriptors_are_the_go_admission_and_codec_authority() {
    let core: Value = serde_json::from_str(CORE).unwrap();
    for key in [
        "baseline",
        "multipart",
        "form",
        "positionalMultipart",
        "stream",
    ] {
        let mut spec = core[key].clone();
        if let Some(paths) = spec["paths"].as_object_mut() {
            for (_, item) in paths {
                for (method, operation) in item.as_object_mut().unwrap() {
                    if ["get", "post"].contains(&method.as_str())
                        && operation.get("operationId").is_none()
                    {
                        operation["operationId"] = json!(format!("{key}{method}"));
                    }
                }
            }
        }
        let p = plan(spec);
        assert_eq!(
            p.protocol().capabilities().adapter(),
            "go-net-http-protocol-v1"
        );
        for root in p.protocol().codec_roots() {
            assert!(
                p.symbols().contains_key(root),
                "actual root missing native codec: {root:?}"
            );
        }
        let files = p.render();
        let manifest: Value = serde_json::from_str(
            &files
                .iter()
                .find(|f| f.path == "go/http-manifest.json")
                .unwrap()
                .content,
        )
        .unwrap();
        assert_eq!(
            manifest["protocol"],
            serde_json::to_value(p.protocol()).unwrap()
        );
        if key == "multipart" {
            let roots = p
                .protocol()
                .codec_roots()
                .iter()
                .map(|s| s.pointer())
                .collect::<Vec<_>>();
            assert!(!roots.iter().any(|p| p.ends_with("/file")));
            assert!(p.operations()[0].body().unwrap().schema().is_none());
        }
    }
}

#[test]
fn unsupported_profiles_refuse_at_source_and_legacy_binary_is_explicit() {
    let mut spec = base();
    spec["paths"]["/upload"] = json!({"post":{"operationId":"upload","requestBody":{"required":true,"content":{"application/octet-stream":{"schema":{"type":"string","format":"binary"}}}},"responses":{"204":{}}}});
    let c = contract(&spec);
    let selected = c
        .operations()
        .map(|o| o.source().clone())
        .collect::<Vec<_>>();
    let failures = go_http::plan_http(c.clone(), &selected, Default::default()).unwrap_err();
    assert!(
        failures
            .iter()
            .any(|e| e.code == "http-binary-legacy-marker"
                && e.source.pointer().ends_with("/format"))
    );
    let mut config = HttpConfig::default();
    config
        .compatibility_profiles
        .insert(CompatibilityProfile::LegacyBinaryStringV1);
    let p = go_http::plan_http(c, &selected, config).unwrap();
    assert!(p.protocol().codec_roots().is_empty());
    assert_eq!(p.operations()[0].body().unwrap().native_type, "[]byte");
    for (media, value, code) in [
        (
            "application/json-seq",
            json!({}),
            "http-stream-framing-unsupported",
        ),
        (
            "text/event-stream",
            json!({"schema":{"type":"object"}}),
            "http-stream-item-schema-required",
        ),
    ] {
        spec["paths"]["/upload"]["post"]["requestBody"]["content"] = json!({media:value});
        let c = contract(&spec);
        let selected = c
            .operations()
            .map(|o| o.source().clone())
            .collect::<Vec<_>>();
        assert!(
            go_http::plan_http(c, &selected, Default::default())
                .unwrap_err()
                .iter()
                .any(|e| e.code == code)
        );
    }
}

#[test]
#[ignore = "requires installed native Go 1.23.12 and 1.27.1"]
fn native_all_literal_parameter_vectors_and_fail_before_transport() {
    let core: Value = serde_json::from_str(CORE).unwrap();
    let cases = core["parameterCases"].as_array().unwrap();
    let mut spec = base();
    for (i, case) in cases.iter().enumerate() {
        let p = &case["parameter"];
        let path = if p["in"] == "path" {
            format!("/case{i}/{{{}}}", p["name"].as_str().unwrap())
        } else {
            format!("/case{i}")
        };
        spec["paths"][&path] = json!({"get":{"operationId":format!("case{i}"),"parameters":[p],"responses":{"200":{"content":{"application/json":{"schema":{"type":"boolean"}}}}}}});
    }
    let p = plan(spec);
    let mut source = PRELUDE.to_owned();
    for (i, case) in cases.iter().enumerate() {
        let operation = op(&p, &format!("case{i}"));
        let parameter = &operation.parameters()[0];
        let model = &p.symbols()[parameter.schema()];
        let expr = if let Some(setter) = &parameter.setter_name {
            format!("sdk.{}().{setter}(value)", operation.input_constructor)
        } else {
            format!("sdk.{}(value)", operation.input_constructor)
        };
        let expected = case["wire"].as_str().unwrap();
        let location = case["parameter"]["in"].as_str().unwrap();
        let assertion = match location {
            "path" => format!(
                "r.URL.EscapedPath()!={}",
                q(&format!("/v1/case{i}/{expected}"))
            ),
            "query" => format!("r.URL.RawQuery!={}", q(expected)),
            "header" => format!(
                "r.Header.Get({})!={}",
                q(case["parameter"]["name"].as_str().unwrap()),
                q(expected)
            ),
            _ => format!("r.Header.Get(\"Cookie\")!={}", q(expected)),
        };
        source.push_str(&format!(r#"func TestVector{i}(t *testing.T){{calls:=0;client:=must(sdk.NewClient(sdk.Credentials{{}},sdk.ClientOptions{{Transport:doer(func(r *http.Request)(*http.Response,error){{calls++;if {assertion}{{t.Fatalf("literal wire: %s %#v",r.URL,r.Header)}};return reply(200,"application/json",[]byte("true")),nil}})}}));value:=must(sdk.Codecs.{model}.Decode([]byte({value})));result,err:=client.{method}(context.Background(),{expr});if err!=nil||!result||calls!=1{{t.Fatal(result,err,calls)}}}}
"#,value=q(&case["value"].to_string()),method=operation.data_method.as_ref().unwrap()));
    }
    // Independently specified negative values: data cannot inject query/header syntax.
    for (case, value) in [
        (21, json!("a&b")),
        (23, json!("bad\r\nInjected: x")),
        (16, json!(["one two"])),
        (18, json!(["one|two"])),
        (14, json!([])),
    ] {
        let operation = op(&p, &format!("case{case}"));
        let parameter = &operation.parameters()[0];
        let model = &p.symbols()[parameter.schema()];
        let setter = parameter.setter_name.as_ref().unwrap();
        source.push_str(&format!(r#"func TestBadVector{case}(t *testing.T){{calls:=0;client:=must(sdk.NewClient(sdk.Credentials{{}},sdk.ClientOptions{{Transport:doer(func(*http.Request)(*http.Response,error){{calls++;return reply(200,"application/json",[]byte("true")),nil}})}}));value:=must(sdk.Codecs.{model}.Decode([]byte({value})));_,err:=client.{method}(context.Background(),sdk.{constructor}().{setter}(value));sdkFailure(t,err,"request-representation");if calls!=0{{t.Fatal("invalid value sent")}}}}
"#,value=q(&value.to_string()),method=operation.method_name,constructor=operation.input_constructor));
    }
    native(
        &p,
        &source,
        &["var _ = sdk.NewCase0Input([]string{\"wrong scalar\"})"],
    );
}

#[test]
#[ignore = "requires installed native Go 1.23.12 and 1.27.1"]
fn native_security_alternatives_servers_methods_and_querystring() {
    let core: Value = serde_json::from_str(CORE).unwrap();
    let mut spec = base();
    spec["components"]["securitySchemes"] = core["securityCases"]["schemes"].clone();
    spec["components"]["securitySchemes"]["queryKey"] =
        json!({"type":"apiKey","in":"query","name":"access"});
    spec["components"]["securitySchemes"]["cookieKey"] =
        json!({"type":"apiKey","in":"cookie","name":"session"});
    spec["security"] = json!([{"token":[]}]);
    for (name, security) in [
        ("anonymous", json!([])),
        ("choice", core["securityCases"]["security"].clone()),
        (
            "conjunctive",
            json!([{"token":["reader"],"key":[],"queryKey":[],"cookieKey":[]}]),
        ),
    ] {
        spec["paths"][&format!("/{name}")] = json!({"get":{"operationId":name,"security":security,"responses":{"200":{"content":{"text/plain":{"schema":{"type":"string"}}}}}}});
    }
    for method in [
        "get", "put", "post", "delete", "options", "head", "patch", "trace", "query",
    ] {
        spec["paths"][&format!("/method-{method}")] = json!({method:{"operationId":format!("method-{method}"),"security":[],"responses":{"204":{}}}});
    }
    spec["paths"]["/custom"] = json!({"additionalOperations":{"GeT":{"operationId":"customMethod","security":[],"responses":{"204":{}}}}});
    spec["paths"]["/servers"] = json!({"get":{"operationId":"servers","security":[],"servers":core["serverCases"]["servers"],"responses":{"204":{}}}});
    for (name, content) in [
        (
            "queryJSON",
            json!({"application/json":{"schema":{"type":"string"}}}),
        ),
        (
            "queryText",
            json!({"text/plain":{"schema":{"type":"string"}}}),
        ),
        (
            "queryForm",
            json!({"application/x-www-form-urlencoded":{"schema":{"type":"object","required":["q"],"properties":{"q":{"type":"string"}},"additionalProperties":false}}}),
        ),
    ] {
        spec["paths"][&format!("/{name}")] = json!({"query":{"operationId":name,"security":[],"parameters":[{"name":"whole","in":"querystring","required":true,"content":content}],"responses":{"204":{}}}});
    }
    let p = plan(spec);
    assert!(
        p.protocol()
            .capabilities()
            .supports(Capability::CustomMethods)
    );
    let mut source = PRELUDE.to_owned();
    source.push_str(r#"
func TestSecurity(t *testing.T){
 ctx:=context.Background();calls:=0
 transport:=doer(func(r *http.Request)(*http.Response,error){calls++;return reply(200,"text/plain",[]byte(r.Header.Get("Authorization")+"|"+r.Header.Get("X-Key")+"|"+r.URL.RawQuery+"|"+r.Header.Get("Cookie"))),nil})
 client:=must(sdk.NewClient(sdk.Credentials{}.WithBearer("token","secret"),sdk.ClientOptions{Transport:transport}))
 if value,err:=client.AnonymousData(ctx);err!=nil||value!="|||"{t.Fatal(value,err)}
 if value,err:=client.ChoiceData(ctx);err!=nil||value!="|||"{t.Fatal("anonymous OR member",value,err)}
 for _,test:=range []struct{index int;credentials sdk.Credentials;want string}{
 {1,sdk.Credentials{}.WithBearer("token","bearer").WithAPIKey("key","key-value"),"Bearer bearer|key-value||"},
 {4,sdk.Credentials{}.WithBasic("basic","user","p:ass"),"Basic dXNlcjpwOmFzcw==|||"},
 {2,sdk.Credentials{}.WithHook("oauth",func(ctx context.Context,request sdk.CredentialRequest)(sdk.Authorization,error){r:=request.Requirement;if r.Kind!="oauth2"||!reflect.DeepEqual(r.Scopes,[]string{"read:items"})||len(r.Roles)!=0||r.Flows[0].TokenURL!="https://auth.example.test/token"||r.Flows[0].Scopes["read:items"]!="Read items"||r.Scheme.Definition.Pointer!="/components/securitySchemes/oauth"{t.Fatalf("scope metadata %#v",r)};r.Scopes[0]="mutated";return sdk.Authorization{Scheme:"Token",Value:"caller-owned"},nil}),"Token caller-owned|||"},
 {3,sdk.Credentials{}.WithHook("oidc",func(ctx context.Context,request sdk.CredentialRequest)(sdk.Authorization,error){if request.Requirement.DiscoveryURL!="https://auth.example.test/.well-known/openid-configuration"||!reflect.DeepEqual(request.Requirement.Scopes,[]string{"openid","profile"}){t.Fatal(request)};return sdk.Authorization{Scheme:"Bearer",Value:"oidc"},nil}),"Bearer oidc|||"},
 }{client:=must(sdk.NewClient(test.credentials,sdk.ClientOptions{Transport:transport,SecurityAlternative:&test.index}));for i:=0;i<2;i++{v,err:=client.ChoiceData(ctx);if err!=nil||v!=test.want{t.Fatal(v,err)}}}
 credentials:=sdk.Credentials{}.WithBearer("token","all").WithAPIKey("key","header").WithAPIKey("queryKey","a +/").WithAPIKey("cookieKey","a +/")
 client=must(sdk.NewClient(credentials,sdk.ClientOptions{Transport:transport}));v,err:=client.ConjunctiveData(ctx);if err!=nil||v!="Bearer all|header|access=a%20%2B%2F|session=a%20%2B%2F"{t.Fatal(v,err)}
 before:=calls;bad:=must(sdk.NewClient(credentials.WithBearer("token","bad\nvalue"),sdk.ClientOptions{Transport:transport}));_,err=bad.Conjunctive(ctx);sdkFailure(t,err,"request-validation");missing:=must(sdk.NewClient(sdk.Credentials{},sdk.ClientOptions{Transport:transport}));_,err=missing.Conjunctive(ctx);sdkFailure(t,err,"request-validation");if calls!=before{t.Fatal("credential refusal reached transport")}
}
func TestServerSelection(t *testing.T){
 ctx:=context.Background();calls:=0
 for _,test:=range []struct{index int;variables map[string]string;document,want string}{
 {0,nil,"","https://demo.example.test:443/v2/servers"},
 {0,map[string]string{"tenant":"customer","port":"8443","basePath":"v3"},"","https://customer.example.test:8443/v3/servers"},
 {1,nil,"https://docs.example.test/specs/openapi.json","https://docs.example.test/api/servers"},
 }{client:=must(sdk.NewClient(sdk.Credentials{},sdk.ClientOptions{ServerIndex:test.index,ServerVariables:test.variables,DocumentURL:test.document,Transport:doer(func(r *http.Request)(*http.Response,error){calls++;if r.URL.String()!=test.want{t.Fatal(r.URL.String(),test.want)};return reply(204,"",nil),nil})}));if _,err:=client.Servers(ctx);err!=nil{t.Fatal(err)}}
 for _,options:=range []sdk.ClientOptions{{ServerIndex:9},{ServerIndex:1},{ServerVariables:map[string]string{"port":"1234"}},{ServerVariables:map[string]string{"unknown":"x"}},{ServerVariables:map[string]string{"tenant":"bad#host"}}}{options.Transport=doer(func(*http.Request)(*http.Response,error){calls++;return reply(204,"",nil),nil});client:=must(sdk.NewClient(sdk.Credentials{},options));_,err:=client.Servers(ctx);sdkFailure(t,err,"request-representation")};if calls!=3{t.Fatal("invalid server reached transport",calls)}
}
"#);
    for operation in p
        .operations()
        .iter()
        .filter(|o| o.operation_id.starts_with("method-") || o.operation_id == "customMethod")
    {
        source.push_str(&format!(r#"func Test{method}(t *testing.T){{client:=must(sdk.NewClient(sdk.Credentials{{}},sdk.ClientOptions{{Transport:doer(func(r *http.Request)(*http.Response,error){{if r.Method!={expected}{{t.Fatal(r.Method)}};return reply(204,"",nil),nil}})}}));if _,err:=client.{method}(context.Background());err!=nil{{t.Fatal(err)}}}}
"#,method=operation.method_name,expected=q(operation.wire().method().as_str())));
    }
    for (name, value, want) in [
        (
            "queryJSON",
            json!("a=1&x=雪"),
            "%22a%3D1%26x%3D%E9%9B%AA%22",
        ),
        ("queryText", json!("a=1&x=雪"), "a%3D1%26x%3D%E9%9B%AA"),
        ("queryForm", json!({"q":"a +/雪"}), "q=a+%2B%2F%E9%9B%AA"),
    ] {
        let operation = op(&p, name);
        let model = &p.symbols()[operation.parameters()[0].schema()];
        source.push_str(&format!(r#"func Test{method}(t *testing.T){{client:=must(sdk.NewClient(sdk.Credentials{{}},sdk.ClientOptions{{Transport:doer(func(r *http.Request)(*http.Response,error){{if r.Method!="QUERY"||r.URL.RawQuery!={want}{{t.Fatal(r.Method,r.URL.RawQuery)}};return reply(204,"",nil),nil}})}}));value:=must(sdk.Codecs.{model}.Decode([]byte({value})));if _,err:=client.{method}(context.Background(),sdk.{constructor}(value));err!=nil{{t.Fatal(err)}}}}
"#,method=operation.method_name,constructor=operation.input_constructor,want=q(want),value=q(&value.to_string())));
    }
    native(
        &p,
        &source,
        &["var _ = sdk.NewQueryTextInput([]byte(\"not text\"))"],
    );
}

#[test]
#[ignore = "requires installed native Go 1.23.12 and 1.27.1"]
fn native_response_status_media_headers_links_and_byte_boundaries() {
    let mut spec = base();
    spec["paths"]["/probe"] = json!({"get":{"operationId":"probe","responses":{
     "200":{"headers":{"X-Count":{"required":true,"schema":{"type":"integer","minimum":1}},"X-Tags":{"schema":{"type":"array","items":{"type":"string"}}},"X-Flags":{"explode":true,"schema":{"type":"object","additionalProperties":{"type":"boolean"}}}},"links":{"next":{"operationId":"next","parameters":{"id":"$response.header.X-Count"},"requestBody":{"schema":{"type":"integer"},"$ref":"literal"}}},"content":{
      "application/json":{"schema":{"type":"integer"}},"application/json;profile=exact":{"schema":{"type":"string","const":"profile"}},"application/problem+json":{"schema":{"type":"string"}},"text/plain":{"schema":{"type":"string"}},"application/*":{},"*/*":{}
     }},
     "2XX":{"content":{"text/plain":{"schema":{"type":"string"}}}},
     "default":{"content":{"application/json":{"schema":{"type":"string"}}}}
    }}});
    spec["paths"]["/next"] = json!({"get":{"operationId":"next","responses":{"204":{}}}});
    spec["paths"]["/bytes"] = json!({"get":{"operationId":"bytes","responses":{"200":{}}}});
    spec["paths"]["/head"] = json!({"head":{"operationId":"head","responses":{"200":{"content":{"application/json":{"schema":{"not":{"type":"string"}}}}}}}});
    spec["paths"]["/exact"] = json!({"get":{"operationId":"exact","responses":{"200":{"content":{"application/json":{"schema":{"type":"boolean"}}}},"2XX":{"content":{"text/plain":{"schema":{"type":"string"}}}},"default":{}}}});
    spec["paths"]["/default"] = json!({"get":{"operationId":"fallback","responses":{"default":{"content":{"application/json":{"schema":{"type":"string"}}}}}}});
    spec["paths"]["/send"] = json!({"post":{"operationId":"send","requestBody":{"required":true,"content":{"application/json":{"schema":{"type":"object","required":["name"],"properties":{"name":{"type":"string","minLength":1}},"additionalProperties":false}},"text/plain":{"schema":{"type":"boolean"}},"*/*":{}}},"responses":{"204":{}}}});
    spec["paths"]["/finite"] = json!({"post":{"operationId":"finite","requestBody":{"required":true,"content":{"application/octet-stream":{"schema":{"maxLength":3}}}},"responses":{"200":{"content":{"application/octet-stream":{"schema":{"maxLength":3}}}}}}});
    let p = plan(spec);
    assert!(
        !p.protocol()
            .codec_roots()
            .iter()
            .any(|r| r.pointer().contains("/head/") || r.pointer().contains("/links/"))
    );
    let probe = op(&p, "probe");
    let mut source = PRELUDE.to_owned();
    let tests = [
        (
            200,
            "Application/JSON; charset=\"UTF-8\"",
            b"9007199254740993".to_vec(),
            "application/json",
        ),
        (
            200,
            "application/json;profile=exact; CHARSET=utf-8",
            b"\"profile\"".to_vec(),
            "application/json;profile=exact",
        ),
        (
            200,
            "application/problem+json",
            b"\"problem\"".to_vec(),
            "application/problem+json",
        ),
        (
            200,
            "text/plain; charset=UTF-8",
            "雪 + %".as_bytes().to_vec(),
            "text/plain",
        ),
        (
            200,
            "application/pdf",
            vec![0, 255, 13, 10, 128],
            "application/*",
        ),
        (200, "image/png", vec![0, 255, 0], "*/*"),
        (201, "text/plain", b"range".to_vec(), "text/plain"),
    ];
    for (i, (status, ct, body, declared)) in tests.iter().enumerate() {
        let response = probe
            .responses()
            .iter()
            .find(|r| {
                r.status()
                    == if *status == 200 {
                        ResponseStatus::Exact(200)
                    } else {
                        ResponseStatus::Range(2)
                    }
                    && r.media()
                        .is_some_and(|m| m.wire().media_type().declared() == *declared)
            })
            .unwrap();
        let type_name = &response.type_name;
        let literal = format!(
            "[]byte{{{}}}",
            body.iter()
                .map(|b| b.to_string())
                .collect::<Vec<_>>()
                .join(",")
        );
        let inspect = match response.media().unwrap().wire().representation() {
            Representation::Json { .. } if declared == &"application/json" => {
                "if value.Data.String()!=\"9007199254740993\"{t.Fatal(value.Data)}".into()
            }
            Representation::Json { .. } | Representation::Text { .. } => format!(
                "if string(value.Data)!={}{{t.Fatal(value.Data)}}",
                q(if declared == &"application/json;profile=exact" {
                    "profile"
                } else if declared == &"application/problem+json" {
                    "problem"
                } else if *status == 201 {
                    "range"
                } else {
                    "雪 + %"
                })
            ),
            _ => format!("if !bytes.Equal(value.Data,{literal}){{t.Fatal(value.Data)}}"),
        };
        let header = if *status == 200 {
            r#"if value.DecodedHeaders.XCount.String()!="7"||!value.DecodedHeaders.XTags.IsSet||!reflect.DeepEqual([]string(value.DecodedHeaders.XTags.Value),[]string{"a","b"})||value.DecodedHeaders.XFlags.Value["B"]!=false{t.Fatal(value.DecodedHeaders)};if len(value.Links)!=1||value.Links[0].Target.Pointer!="/paths/~1next/get"||string(value.Links[0].RequestBody.JSON)!=`{"$ref":"literal","schema":{"type":"integer"}}`{t.Fatal(value.Links)}"#
        } else {
            ""
        };
        source.push_str(&format!(r#"func TestDispatch{i}(t *testing.T){{client:=must(sdk.NewClient(sdk.Credentials{{}},sdk.ClientOptions{{Transport:doer(func(*http.Request)(*http.Response,error){{r:=reply({status},{ct},{literal});r.Header.Set("X-Count","7");r.Header["X-Tags"]=[]string{{"a","b"}};r.Header.Set("X-Flags","A=true,B=false");return r,nil}})}}));result,err:=client.Probe(context.Background());if err!=nil{{t.Fatal(err)}};value,ok:=result.(sdk.{type_name});if !ok||value.Status!={status}{{t.Fatalf("wrong wrapper/status: %#v",result)}};{inspect};{header}}}
"#,ct=q(ct)));
    }
    let fallback = op(&p, "fallback")
        .responses()
        .iter()
        .find(|r| r.media().is_some())
        .unwrap();
    source.push_str(&format!(r#"
func TestActualDefaultStatus(t *testing.T){{for _,status:=range []int{{201,299,404,503}}{{client:=must(sdk.NewClient(sdk.Credentials{{}},sdk.ClientOptions{{Transport:doer(func(*http.Request)(*http.Response,error){{return reply(status,"application/json",[]byte(`"default"`)),nil}})}}));result,err:=client.Fallback(context.Background());if status<300{{if err!=nil||result.(sdk.{ty}).Status!=status{{t.Fatal(result,err)}}}}else{{var failure *sdk.{ty};if !errors.As(err,&failure)||failure.Status!=status||failure.Data!="default"{{t.Fatal(err)}}}}}}}}
"#,ty=fallback.type_name));
    source.push_str(r#"
type neverRead struct{closed bool;reads int}
func(b *neverRead)Read([]byte)(int,error){b.reads++;return 0,errors.New("forbidden body read")}
func(b *neverRead)Close()error{b.closed=true;return nil}
func TestHeaderAndMediaRefusals(t *testing.T){
 for _,test:=range []struct{media,count string;duplicate bool}{
 {"application/json","",false},{"application/json","0",false},{"application/json","no",false},
 {"","7",false},{"application/json; profile=a; PROFILE=a","7",false},{"text/plain;charset=latin1","7",false},{"application/json","7",true},
 }{client:=must(sdk.NewClient(sdk.Credentials{},sdk.ClientOptions{Transport:doer(func(*http.Request)(*http.Response,error){r:=reply(200,test.media,[]byte("1"));if test.count!=""{r.Header.Set("X-Count",test.count)};if test.duplicate{r.Header.Add("Content-Type","application/json")};return r,nil})}));_,err:=client.Probe(context.Background());var failure *sdk.SDKError;if !errors.As(err,&failure)||failure.Status!=200{t.Fatal(test,err)};if test.media=="application/json"&&!test.duplicate&&failure.Source.Pointer!="/paths/~1probe/get/responses/200/headers/X-Count"{t.Fatal("missing header provenance",failure.Source)}}
 for _,media:=range []string{"text/plain","application/json;profile=a;PROFILE=b","application/*"}{client:=must(sdk.NewClient(sdk.Credentials{},sdk.ClientOptions{Transport:doer(func(*http.Request)(*http.Response,error){return reply(200,media,[]byte("true")),nil})}));_,err:=client.Exact(context.Background());sdkFailure(t,err,"unexpected-response")}
}
"#);
    source.push_str(RESPONSE_TAIL);
    let send = op(&p, "send");
    let body = send.body().unwrap();
    let json_media = body
        .media()
        .iter()
        .find(|m| m.wire().media_type().is_json())
        .unwrap();
    let text_media = body
        .media()
        .iter()
        .find(|m| m.wire().media_type().is_text())
        .unwrap();
    let bytes_media = body
        .media()
        .iter()
        .find(|m| m.requires_content_type())
        .unwrap();
    let model = &p.symbols()[json_media.schema().unwrap()];
    source.push_str(&format!(r#"
func TestRequestMedia(t *testing.T){{calls:=0;client:=must(sdk.NewClient(sdk.Credentials{{}},sdk.ClientOptions{{Transport:doer(func(r *http.Request)(*http.Response,error){{calls++;data,err:=io.ReadAll(r.Body);if err!=nil{{t.Fatal(err)}};expected:=map[string]string{{"application/json":`{{"name":"native"}}`,"text/plain":"false","image/png":string([]byte{{0,255}})}};if string(data)!=expected[r.Header.Get("Content-Type")]{{t.Fatal(r.Header,string(data))}};return reply(204,"",nil),nil}})}}));for _,body:=range []sdk.{body_type}{{sdk.{json_ctor}(sdk.New{model}("native")),sdk.{text_ctor}(false),sdk.{bytes_ctor}("image/png",[]byte{{0,255}})}}{{if _,err:=client.Send(context.Background(),sdk.NewSendInput(body));err!=nil{{t.Fatal(err)}}}};_,err:=client.Send(context.Background(),sdk.NewSendInput(sdk.{bytes_ctor}("application/json",[]byte(`{{}}`))));sdkFailure(t,err,"request-validation");_,err=client.Send(context.Background(),sdk.NewSendInput(sdk.{bytes_ctor}("*/*",nil)));sdkFailure(t,err,"request-validation");if calls!=3{{t.Fatal(calls)}}}}
"#,body_type=body.native_type,json_ctor=json_media.constructor,text_ctor=text_media.constructor,bytes_ctor=bytes_media.constructor));
    native(
        &p,
        &source,
        &[
            "var _ = sdk.NewFiniteInput(\"not bytes\")",
            &format!("var _ = sdk.NewSendInput(sdk.New{model}(\"needs media choice\"))"),
        ],
    );
}

#[test]
#[ignore = "requires installed native Go 1.23.12 and 1.27.1"]
fn native_forms_named_and_positional_multipart_are_structural_and_typed() {
    let mut spec = base();
    let named = json!({"schema":{"type":"object","required":["file","metadata"],"maxProperties":4,"additionalProperties":{"type":"integer","minimum":1},"properties":{
  "file":{"maxLength":3},"files":{"type":"array","minItems":1,"maxItems":2,"items":{}},"metadata":{"type":"object","required":["title"],"properties":{"title":{"type":"string","minLength":1}},"additionalProperties":false},"labels":{"type":"array","minItems":1,"maxItems":2,"items":{"type":"string","minLength":1}}
 }},"encoding":{"file":{"contentType":"image/png, image/jpeg","headers":{"X-Part-Id":{"required":true,"schema":{"type":"string","minLength":1}}}}}});
    let form = json!({"schema":{"type":"object","required":["id"],"additionalProperties":false,"properties":{"id":{"type":"string","minLength":1},"address":{"type":"object","required":["city"],"properties":{"city":{"type":"string"}},"additionalProperties":false},"tags":{"type":"array","minItems":1,"maxItems":2,"items":{"type":"string"}},"codes":{"type":"object","additionalProperties":{"type":"integer"}}}},"encoding":{"codes":{"style":"deepObject","explode":true}}});
    let positional = json!({"schema":{"type":"array","minItems":1,"maxItems":3,"prefixItems":[{"type":"string","minLength":1}],"items":{"type":"object","required":["n"],"properties":{"n":{"type":"integer","minimum":1}},"additionalProperties":false}},"prefixEncoding":[{"contentType":"text/plain"}],"itemEncoding":{"contentType":"application/json","headers":{"X-Seq":{"required":true,"schema":{"type":"integer","minimum":1}}}}});
    for (name, media, body) in [
        ("upload", "multipart/form-data", named),
        ("form", "application/x-www-form-urlencoded", form),
        ("parts", "multipart/mixed", positional),
    ] {
        spec["paths"][&format!("/{name}")] = json!({"post":{"operationId":name,"requestBody":{"required":true,"content":{media:body.clone()}},"responses":{"200":{"content":{media:body}}}}});
    }
    let p = plan(spec);
    let upload = op(&p, "upload").body().unwrap().media()[0]
        .aggregate
        .as_ref()
        .unwrap();
    let form = op(&p, "form").body().unwrap().media()[0]
        .aggregate
        .as_ref()
        .unwrap();
    let parts = op(&p, "parts").body().unwrap().media()[0]
        .aggregate
        .as_ref()
        .unwrap();
    let metadata = upload
        .parts
        .iter()
        .find(|p| p.wire.name() == Some("metadata"))
        .unwrap();
    let address = form
        .parts
        .iter()
        .find(|p| p.wire.name() == Some("address"))
        .unwrap();
    let codes = form
        .parts
        .iter()
        .find(|p| p.wire.name() == Some("codes"))
        .unwrap();
    let mut source = PRELUDE.replace("\"bytes\";", "\"bytes\";\"mime\";\"mime/multipart\";");
    source.push_str(&format!(r#"
func TestNamedMultipart(t *testing.T){{
 calls:=0
 server:=httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter,r *http.Request){{calls++;data,err:=io.ReadAll(r.Body);if err!=nil{{t.Fatal(err)}};kind,params,err:=mime.ParseMediaType(r.Header.Get("Content-Type"));if err!=nil||kind!="multipart/form-data"{{t.Fatal(kind,err)}};reader:=multipart.NewReader(bytes.NewReader(data),params["boundary"]);values:=map[string][][]byte{{}};for{{part,err:=reader.NextPart();if err==io.EOF{{break}};if err!=nil{{t.Fatal(err)}};body,err:=io.ReadAll(part);if err!=nil{{t.Fatal(err)}};values[part.FormName()]=append(values[part.FormName()],body);if part.FormName()=="file"&&(part.FileName()!="snow 雪.png"||part.Header.Get("Content-Type")!="image/png"||part.Header.Get("X-Part-Id")!="p1"){{t.Fatal(part.Header)}}}};if !bytes.Equal(values["file"][0],[]byte{{0,255,10}})||string(values["metadata"][0])!=`{{"title":"native"}}`||len(values["files"])!=2||!bytes.Equal(values["files"][1],[]byte{{128,0}})||string(values["labels"][1])!="two"{{t.Fatal(values)}};w.Header().Set("Content-Type",r.Header.Get("Content-Type"));_,_=w.Write(data)}}));defer server.Close()
 client:=must(sdk.NewClient(sdk.Credentials{{}},sdk.ClientOptions{{ServerURL:server.URL}}));defer client.CloseIdleConnections()
 body:=sdk.{upload_ctor}(sdk.NewPart([]byte{{0,255,10}}).WithContentType("image/png").WithFilename("snow 雪.png").WithHeader("X-Part-Id","p1"),sdk.NewPart(sdk.New{metadata}("native"))).WithFiles([]sdk.Part[[]byte]{{sdk.NewPart([]byte{{}}),sdk.NewPart([]byte{{128,0}})}}).WithLabels([]sdk.Part[string]{{sdk.NewPart("one"),sdk.NewPart("two")}})
 response,err:=client.UploadData(context.Background(),sdk.NewUploadInput(body));if err!=nil{{t.Fatal(err)}};if !bytes.Equal(response.File.Data,[]byte{{0,255,10}})||response.Metadata.Data.Title!="native"||response.File.Filename!="snow 雪.png"||response.Labels.Value[1].Data!="two"||len(response.Files.Value)!=2{{t.Fatal(response)}}
 invalid:=body;invalid.File=sdk.NewPart([]byte{{1}}).WithContentType("image/png");_,err=client.Upload(context.Background(),sdk.NewUploadInput(invalid));failure:=sdkFailure(t,err,"request-validation");if !strings.Contains(failure.Source.Pointer,"X-Part-Id"){{t.Fatal(failure.Source)}}
 invalid=body;invalid.File.Data=[]byte{{0,1,2,3}};_,err=client.Upload(context.Background(),sdk.NewUploadInput(invalid));sdkFailure(t,err,"resource-limit")
 invalid=body;invalid.Labels=sdk.OptionalSome([]sdk.Part[string]{{sdk.NewPart("one"),sdk.NewPart("two"),sdk.NewPart("three")}});_,err=client.Upload(context.Background(),sdk.NewUploadInput(invalid));sdkFailure(t,err,"request-validation")
 invalid=body;invalid.Extras=map[string]sdk.Part[sdk.Integer]{{"extra":sdk.NewPart(must(sdk.ParseInteger("1")))}};_,err=client.Upload(context.Background(),sdk.NewUploadInput(invalid));sdkFailure(t,err,"request-validation")
 invalid=body;invalid.Extras=map[string]sdk.Part[sdk.Integer]{{"file":sdk.NewPart(must(sdk.ParseInteger("1")))}};_,err=client.Upload(context.Background(),sdk.NewUploadInput(invalid));sdkFailure(t,err,"request-validation")
 bounded:=must(sdk.NewClient(sdk.Credentials{{}},sdk.ClientOptions{{ServerURL:server.URL,MaxPartBytes:2}}));_,err=bounded.Upload(context.Background(),sdk.NewUploadInput(body));sdkFailure(t,err,"resource-limit")
 if calls!=1{{t.Fatal("invalid multipart reached transport",calls)}}
}}
func TestForm(t *testing.T){{
 client:=must(sdk.NewClient(sdk.Credentials{{}},sdk.ClientOptions{{Transport:doer(func(r *http.Request)(*http.Response,error){{data,err:=io.ReadAll(r.Body);if err!=nil{{t.Fatal(err)}};want:="address=%7B%22city%22%3A%22%E9%9B%AA%22%7D&codes%5BA%5D=1&codes%5BB%5D=2&id=a+b%2Bc&tags=one&tags=two";if string(data)!=want||r.Header.Get("Content-Type")!="application/x-www-form-urlencoded"{{t.Fatal(string(data),r.Header)}};return reply(200,r.Header.Get("Content-Type"),data),nil}})}}))
 value:=sdk.{form_ctor}("a b+c").WithAddress(sdk.New{address}("雪")).WithCodes(sdk.{codes}{{"B":must(sdk.ParseInteger("2")),"A":must(sdk.ParseInteger("1"))}}).WithTags([]string{{"one","two"}})
 response,err:=client.FormData(context.Background(),sdk.NewFormInput(value));if err!=nil{{t.Fatal(err)}};if response.Id!="a b+c"||response.Address.Value.City!="雪"||response.Codes.Value["A"].String()!="1"||!reflect.DeepEqual(response.Tags.Value,[]string{{"one","two"}}){{t.Fatal(response)}}
 invalid:=value;invalid.Id="";_,err=client.Form(context.Background(),sdk.NewFormInput(invalid));sdkFailure(t,err,"request-validation")
}}
func TestPositionalMultipart(t *testing.T){{calls:=0;client:=must(sdk.NewClient(sdk.Credentials{{}},sdk.ClientOptions{{Transport:doer(func(r *http.Request)(*http.Response,error){{calls++;data,err:=io.ReadAll(r.Body);if err!=nil{{t.Fatal(err)}};kind,params,err:=mime.ParseMediaType(r.Header.Get("Content-Type"));if err!=nil||kind!="multipart/mixed"{{t.Fatal(kind,err)}};reader:=multipart.NewReader(bytes.NewReader(data),params["boundary"]);first,err:=reader.NextPart();if err!=nil{{t.Fatal(err)}};prefix:=must(io.ReadAll(first));if string(prefix)!="first"||first.Header.Get("Content-Type")!="text/plain"||first.Header.Get("Content-Disposition")!=""{{t.Fatal(string(prefix),first.Header)}};item,err:=reader.NextPart();if err!=nil{{t.Fatal(err)}};content:=must(io.ReadAll(item));if string(content)!=`{{"n":1}}`||item.Header.Get("X-Seq")!="2"{{t.Fatal(string(content),item.Header)}};return reply(200,r.Header.Get("Content-Type"),data),nil}})}}));value:=sdk.{parts_ctor}(sdk.NewPart("first"));value.Items=[]sdk.Part[sdk.{item}]{{sdk.NewPart(sdk.New{item}(must(sdk.ParseInteger("1")))).WithHeader("X-Seq","2")}};response,err:=client.PartsData(context.Background(),sdk.NewPartsInput(value));if err!=nil{{t.Fatal(err)}};if response.Item0.Data!="first"||response.Items[0].Data.N.String()!="1"{{t.Fatal(response)}};value.Items[0].Data.N=must(sdk.ParseInteger("0"));_,err=client.Parts(context.Background(),sdk.NewPartsInput(value));sdkFailure(t,err,"request-validation");if calls!=1{{t.Fatal(calls)}}}}
func TestMalformedMultipartAndFormResponses(t *testing.T){{
 for _,wire:=range []string{{"--x\r\nContent-Disposition: form-data; name=\"file\"\r\nContent-Type: image/png\r\n\r\na\r\n--x--\r\n","--x\r\nContent-Disposition: form-data; name=\"unexpected\"\r\nContent-Type: text/plain\r\n\r\nnot-an-integer\r\n--x--\r\n"}}{{client:=must(sdk.NewClient(sdk.Credentials{{}},sdk.ClientOptions{{Transport:doer(func(*http.Request)(*http.Response,error){{return reply(200,"multipart/form-data;boundary=x",[]byte(wire)),nil}})}}));body:=sdk.{upload_ctor}(sdk.NewPart([]byte{{}}).WithContentType("image/png").WithHeader("X-Part-Id","p1"),sdk.NewPart(sdk.New{metadata}("native")));_,err:=client.Upload(context.Background(),sdk.NewUploadInput(body));sdkFailure(t,err,"response-decoding")}}
 for _,wire:=range []string{{"tags=one","id=ok&unexpected=value","id=ok&id=duplicate","id=ok&tags=a&tags=b&tags=c"}}{{client:=must(sdk.NewClient(sdk.Credentials{{}},sdk.ClientOptions{{Transport:doer(func(*http.Request)(*http.Response,error){{return reply(200,"application/x-www-form-urlencoded",[]byte(wire)),nil}})}}));_,err:=client.Form(context.Background(),sdk.NewFormInput(sdk.{form_ctor}("ok")));sdkFailure(t,err,"response-decoding")}}
}}
"#,upload_ctor=upload.constructor,metadata=metadata.data_type,form_ctor=form.constructor,address=address.data_type,codes=codes.data_type,parts_ctor=parts.constructor,item=parts.additional.as_ref().unwrap().data_type));
    native(
        &p,
        &source,
        &[
            &format!("var _ = sdk.{}()", upload.constructor),
            &format!(
                "var _ = sdk.{}(sdk.NewPart(\"not bytes\"), sdk.NewPart(sdk.New{}(\"ok\")))",
                upload.constructor, metadata.data_type
            ),
        ],
    );
}
#[test]
#[ignore = "requires installed native Go 1.23.12 and 1.27.1"]
fn native_sse_json_lines_context_owned_iteration_and_cleanup() {
    let mut spec = base();
    let event = json!({"type":"object","required":["data"],"properties":{"data":{"type":"string","maxLength":64},"id":{"type":"string"},"event":{"type":"string"},"retry":{"type":"integer","minimum":0}},"additionalProperties":false});
    let item = json!({"type":"object","required":["n"],"properties":{"n":{"type":"integer","minimum":1}},"additionalProperties":false});
    for (name, media, schema) in [
        ("events", "text/event-stream", event.clone()),
        ("lines", "application/x-ndjson", item.clone()),
    ] {
        spec["paths"][&format!("/{name}")] = json!({"get":{"operationId":name,"responses":{"200":{"content":{media:{"itemSchema":schema}}}}}});
    }
    for (name, media, schema) in [
        ("sendEvents", "text/event-stream", event),
        ("sendLines", "application/jsonl", item),
    ] {
        spec["paths"][&format!("/{name}")] = json!({"post":{"operationId":name,"requestBody":{"required":true,"content":{media:{"itemSchema":schema}}},"responses":{"204":{}}}});
    }
    let p = plan(spec);
    let mut source = PRELUDE.replace("\"bytes\";", "\"bytes\";\"runtime\";\"sync\";");
    source.push_str(r#"
type fragmentBody struct{data []byte;chunk,read int;closed atomic.Int32}
func(b *fragmentBody)Read(p []byte)(int,error){if len(b.data)==0{return 0,io.EOF};n:=b.chunk;if n>len(p){n=len(p)};if n>len(b.data){n=len(b.data)};copy(p,b.data[:n]);b.data=b.data[n:];b.read+=n;return n,nil}
func(b *fragmentBody)Close()error{b.closed.Add(1);return nil}
func TestSSEFramingAndLazyPull(t *testing.T){
 for chunk:=1;chunk<=9;chunk++{
  wire:="\uFEFF: comment\r\nid: id1\revent: custom\ndata: {\"x\":1}\rdata: 雪\nretry: 0005\nignored: skip\n\ndata: [DONE]\nretry: bogus\nid: bad\x00id\n\ndata: after\n\ndata\n\ndata: discarded"
  body:=&fragmentBody{data:[]byte(wire),chunk:chunk};calls:=0
  client:=must(sdk.NewClient(sdk.Credentials{},sdk.ClientOptions{Transport:doer(func(r *http.Request)(*http.Response,error){calls++;return &http.Response{StatusCode:200,Header:http.Header{"Content-Type":{"text/event-stream; charset=UTF-8"}},Body:body},nil})}))
  stream,err:=client.EventsData(context.Background());if err!=nil{t.Fatal(err)};if body.read!=0{t.Fatal("iterator eagerly read transport")}
  if !stream.Next(){t.Fatal(stream.Err())};event:=stream.Value();if event.Data!="{\"x\":1}\n雪"||!event.Id.IsSet||event.Id.Value!="id1"||event.Event.Value!="custom"||event.Retry.Value.String()!="5"{t.Fatal(event)}
  if !stream.Next(){t.Fatal(stream.Err())};event=stream.Value();if event.Data!="[DONE]"||event.Retry.IsSet||event.Id.IsSet{t.Fatal("sentinel/ignored-field inference",event)}
  if !stream.Next()||stream.Value().Data!="after"{t.Fatal("sentinel stopped stream",stream.Err())};if !stream.Next()||stream.Value().Data!=""{t.Fatal("empty data line lost",stream.Err())}
  if stream.Next()||stream.Err()!=nil{t.Fatal("unterminated event dispatched",stream.Err())};_=stream.Close();if body.closed.Load()!=1||calls!=1{t.Fatal("EOF cleanup/retry",body.closed.Load(),calls)}
 }
}
func TestLinesFramingAndExactNumbers(t *testing.T){
 for chunk:=1;chunk<8;chunk++{body:=&fragmentBody{data:[]byte("{\"n\":9007199254740993}\r\n{\"n\":2}"),chunk:chunk};client:=must(sdk.NewClient(sdk.Credentials{},sdk.ClientOptions{Transport:doer(func(*http.Request)(*http.Response,error){return &http.Response{StatusCode:200,Header:http.Header{"Content-Type":{"application/x-ndjson"}},Body:body},nil})}));stream:=must(client.LinesData(context.Background()));if !stream.Next()||stream.Value().N.String()!="9007199254740993"{t.Fatal(stream.Err())};if !stream.Next()||stream.Value().N.String()!="2"{t.Fatal(stream.Err())};if stream.Next()||stream.Err()!=nil||body.closed.Load()!=1{t.Fatal(stream.Err(),body.closed.Load())}}
 for _,wire:=range []string{"\n","[DONE]\n","{\"n\":0}\n","{\"n\":1,\"n\":2}\n"}{body:=&fragmentBody{data:[]byte(wire),chunk:1};client:=must(sdk.NewClient(sdk.Credentials{},sdk.ClientOptions{Transport:doer(func(*http.Request)(*http.Response,error){return &http.Response{StatusCode:200,Header:http.Header{"Content-Type":{"application/x-ndjson"}},Body:body},nil})}));stream:=must(client.LinesData(context.Background()));if stream.Next(){t.Fatal("invalid line accepted")};sdkFailure(t,stream.Err(),"response-decoding");if body.closed.Load()!=1{t.Fatal("invalid line leaked body")}}
}
func TestStreamBudgetsAndItemCodecs(t *testing.T){
 for _,test:=range []struct{wire string;item,total int;kind string}{
 {"data: "+strings.Repeat("x",65)+"\n\n",0,0,"response-decoding"},
 {"data: "+strings.Repeat("x",40)+"\n\n",16,0,"resource-limit"},
 {strings.Repeat("data: one\n\n",10),0,30,"resource-limit"},
  }{body:=&fragmentBody{data:[]byte(test.wire),chunk:1};client:=must(sdk.NewClient(sdk.Credentials{},sdk.ClientOptions{MaxStreamItemBytes:test.item,MaxResponseBytes:test.total,MaxCaptureBytes:4,Transport:doer(func(*http.Request)(*http.Response,error){return &http.Response{StatusCode:200,Header:http.Header{"Content-Type":{"text/event-stream"}},Body:body},nil})}));stream:=must(client.EventsData(context.Background()));for stream.Next(){};failure:=sdkFailure(t,stream.Err(),test.kind);if failure.Status!=200||len(failure.RawCapture)>4||!failure.Truncated||body.closed.Load()!=1{t.Fatal(failure,body.closed.Load())};if test.kind=="response-decoding"{var codec *sdk.CodecError;if !errors.As(stream.Err(),&codec)||!strings.Contains(codec.Source.Pointer,"/itemSchema"){t.Fatal("lost item codec",stream.Err())}}}
}
type blockedBody struct{started,closed chan struct{};once sync.Once;count atomic.Int32}
func(b *blockedBody)Read([]byte)(int,error){b.once.Do(func(){close(b.started)});<-b.closed;return 0,io.ErrClosedPipe}
func(b *blockedBody)Close()error{if b.count.Add(1)==1{close(b.closed)};return nil}
func TestStreamContextTimeoutAndEarlyClose(t *testing.T){
 for _,mode:=range []string{"cancel","close","timeout"}{body:=&blockedBody{started:make(chan struct{}),closed:make(chan struct{})};options:=sdk.ClientOptions{Transport:doer(func(r *http.Request)(*http.Response,error){if r.Context().Err()!=nil{t.Fatal("prematurely cancelled context")};return &http.Response{StatusCode:200,Header:http.Header{"Content-Type":{"text/event-stream"}},Body:body},nil})};if mode=="timeout"{options.Timeout=30*time.Millisecond};client:=must(sdk.NewClient(sdk.Credentials{},options));ctx,cancel:=context.WithCancel(context.Background());stream:=must(client.EventsData(ctx));done:=make(chan bool,1);go func(){done<-stream.Next()}();select{case<-body.started:case<-time.After(time.Second):t.Fatal("iterator did not pull")};if mode=="cancel"{cancel()}else if mode=="close"{_=stream.Close()};select{case got:=<-done:if got{t.Fatal("blocked frame yielded")};case<-time.After(time.Second):t.Fatal("cancel/close did not unblock reader")};want:=context.Canceled;if mode=="timeout"{want=context.DeadlineExceeded};if !errors.Is(stream.Err(),want){t.Fatal(mode,stream.Err())};sdkFailure(t,stream.Err(),"cancelled");_=stream.Close();cancel();if body.count.Load()!=1{t.Fatal("body closed multiple times",body.count.Load())}}
}
func TestNativeConnectionCancellation(t *testing.T){
 released:=make(chan struct{});server:=httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter,r *http.Request){w.Header().Set("Content-Type","text/event-stream");_,_=io.WriteString(w,"data: one\n\n");w.(http.Flusher).Flush();<-r.Context().Done();close(released)}));defer server.Close();client:=must(sdk.NewClient(sdk.Credentials{},sdk.ClientOptions{ServerURL:server.URL}));defer client.CloseIdleConnections();stream:=must(client.EventsData(context.Background()));if !stream.Next()||stream.Value().Data!="one"{t.Fatal(stream.Err())};_=stream.Close();select{case<-released:case<-time.After(time.Second):t.Fatal("stream close left native connection active")}
}
func TestEarlyCloseDoesNotLeakReadersOrTimers(t *testing.T){
 before:=runtime.NumGoroutine();var closed atomic.Int32
 client:=must(sdk.NewClient(sdk.Credentials{},sdk.ClientOptions{Timeout:time.Hour,Transport:doer(func(*http.Request)(*http.Response,error){return &http.Response{StatusCode:200,Header:http.Header{"Content-Type":{"text/event-stream"}},Body:countClose{Reader:strings.NewReader("data: unused\n\n"),count:&closed}},nil})}))
 for i:=0;i<100;i++{stream:=must(client.EventsData(context.Background()));_=stream.Close()};runtime.GC();if closed.Load()!=100{t.Fatal("close ownership",closed.Load())};if got:=runtime.NumGoroutine();got>before+8{t.Fatal("reader goroutine leak",before,got)}
}
type countClose struct{io.Reader;count *atomic.Int32}
func(b countClose)Close()error{b.count.Add(1);return nil}
"#);
    let event_model = &p.symbols()[op(&p, "sendEvents").body().unwrap().schema().unwrap()];
    let line_model = &p.symbols()[op(&p, "sendLines").body().unwrap().schema().unwrap()];
    source.push_str(&format!(r#"
func TestFiniteRequestStreams(t *testing.T){{calls:=0;client:=must(sdk.NewClient(sdk.Credentials{{}},sdk.ClientOptions{{Transport:doer(func(r *http.Request)(*http.Response,error){{calls++;data,err:=io.ReadAll(r.Body);if err!=nil{{t.Fatal(err)}};if calls==1{{if r.Header.Get("Content-Type")!="text/event-stream"||string(data)!="retry: 5\ndata: first\ndata: 雪\n\n"{{t.Fatal(string(data),r.Header)}}}}else if r.Header.Get("Content-Type")!="application/jsonl"||string(data)!="{{\"n\":2}}\n"{{t.Fatal(string(data),r.Header)}};return reply(204,"",nil),nil}})}}));event:=sdk.New{event_model}("first\n雪");event.Retry=sdk.OptionalSome(must(sdk.ParseInteger("5")));if _,err:=client.SendEvents(context.Background(),sdk.NewSendEventsInput([]sdk.{event_model}{{event}}));err!=nil{{t.Fatal(err)}};if _,err:=client.SendLines(context.Background(),sdk.NewSendLinesInput([]sdk.{line_model}{{sdk.New{line_model}(must(sdk.ParseInteger("2")))}}));err!=nil{{t.Fatal(err)}};event.Data=strings.Repeat("x",65);_,err:=client.SendEvents(context.Background(),sdk.NewSendEventsInput([]sdk.{event_model}{{event}}));sdkFailure(t,err,"request-validation");if calls!=2{{t.Fatal(calls)}}}}
"#));
    native(
        &p,
        &source,
        &["var _ = sdk.NewSendEventsInput([]string{\"not envelopes\"})"],
    );
}

#[test]
#[ignore = "requires OPENROUTER_WEB_ROOT and native Go 1.23.12/1.27.1"]
fn native_actual_openrouter_five_plus_crud_and_binary_operations() {
    let path =
        PathBuf::from(std::env::var_os("OPENROUTER_WEB_ROOT").expect("set OPENROUTER_WEB_ROOT"))
            .join("projects/docs/openapi/openapi.yaml");
    let contract = load(&path);
    let wanted = [
        "getCredits",
        "createKeys",
        "updateKeys",
        "listContainerFiles",
        "getContainerFile",
        "deleteKeys",
        "getKey",
        "listProviders",
        "listOauthJwks",
        "downloadContainerFileContent",
        "downloadFileContent",
        "createAudioSpeech",
    ];
    let selected = contract
        .operations()
        .filter(|o| wanted.contains(&o.operation_id().unwrap_or("")))
        .map(|o| o.source().clone())
        .collect::<Vec<_>>();
    assert_eq!(selected.len(), wanted.len());
    let default_errors =
        go_http::plan_http(contract.clone(), &selected, Default::default()).unwrap_err();
    assert!(
        default_errors
            .iter()
            .any(|e| e.code == "http-binary-legacy-marker")
    );
    let mut config = HttpConfig::default();
    config
        .compatibility_profiles
        .insert(CompatibilityProfile::LegacyBinaryStringV1);
    let p = go_http::plan_http(contract.clone(), &selected, config.clone()).unwrap();
    assert_eq!(p.operations().len(), 12);
    assert!(
        p.protocol()
            .diagnostics()
            .iter()
            .any(|d| d.code() == "http-compatibility-profile")
    );
    for name in ["uploadFile", "createOauthToken"] {
        let source = contract
            .operations()
            .find(|o| o.operation_id() == Some(name))
            .unwrap()
            .source()
            .clone();
        let failures = go_http::plan_http(contract.clone(), &[source], config.clone()).unwrap_err();
        assert!(
            failures
                .iter()
                .any(|e| e.code == "http-form-untyped-extras"),
            "{name}: {failures:?}"
        );
    }
    let chat = contract
        .operations()
        .find(|o| o.operation_id() == Some("sendChatCompletionRequest"))
        .unwrap()
        .source()
        .clone();
    let failures = go_http::plan_http(contract.clone(), &[chat], config).unwrap_err();
    assert!(
        failures
            .iter()
            .any(|e| e.code == "http-stream-item-schema-required"),
        "3.1 stream conventions must not activate: {failures:?}"
    );
    let fixture: Value =
        serde_json::from_str(include_str!("fixtures/openrouter-five-responses.json")).unwrap();
    let create = &p.symbols()[op(&p, "createKeys").body().unwrap().schema().unwrap()];
    let update = &p.symbols()[op(&p, "updateKeys").body().unwrap().schema().unwrap()];
    let speech = &p.symbols()[op(&p, "createAudioSpeech")
        .body()
        .unwrap()
        .schema()
        .unwrap()];
    let mut source = PRELUDE.to_owned();
    source.push_str(&format!("var corpus=map[string]string{{\"credits\":{},\"create\":{},\"update\":{},\"file\":{},\"list\":{}}}\n",q(fixture["credits"].as_str().unwrap()),q(fixture["create"].as_str().unwrap()),q(fixture["update"].as_str().unwrap()),q(fixture["file"].as_str().unwrap()),q(fixture["list"].as_str().unwrap())));
    source.push_str(&format!(r#"
func TestActualOpenRouterSelectedSlice(t *testing.T){{
 var seen [][4]string
 server:=httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter,r *http.Request){{data,err:=io.ReadAll(r.Body);if err!=nil{{t.Fatal(err)}};seen=append(seen,[4]string{{r.Method,r.RequestURI,r.Header.Get("Authorization"),string(data)}});w.Header().Set("Content-Type","application/json");path:=r.URL.Path
 switch{{
 case path=="/api/v1/credits":_,_=io.WriteString(w,corpus["credits"])
 case path=="/api/v1/keys"&&r.Method=="POST":w.WriteHeader(201);_,_=io.WriteString(w,corpus["create"])
 case strings.HasPrefix(path,"/api/v1/keys/")&&r.Method=="DELETE":_,_=io.WriteString(w,`{{"deleted":true}}`)
 case strings.HasPrefix(path,"/api/v1/keys/"):_,_=io.WriteString(w,corpus["update"])
 case path=="/api/v1/providers":_,_=io.WriteString(w,`{{"data":[]}}`)
 case path=="/api/v1/oauth/jwks":_,_=io.WriteString(w,`{{"keys":[{{"kty":"EC","crv":"P-256","kid":"fixture-key","x":"abc","y":"def","alg":"ES256","use":"sig"}}]}}`)
 case strings.HasSuffix(path,"/content")||path=="/api/v1/audio/speech":if path=="/api/v1/audio/speech"{{w.Header().Set("Content-Type","audio/pcm; rate=16000")}}else{{w.Header().Set("Content-Type","application/octet-stream")}};_,_=w.Write([]byte{{0,255,128,13,10,0}})
 case strings.HasSuffix(path,"/files"):_,_=io.WriteString(w,corpus["list"])
 default:_,_=io.WriteString(w,corpus["file"])
 }}
 }}));defer server.Close()
 client:=must(sdk.NewClient(sdk.ApiKey("fixture-token"),sdk.ClientOptions{{ServerURL:server.URL+"/api/v1"}}));defer client.CloseIdleConnections();ctx:=context.Background()
 credits,err:=client.GetCreditsData(ctx);if err!=nil||credits.Data.TotalCredits.String()!="100.50000000000000001"{{t.Fatal(credits,err)}}
 created:=must(sdk.Codecs.{create}.Decode([]byte(`{{"name":"Native Test Key","limit":50.250,"limit_reset":"monthly","include_byok_in_limit":true}}`)));if _,err=client.CreateKeys(ctx,sdk.NewCreateKeysInput(created));err!=nil{{t.Fatal(err)}}
 patched:=must(sdk.Codecs.{update}.Decode([]byte(`{{"name":"Updated Native Key","disabled":true,"limit":75.50,"limit_reset":"daily"}}`)));if _,err=client.UpdateKeys(ctx,sdk.NewUpdateKeysInput("fixture-hash",patched));err!=nil{{t.Fatal(err)}}
 if _,err=client.ListContainerFiles(ctx,sdk.NewListContainerFilesInput("sess_abc123").WithLimit(must(sdk.ParseInteger("2"))));err!=nil{{t.Fatal(err)}}
 if _,err=client.GetContainerFile(ctx,sdk.NewGetContainerFileInput("sess_abc123","cfile_1"));err!=nil{{t.Fatal(err)}}
 if _,err=client.GetKeyData(ctx,sdk.NewGetKeyInput("fixture-hash"));err!=nil{{t.Fatal(err)}}
 if _,err=client.DeleteKeysData(ctx,sdk.NewDeleteKeysInput("fixture-hash"));err!=nil{{t.Fatal(err)}}
 if _,err=client.ListProvidersData(ctx);err!=nil{{t.Fatal(err)}};if _,err=client.ListOauthJwksData(ctx);err!=nil{{t.Fatal(err)}}
 downloaded,err:=client.DownloadContainerFileContentData(ctx,sdk.NewDownloadContainerFileContentInput("sess_abc123","cfile_1"));if err!=nil||!bytes.Equal(downloaded,[]byte{{0,255,128,13,10,0}}){{t.Fatal(downloaded,err)}}
 downloaded,err=client.DownloadFileContentData(ctx,sdk.NewDownloadFileContentInput("or_file_1"));if err!=nil||!bytes.Equal(downloaded,[]byte{{0,255,128,13,10,0}}){{t.Fatal(downloaded,err)}}
 text:=must(sdk.Codecs.{speech}.Decode([]byte(`{{"model":"voice/model","input":"hello"}}`)));audio,err:=client.CreateAudioSpeechData(ctx,sdk.NewCreateAudioSpeechInput(text));if err!=nil||!bytes.Equal(audio,[]byte{{0,255,128,13,10,0}}){{t.Fatal(audio,err)}}
 if len(seen)!=12{{t.Fatal("retry/pagination inferred",seen)}}
 expected:=[][2]string{{{{"GET","/api/v1/credits"}},{{"POST","/api/v1/keys"}},{{"PATCH","/api/v1/keys/fixture-hash"}},{{"GET","/api/v1/containers/sess_abc123/files?limit=2"}},{{"GET","/api/v1/containers/sess_abc123/files/cfile_1"}},{{"GET","/api/v1/keys/fixture-hash"}},{{"DELETE","/api/v1/keys/fixture-hash"}},{{"GET","/api/v1/providers"}},{{"GET","/api/v1/oauth/jwks"}},{{"GET","/api/v1/containers/sess_abc123/files/cfile_1/content"}},{{"GET","/api/v1/files/or_file_1/content"}},{{"POST","/api/v1/audio/speech"}}}}
 for i,got:=range seen{{if got[0]!=expected[i][0]||got[1]!=expected[i][1]||got[2]!="Bearer fixture-token"{{t.Fatal(i,got,expected[i])}}}}
 if seen[1][3]!=`{{"include_byok_in_limit":true,"limit":50.250,"limit_reset":"monthly","name":"Native Test Key"}}`||seen[2][3]!=`{{"disabled":true,"limit":75.50,"limit_reset":"daily","name":"Updated Native Key"}}`||seen[11][3]!=`{{"input":"hello","model":"voice/model"}}`{{t.Fatal("exact request JSON changed",seen)}}
}}
"#));
    native(
        &p,
        &source,
        &["var _ = sdk.NewDownloadFileContentInput([]byte(\"not a path parameter\"))"],
    );
}

#[test]
#[ignore = "requires native Go 1.23.12 and 1.27.1"]
fn native_schema_free_content_header_codecs_part_styles_and_work_bounds() {
    let mut spec = base();
    for (name, media) in [("freeJSON", "application/json"), ("freeText", "text/plain")] {
        spec["paths"][&format!("/{name}")] = json!({"post":{"operationId":name,"requestBody":{"required":true,"content":{media:{}}},"responses":{"200":{"content":{media:{}}}}}});
    }
    spec["paths"]["/unknown"] = json!({"get":{"operationId":"unknown"}});
    spec["paths"]["/header"] = json!({"get":{
        "operationId":"header",
        "responses":{"204":{"headers":{
            "X-Count":{"required":true,"content":{"text/plain":{"schema":{"type":"integer","minimum":1}}}},
            "X-Object":{"required":true,"content":{"application/json":{"schema":{
                "type":"object","required":["n"],"properties":{"n":{"type":"integer"}},"additionalProperties":false
            }}}}
        }}}
    }});
    spec["components"]["schemas"]["Text"] = json!({"type":"string","minLength":1});
    let style = json!({"schema":{"type":"object","required":["plain = name","values"],"additionalProperties":false,"properties":{"plain = name":{"type":"string"},"values":{"type":"array","items":{"type":"string"}},"object":{"type":"object","additionalProperties":{"type":"integer"}}}},"encoding":{"plain = name":{"style":"form","headers":{"X-Part":{"required":true,"content":{"text/plain":{"schema":{"type":"integer"}}}},"Content-Disposition":{"required":true,"schema":{"type":"string","pattern":"^form-data;"}}}},"values":{"style":"form","explode":true},"object":{"style":"deepObject"}}});
    let one = json!({"schema":{"type":"object","required":["a long part name"],"additionalProperties":false,"properties":{"a long part name":{}}}});
    for (name, body) in [("style", style), ("one", one)] {
        spec["paths"][&format!("/{name}")] = json!({"post":{"operationId":name,"requestBody":{"required":true,"content":{"multipart/form-data":body.clone()}},"responses":{"200":{"content":{"multipart/form-data":body}}}}});
    }
    let p = plan(spec);
    let style = op(&p, "style").body().unwrap().media()[0]
        .aggregate
        .as_ref()
        .unwrap();
    let object = &style
        .parts
        .iter()
        .find(|p| p.wire.name() == Some("object"))
        .unwrap()
        .data_type;
    let one = op(&p, "one").body().unwrap().media()[0]
        .aggregate
        .as_ref()
        .unwrap();
    let mut source = PRELUDE.replace("\"bytes\";", "\"bytes\";\"mime\";\"mime/multipart\";");
    source.push_str(r#"
func TestSchemaFreeBodies(t *testing.T){
 calls:=0;client:=must(sdk.NewClient(sdk.Credentials{},sdk.ClientOptions{Transport:doer(func(r *http.Request)(*http.Response,error){calls++;data,err:=io.ReadAll(r.Body);if err!=nil{t.Fatal(err)};return reply(200,r.Header.Get("Content-Type"),data),nil})}))
 for _,value:=range []sdk.Value{nil,false,must(sdk.ParseNumber("9007199254740993.00001")),map[string]sdk.Value{"雪":"ok"}}{decoded,err:=client.FreeJSONData(context.Background(),sdk.NewFreeJSONInput(value));if err!=nil||!reflect.DeepEqual(decoded,value){t.Fatal(decoded,value,err)}}
 text,err:=client.FreeTextData(context.Background(),sdk.NewFreeTextInput("snow 雪\n"));if err!=nil||text!="snow 雪\n"{t.Fatal(text,err)}
 before:=calls;_,err=client.FreeText(context.Background(),sdk.NewFreeTextInput(string([]byte{255})));sdkFailure(t,err,"request-validation");if calls!=before{t.Fatal("invalid UTF-8 sent")}
}
func TestContentHeadersAndUndeclaredResponses(t *testing.T){client:=must(sdk.NewClient(sdk.Credentials{},sdk.ClientOptions{Transport:doer(func(r *http.Request)(*http.Response,error){response:=reply(204,"",nil);response.Header.Set("X-Count","7");response.Header.Set("X-Object",`{"n":9007199254740993}`);return response,nil})}));value,err:=client.Header(context.Background());if err!=nil{t.Fatal(err)};decoded:=value.(sdk.HeaderStatus204).DecodedHeaders;if decoded.XCount.String()!="7"||decoded.XObject.N.String()!="9007199254740993"{t.Fatal(decoded)};_=value.Close()
 client=must(sdk.NewClient(sdk.Credentials{},sdk.ClientOptions{Transport:doer(func(*http.Request)(*http.Response,error){return reply(200,"application/json",[]byte(`"undeclared"`)),nil})}));_,err=client.Unknown(context.Background());failure:=sdkFailure(t,err,"unexpected-response");if failure.Status!=200{t.Fatal(failure)}
}
"#);
    source.push_str(&format!(r#"
func TestMultipartStyleLiteralNamesAndValues(t *testing.T){{calls:=0;client:=must(sdk.NewClient(sdk.Credentials{{}},sdk.ClientOptions{{Transport:doer(func(r *http.Request)(*http.Response,error){{calls++;data:=must(io.ReadAll(r.Body));_,params,err:=mime.ParseMediaType(r.Header.Get("Content-Type"));if err!=nil{{t.Fatal(err)}};reader:=multipart.NewReader(bytes.NewReader(data),params["boundary"]);fields:=map[string][]string{{}};for{{part,err:=reader.NextRawPart();if err==io.EOF{{break}};if err!=nil{{t.Fatal(err)}};fields[part.FormName()]=append(fields[part.FormName()],string(must(io.ReadAll(part))))}};want:=map[string][]string{{"plain = name":{{"a&b=c + 雪"}},"object[x]":{{"1"}},"object[y]":{{"2"}},"values":{{"red,blue","a&b=x"}}}};if !reflect.DeepEqual(fields,want){{t.Fatal(fields,want)}};return reply(200,r.Header.Get("Content-Type"),data),nil}})}}))
 body:=sdk.{constructor}(sdk.NewPart("a&b=c + 雪").WithHeader("X-Part","7"),[]sdk.Part[string]{{sdk.NewPart("red,blue"),sdk.NewPart("a&b=x")}}).WithObject(sdk.NewPart(sdk.{object}{{"x":must(sdk.ParseInteger("1")),"y":must(sdk.ParseInteger("2"))}}))
 value,err:=client.StyleData(context.Background(),sdk.NewStyleInput(body));if err!=nil{{t.Fatal(err)}};if value.PlainName.Data!="a&b=c + 雪"||value.Object.Value.Data["x"].String()!="1"||value.Values[0].Data!="red,blue"{{t.Fatal(value)}}
 body.Values=make([]sdk.Part[string],4097);_,err=client.Style(context.Background(),sdk.NewStyleInput(body));sdkFailure(t,err,"resource-limit");if calls!=1{{t.Fatal("part work limit exceeded after transport")}}
}}
func TestPartByteBoundaryExcludesHeaderOverhead(t *testing.T){{client:=must(sdk.NewClient(sdk.Credentials{{}},sdk.ClientOptions{{MaxPartBytes:1,Transport:doer(func(r *http.Request)(*http.Response,error){{data:=must(io.ReadAll(r.Body));return reply(200,r.Header.Get("Content-Type"),data),nil}})}}));body:=sdk.{one_ctor}(sdk.NewPart([]byte{{255}}));result,err:=client.OneData(context.Background(),sdk.NewOneInput(body));if err!=nil||!bytes.Equal(result.ALongPartName.Data,[]byte{{255}}){{t.Fatal(result,err)}}}}
"#,constructor=style.constructor,one_ctor=one.constructor));
    native(
        &p,
        &source,
        &[
            "var _ = sdk.NewFreeTextInput([]byte(\"not text\"))",
            "var _ = sdk.NewFreeJSONInput()",
        ],
    );

    let mut spec = base();
    spec["openapi"] = json!("3.0.3");
    spec["paths"]["/blob"] = json!({"post":{"operationId":"blob","requestBody":{"required":true,"content":{"application/octet-stream":{"schema":{"type":"string","format":"binary"}}}},"responses":{"200":{"description":"binary","content":{"application/octet-stream":{"schema":{"type":"string","format":"binary"}}}}}}});
    let p = plan(spec);
    assert!(p.protocol().codec_roots().is_empty());
    native(
        &p,
        &format!(
            "{PRELUDE}{}",
            r#"func TestNativeOAS30Binary(t *testing.T){client:=must(sdk.NewClient(sdk.Credentials{},sdk.ClientOptions{Transport:doer(func(r *http.Request)(*http.Response,error){data:=must(io.ReadAll(r.Body));return reply(200,"application/octet-stream",data),nil})}));value,err:=client.BlobData(context.Background(),sdk.NewBlobInput([]byte{0,255}));if err!=nil||!bytes.Equal(value,[]byte{0,255}){t.Fatal(value,err)}}"#
        ),
        &["var _ = sdk.NewBlobInput(\"binary is bytes\")"],
    );
}

#[test]
#[ignore = "requires native Go and Sphinx 8.2.3 (SUSPECT_SPHINX_PYTHON)"]
fn native_protocol_documentation_builds_from_real_symbols() {
    let core: Value = serde_json::from_str(CORE).unwrap();
    let mut spec = base();
    spec["components"] = core["baseline"]["components"].clone();
    for key in ["baseline", "multipart", "stream"] {
        for (path, value) in core[key]["paths"].as_object().unwrap() {
            spec["paths"][path] = value.clone();
        }
    }
    let p = plan(spec);
    let root = tempfile::Builder::new()
        .prefix("suspect-go-protocol-docs-")
        .tempdir()
        .unwrap()
        .keep();
    suspect_codegen::write_files(&p.render(), &root).unwrap();
    let python = std::env::var_os("SUSPECT_SPHINX_PYTHON")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../target/sdk-native-python-tools/bin/python")
        });
    let output = Command::new(python)
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
        .env("SUSPECT_GO_TOOLCHAIN", "go1.23.12")
        .current_dir(root.join("go"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}{}",
        root.display(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let coverage: Value = serde_json::from_slice(
        &std::fs::read(root.join("go/docs/_build/html/coverage.json")).unwrap(),
    )
    .unwrap();
    let names = coverage["documented"].as_array().unwrap();
    for expected in [
        "Client.GetItemData",
        "Part.WithHeader",
        "Stream.Close",
        "CredentialHook",
        "APIResponse.Close",
    ] {
        assert!(
            names.iter().any(|n| n == expected),
            "missing native docs: {expected}"
        );
    }
    assert!(
        p.render()
            .iter()
            .find(|f| f.path == "go/README.md")
            .unwrap()
            .content
            .contains("NewGetItemInput")
    );
    eprintln!(
        "Go 1.23.12 / Sphinx: {} documented symbols",
        coverage["plannedSymbols"]
    );
    std::fs::remove_dir_all(root).unwrap();
}

const RESPONSE_TAIL: &str = r#"
func TestForbiddenAndUnspecifiedBodies(t *testing.T){
 for _,status:=range []int{200,500}{body:=&neverRead{};client:=must(sdk.NewClient(sdk.Credentials{},sdk.ClientOptions{Transport:doer(func(*http.Request)(*http.Response,error){return &http.Response{StatusCode:status,Header:http.Header{"Content-Type":{"not a media type"}},Body:body},nil})}));_,err:=client.Head(context.Background());if status==200&&err!=nil{t.Fatal(err)};if status==500{sdkFailure(t,err,"unexpected-response")};if !body.closed||body.reads!=0{t.Fatal("HEAD body touched/leaked",body)}}
 bytesBody:=[]byte{0,255,128,1};client:=must(sdk.NewClient(sdk.Credentials{},sdk.ClientOptions{Transport:doer(func(*http.Request)(*http.Response,error){return reply(200,"not a media type",bytesBody),nil})}));value,err:=client.BytesData(context.Background());if err!=nil||!bytes.Equal(value,bytesBody){t.Fatal(value,err)}
 bounded:=must(sdk.NewClient(sdk.Credentials{},sdk.ClientOptions{MaxResponseBytes:3,MaxCaptureBytes:2,Transport:doer(func(*http.Request)(*http.Response,error){return reply(200,"",bytesBody),nil})}));_,err=bounded.Bytes(context.Background());failure:=sdkFailure(t,err,"resource-limit");if failure.Status!=200||len(failure.RawCapture)!=2||!failure.Truncated{t.Fatal(failure)}
}
func TestBinaryCeilings(t *testing.T){calls:=0;client:=must(sdk.NewClient(sdk.Credentials{},sdk.ClientOptions{Transport:doer(func(r *http.Request)(*http.Response,error){calls++;data,err:=io.ReadAll(r.Body);if err!=nil{t.Fatal(err)};if r.Header.Get("Content-Type")!="application/octet-stream"||r.GetBody!=nil{t.Fatal(r)};return reply(200,"application/octet-stream",data),nil})}));value,err:=client.FiniteData(context.Background(),sdk.NewFiniteInput([]byte{0,255,128}));if err!=nil||!bytes.Equal(value,[]byte{0,255,128}){t.Fatal(value,err)};_,err=client.Finite(context.Background(),sdk.NewFiniteInput([]byte{0,1,2,3}));sdkFailure(t,err,"resource-limit");if calls!=1{t.Fatal(calls)};oversized:=must(sdk.NewClient(sdk.Credentials{},sdk.ClientOptions{Transport:doer(func(*http.Request)(*http.Response,error){return reply(200,"application/octet-stream",[]byte{0,1,2,3}),nil})}));_,err=oversized.Finite(context.Background(),sdk.NewFiniteInput(nil));sdkFailure(t,err,"resource-limit")}
"#;
