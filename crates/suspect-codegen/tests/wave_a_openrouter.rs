//! Primary-corpus Python/Go consumers use independent hand-authored HTTP bytes.
use std::{
    path::{Path, PathBuf},
    process::Command,
    sync::Arc,
};
use suspect_ir::contract::{Contract, SourceId};
use suspect_ref::WorkspaceBuilder;
use suspect_source::Uri;
const WANTED: &[&str] = &[
    "getCredits",
    "createKeys",
    "updateKeys",
    "listContainerFiles",
    "getContainerFile",
];
fn source() -> (Arc<Contract>, Vec<SourceId>) {
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
        .filter(|op| WANTED.contains(&op.operation_id().unwrap_or_default()))
        .map(|op| op.source().clone())
        .collect::<Vec<_>>();
    assert_eq!(selected.len(), 5);
    (contract, selected)
}
fn checked(command: &mut Command, root: &Path) {
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "{}\n{command:?}\n{}{}",
        root.display(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
fn body(contract: &Contract, id: &str) -> SourceId {
    contract
        .operations()
        .find(|op| op.operation_id() == Some(id))
        .unwrap()
        .request_body()
        .unwrap()
        .content()[0]
        .schema()
        .unwrap()
        .id()
        .clone()
}
#[test]
#[ignore = "requires tracked public corpus, Python wheel tools and native Python"]
fn installed_python_five_operations_preserve_the_actual_wire_contract() {
    let (contract, selected) = source();
    let plan =
        suspect_codegen::python_http::plan_http(contract.clone(), &selected, Default::default())
            .unwrap();
    let name = |id: &str| {
        plan.codecs()
            .models()
            .symbols()
            .iter()
            .find(|symbol| symbol.source() == &body(&contract, id))
            .unwrap()
            .name()
            .to_owned()
    };
    let create = name("createKeys");
    let update = name("updateKeys");
    let root = tempfile::tempdir().unwrap().keep();
    let files = suspect_codegen::python_http::emit_http(
        &plan,
        &suspect_codegen::python_http::PackageConfig {
            name: "wave-a-probe".into(),
            version: "0.0.0".into(),
            import_name: "wave_a_probe".into(),
        },
    )
    .unwrap();
    suspect_codegen::write_files(&files, &root).unwrap();
    let tools = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/sdk-native-python-tools/bin/python");
    checked(
        Command::new(&tools)
            .args(["-m", "build", "--wheel", "--no-isolation"])
            .current_dir(root.join("python")),
        &root,
    );
    let wheel = std::fs::read_dir(root.join("python/dist"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|file| file.extension().is_some_and(|ext| ext == "whl"))
        .unwrap();
    let interpreter = std::env::var_os("SUSPECT_PYTHON_BIN").unwrap_or_else(|| "python3".into());
    checked(
        Command::new("uv")
            .args(["venv", "--python"])
            .arg(interpreter)
            .arg(root.join("venv")),
        &root,
    );
    let python = root.join("venv/bin/python");
    checked(
        Command::new("uv")
            .args(["pip", "install", "--offline", "--python"])
            .arg(&python)
            .arg(&wheel),
        &root,
    );
    std::fs::write(
        root.join("responses.json"),
        include_str!("fixtures/openrouter-five-responses.json"),
    )
    .unwrap();
    std::fs::write(
        root.join("consumer.py"),
        PYTHON
            .replace("__CREATE__", &create)
            .replace("__UPDATE__", &update),
    )
    .unwrap();
    checked(
        Command::new(&python).arg("consumer.py").current_dir(&root),
        &root,
    );
    std::fs::remove_dir_all(root).unwrap();
}
#[test]
#[ignore = "requires tracked public corpus and native Go"]
fn native_go_five_operations_preserve_the_actual_wire_contract() {
    let (contract, selected) = source();
    let plan = suspect_codegen::go_http::plan_http(contract.clone(), &selected, Default::default())
        .unwrap();
    let name = |id: &str| {
        plan.codecs()
            .models()
            .symbols()
            .iter()
            .find(|symbol| symbol.source() == &body(&contract, id))
            .unwrap()
            .name()
            .to_owned()
    };
    let create = name("createKeys");
    let update = name("updateKeys");
    let root = tempfile::tempdir().unwrap().keep();
    suspect_codegen::write_files(&plan.render(), &root).unwrap();
    let consumer = root.join("consumer");
    std::fs::create_dir(&consumer).unwrap();
    std::fs::write(consumer.join("go.mod"),"module example.com/consumer\n\ngo 1.23\nrequire example.com/generated-sdk v0.0.0\nreplace example.com/generated-sdk => ../go\n").unwrap();
    std::fs::write(
        consumer.join("responses.json"),
        include_str!("fixtures/openrouter-five-responses.json"),
    )
    .unwrap();
    std::fs::write(
        consumer.join("client_test.go"),
        GO.replace("__CREATE__", &create)
            .replace("__UPDATE__", &update),
    )
    .unwrap();
    let mut command = Command::new("go");
    command
        .args(["test", "./..."])
        .current_dir(&consumer)
        .env("GOWORK", "off");
    if let Some(toolchain) = std::env::var_os("SUSPECT_GO_TOOLCHAIN") {
        command.env("GOTOOLCHAIN", toolchain);
    }
    checked(&mut command, &root);
    std::fs::remove_dir_all(root).unwrap();
}
const PYTHON: &str = r#"
import json, threading, asyncio
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from wave_a_probe import Client, AsyncClient, models
from wave_a_probe.json_runtime import JsonNumber
data=json.load(open('responses.json'));seen=[]
class Handler(BaseHTTPRequestHandler):
    def log_message(self,*args): pass
    def handle_request(self):
        body=self.rfile.read(int(self.headers.get('Content-Length','0')))
        seen.append((self.command,self.path,self.headers.get('Authorization'),body))
        which='create' if self.command=='POST' else 'update' if self.command=='PATCH' else 'credits' if self.path.endswith('/credits') else 'list' if '?' in self.path else 'file'
        raw=data[which].encode();self.send_response(201 if which=='create' else 200);self.send_header('Content-Type','application/json');self.send_header('Content-Length',str(len(raw)));self.end_headers();self.wfile.write(raw)
    do_GET=handle_request;do_POST=handle_request;do_PATCH=handle_request
server=ThreadingHTTPServer(('127.0.0.1',0),Handler);thread=threading.Thread(target=server.serve_forever,daemon=True);thread.start()
base=f'http://127.0.0.1:{server.server_port}/api/v1'
try:
    with Client(auth={'apiKey':'test-key'},server_url=base) as client:
        assert client.get_credits().data.data.total_credits.token=='100.50000000000000001'
        created=client.create_keys(body=models.__CREATE__(name='Native Test Key',limit=JsonNumber('50.25'),limit_reset=None))
        assert created.status==201 and created.data.data.limit.token=='50.250'
        updated=client.update_keys(hash='fixture-hash',body=models.__UPDATE__(name='Updated Native Key',limit=JsonNumber('75.50'),limit_reset=None,disabled=True))
        assert updated.data.data.limit.token=='75.50'
        assert client.list_container_files(container_id='sess_abc123',limit=2,after='a/b 雪').data.has_more is False
        assert client.get_container_file(container_id='sess_abc123',file_id='a/b 雪').data.bytes==123
    assert seen==[
      ('GET','/api/v1/credits','Bearer test-key',b''),
      ('POST','/api/v1/keys','Bearer test-key',b'{"limit":50.25,"limit_reset":null,"name":"Native Test Key"}'),
      ('PATCH','/api/v1/keys/fixture-hash','Bearer test-key',b'{"disabled":true,"limit":75.50,"limit_reset":null,"name":"Updated Native Key"}'),
      ('GET','/api/v1/containers/sess_abc123/files?limit=2&after=a%2Fb%20%E9%9B%AA','Bearer test-key',b''),
      ('GET','/api/v1/containers/sess_abc123/files/a%2Fb%20%E9%9B%AA','Bearer test-key',b''),
    ],seen
    async def run():
        async with AsyncClient(auth={'apiKey':'test-key'},server_url=base) as client:
            assert (await client.get_credits()).data.data.total_usage.token=='25.75'
    asyncio.run(run())
finally: server.shutdown();server.server_close();thread.join()
"#;
const GO: &str = r#"package consumer
import("context";"encoding/json";"io";"net/http";"net/http/httptest";"os";"testing";"reflect";sdk "example.com/generated-sdk")
func TestFive(t *testing.T){
 data,err:=os.ReadFile("responses.json");if err!=nil{t.Fatal(err)};var fixtures map[string]string;if err=json.Unmarshal(data,&fixtures);err!=nil{t.Fatal(err)}
 var seen [][4]string
 server:=httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter,r *http.Request){body,_:=io.ReadAll(r.Body);seen=append(seen,[4]string{r.Method,r.RequestURI,r.Header.Get("Authorization"),string(body)});which:="file";switch{case r.Method=="POST":which="create";case r.Method=="PATCH":which="update";case r.URL.Path=="/api/v1/credits":which="credits";case r.URL.RawQuery!="":which="list"};w.Header().Set("Content-Type","application/json");if which=="create"{w.WriteHeader(201)};_,_=io.WriteString(w,fixtures[which])}));defer server.Close()
 client,err:=sdk.NewClient(sdk.ApiKey("test-key"),sdk.ClientOptions{ServerURL:server.URL+"/api/v1"});if err!=nil{t.Fatal(err)};defer client.CloseIdleConnections();ctx:=context.Background()
 credits,err:=client.GetCredits(ctx,sdk.NewGetCreditsInput());if err!=nil{t.Fatal(err)};if credits.(sdk.GetCreditsStatus200).Data.Data.TotalCredits.String()!="100.50000000000000001"{t.Fatal("precision")}
 body:=sdk.New__CREATE__("Native Test Key");number,_:=sdk.ParseNumber("50.25");body.Limit.IsSet=true;body.Limit.Value=number;body.LimitReset.IsSet=true;body.LimitReset.Null=true
 created,err:=client.CreateKeys(ctx,sdk.NewCreateKeysInput(body));if err!=nil{t.Fatal(err)};if created.(sdk.CreateKeysStatus201).Data.Data.Limit.Value.String()!="50.250"{t.Fatal("create exact limit")}
 update:=sdk.New__UPDATE__();update.Name.IsSet=true;update.Name.Value="Updated Native Key";update.Disabled.IsSet=true;update.Disabled.Value=true;number,_=sdk.ParseNumber("75.50");update.Limit.IsSet=true;update.Limit.Value=number;update.LimitReset.IsSet=true;update.LimitReset.Null=true
 _,err=client.UpdateKeys(ctx,sdk.NewUpdateKeysInput("fixture-hash",update));if err!=nil{t.Fatal(err)}
 limit,_:=sdk.ParseInteger("2");_,err=client.ListContainerFiles(ctx,sdk.NewListContainerFilesInput("sess_abc123").WithLimit(limit).WithAfter("a/b 雪"));if err!=nil{t.Fatal(err)}
 file,err:=client.GetContainerFile(ctx,sdk.NewGetContainerFileInput("sess_abc123","a/b 雪"));if err!=nil{t.Fatal(err)};if file.(sdk.GetContainerFileStatus200).Data.Bytes.String()!="123"{t.Fatal("file bytes")}
 expected:=[][4]string{{"GET","/api/v1/credits","Bearer test-key",""},{"POST","/api/v1/keys","Bearer test-key",`{"limit":50.25,"limit_reset":null,"name":"Native Test Key"}`},{"PATCH","/api/v1/keys/fixture-hash","Bearer test-key",`{"disabled":true,"limit":75.50,"limit_reset":null,"name":"Updated Native Key"}`},{"GET","/api/v1/containers/sess_abc123/files?limit=2&after=a%2Fb%20%E9%9B%AA","Bearer test-key",""},{"GET","/api/v1/containers/sess_abc123/files/a%2Fb%20%E9%9B%AA","Bearer test-key",""}};if !reflect.DeepEqual(seen,expected){t.Fatalf("wire %#v",seen)}
}
"#;
