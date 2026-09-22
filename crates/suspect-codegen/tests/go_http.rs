//! Installed native Go module consuming the same independent M2 contract.
use std::{path::Path, process::Command, sync::Arc};
use suspect_codegen::go_http::{HttpConfig, PackageConfig, emit_http, plan_http};
use suspect_ir::contract::Contract;
use suspect_ref::WorkspaceBuilder;
use suspect_source::Uri;
fn plan() -> suspect_codegen::go_http::HttpPlan {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/m2/canonical.openapi.yaml");
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
        .map(|op| op.source().clone())
        .collect::<Vec<_>>();
    plan_http(contract, &selected, HttpConfig::default()).unwrap()
}
#[test]
fn module_identity_is_validated_before_artifacts() {
    let plan = plan();
    assert!(
        emit_http(
            &plan,
            &PackageConfig {
                module_path: "../escape".into(),
                package_name: "sdk".into(),
                version: "0.0.0".into()
            }
        )
        .is_err()
    );
    let files = emit_http(
        &plan,
        &PackageConfig {
            module_path: "example.com/m3-sdk".into(),
            package_name: "sdk".into(),
            version: "0.0.0".into(),
        },
    )
    .unwrap();
    for path in [
        "go/go.mod",
        "go/models.go",
        "go/codecs.go",
        "go/operations.go",
        "go/http_runtime.go",
        "go/http-manifest.json",
    ] {
        assert!(files.iter().any(|file| file.path == path), "{path}");
    }
}
#[test]
#[ignore = "requires native Go"]
fn independent_module_consumer_calls_all_operations_and_cancels() {
    let root = tempfile::tempdir().unwrap().keep();
    let files = emit_http(
        &plan(),
        &PackageConfig {
            module_path: "example.com/m3-sdk".into(),
            package_name: "sdk".into(),
            version: "0.0.0".into(),
        },
    )
    .unwrap();
    suspect_codegen::write_files(&files, &root).unwrap();
    let consumer = root.join("consumer");
    std::fs::create_dir(&consumer).unwrap();
    std::fs::write(consumer.join("go.mod"),"module example.com/consumer\n\ngo 1.23\nrequire example.com/m3-sdk v0.0.0\nreplace example.com/m3-sdk => ../go\n").unwrap();
    std::fs::write(consumer.join("client_test.go"), CONSUMER).unwrap();
    let mut command = Command::new("go");
    command
        .args(["test", "./..."])
        .current_dir(&consumer)
        .env("GOWORK", "off");
    if let Some(toolchain) = std::env::var_os("SUSPECT_GO_TOOLCHAIN") {
        command.env("GOTOOLCHAIN", toolchain);
    }
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "{}\n{}{}",
        root.display(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let output = Command::new("go")
        .args(["doc", "-all", "example.com/m3-sdk"])
        .current_dir(&consumer)
        .env("GOWORK", "off")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("CreateWidgetStatus200"));
    std::fs::remove_dir_all(root).unwrap();
}
const CONSUMER: &str = r#"package consumer
import("testing";"context";"net/http";"net/http/httptest";"io";"sync";"reflect";"errors";"time";sdk "example.com/m3-sdk")
func TestWire(t *testing.T){
 const widget=`{"id":"w1","amount":9007199254740993.000000000000000001,"meta":null,"payload":{"kind":"standard","text":"ok"},"child":{"label":"root"}}`
 const page=`{"items":[{"id":"w2","amount":1e-400,"payload":{"kind":"secure","vault":"v1"}}]}`
 var mu sync.Mutex;var seen [][4]string
 server:=httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter,r *http.Request){
  body,_:=io.ReadAll(r.Body);mu.Lock();seen=append(seen,[4]string{r.Method,r.RequestURI,r.Header.Get("Authorization"),string(body)});mu.Unlock()
  w.Header().Set("Content-Type","Application/JSON; charset=\"utf-8\"")
  if string(body)==`{"name":"deny"}`{w.WriteHeader(422);_,_=io.WriteString(w,`{"message":"rejected"}`);return}
  if r.URL.RawQuery!=""{_,_=io.WriteString(w,page)}else{_,_=io.WriteString(w,widget)}
 }));defer server.Close()
 client,err:=sdk.NewClient(sdk.ApiKey("test-key"),sdk.ClientOptions{ServerURL:server.URL+"/api/v1"});if err!=nil{t.Fatal(err)};defer client.CloseIdleConnections()
 ctx:=context.Background()
 result,err:=client.CreateWidget(ctx,sdk.NewCreateWidgetInput(sdk.NewWidgetInput("alpha")));if err!=nil{t.Fatal(err)}
 created:=result.(sdk.CreateWidgetStatus200)
 if created.Data.Amount.String()!="9007199254740993.000000000000000001"||!created.Data.Meta.Null{t.Fatal("exact model changed")}
 limit,_:=sdk.ParseInteger("2")
 listed,err:=client.ListWidgets(ctx,sdk.NewListWidgetsInput().WithTag("a").WithTags([]string{"x","y"}).WithLabels([]string{"a,b","c"}).WithLimit(limit));if err!=nil{t.Fatal(err)}
 if listed.(sdk.ListWidgetsStatus200).Data.Items[0].Amount.String()!="1e-400"{t.Fatal("rounded")}
 _,err=client.GetWidget(ctx,sdk.NewGetWidgetInput("a/b 雪!'()*"));if err!=nil{t.Fatal(err)}
 _,err=client.UpdateWidget(ctx,sdk.NewUpdateWidgetInput("w1",sdk.NewWidgetPatch()));if err!=nil{t.Fatal(err)}
 expected:=[][4]string{
 {"POST","/api/v1/widgets","Bearer test-key",`{"name":"alpha"}`},
 {"GET","/api/v1/widgets?tag=a&tags=x&tags=y&labels=a%2Cb,c&limit=2","Bearer test-key",""},
 {"GET","/api/v1/widgets/a%2Fb%20%E9%9B%AA%21%27%28%29%2A","Bearer test-key",""},
 {"PATCH","/api/v1/widgets/w1","Bearer test-key",`{}`},
 };if !reflect.DeepEqual(seen,expected){t.Fatalf("wire mismatch %#v",seen)}
 invalid:=sdk.NewWidgetInput("a");invalid.Name=""
 if _,err=client.CreateWidget(ctx,sdk.NewCreateWidgetInput(invalid));err==nil{t.Fatal("invalid mutable body sent")}
 cancelled,cancel:=context.WithCancel(ctx);cancel()
 if _,err=client.GetWidget(cancelled,sdk.NewGetWidgetInput("cancelled"));!errors.Is(err,context.Canceled){t.Fatal("cancellation lost",err)}
 if len(seen)!=4{t.Fatal("invalid/cancelled call reached transport")}
 _,err=client.CreateWidget(ctx,sdk.NewCreateWidgetInput(sdk.NewWidgetInput("deny")));var api *sdk.CreateWidgetStatus422
 if !errors.As(err,&api)||api.Data.Message!="rejected"{t.Fatal("typed API error missing",err)}
 bounded,err:=sdk.NewClient(sdk.ApiKey("test-key"),sdk.ClientOptions{ServerURL:server.URL+"/api/v1",MaxResponseBytes:8,MaxCaptureBytes:4});if err!=nil{t.Fatal(err)}
 _,err=bounded.GetWidget(ctx,sdk.NewGetWidgetInput("limited"));var failure *sdk.SDKError
 if !errors.As(err,&failure)||failure.Kind!="resource-limit"||failure.Status!=200||!failure.Truncated||len(failure.RawCapture)>4{t.Fatal("response ceiling failed",err)}
}
func TestCancellationWhileReadingBody(t *testing.T){
 started:=make(chan struct{});closed:=make(chan struct{})
 server:=httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter,r *http.Request){w.Header().Set("Content-Type","application/json");_,_=io.WriteString(w,"{");w.(http.Flusher).Flush();close(started);<-r.Context().Done();close(closed)}));defer server.Close()
 client,err:=sdk.NewClient(sdk.ApiKey("test-key"),sdk.ClientOptions{ServerURL:server.URL});if err!=nil{t.Fatal(err)};defer client.CloseIdleConnections()
 ctx,cancel:=context.WithCancel(context.Background());done:=make(chan error,1)
 go func(){_,err:=client.GetWidget(ctx,sdk.NewGetWidgetInput("body"));done<-err}()
 select{case<-started:case<-time.After(3*time.Second):t.Fatal("request never reached server")};cancel()
 select{case err:=<-done:if !errors.Is(err,context.Canceled){t.Fatal("cancellation cause lost",err)};case<-time.After(3*time.Second):t.Fatal("body read did not cancel")}
 select{case<-closed:case<-time.After(3*time.Second):t.Fatal("connection remained open after cancellation")}
}
"#;
