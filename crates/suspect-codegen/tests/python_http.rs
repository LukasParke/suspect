//! Native installed-wheel gate for source-selected sync/async Python clients.
use std::{
    path::{Path, PathBuf},
    process::Command,
    sync::Arc,
};
use suspect_codegen::python_http::{HttpConfig, PackageConfig, emit_http, plan_http};
use suspect_ir::contract::Contract;
use suspect_ref::WorkspaceBuilder;
use suspect_source::Uri;

fn plan() -> suspect_codegen::python_http::HttpPlan {
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
fn checked(command: &mut Command, root: &Path) -> std::process::Output {
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "{}\n{command:?}\n{}{}",
        root.display(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    output
}
#[test]
fn wheel_identity_is_configuration_and_invalid_names_fail_before_emission() {
    let plan = plan();
    for name in ["../escape", "httpx", "not-a-module"] {
        assert!(
            emit_http(
                &plan,
                &PackageConfig {
                    name: "test-sdk".into(),
                    version: "0.0.0".into(),
                    import_name: name.into()
                }
            )
            .is_err()
        );
    }
    for version in ["1.0.0-foo", "1.0.0+build-"] {
        assert!(
            emit_http(
                &plan,
                &PackageConfig {
                    name: "test-sdk".into(),
                    version: version.into(),
                    import_name: "test_sdk".into()
                }
            )
            .is_err(),
            "{version}"
        );
    }
    for version in ["1.0.0", "1.0.0-rc.1", "1.0.0+build.7"] {
        assert!(
            emit_http(
                &plan,
                &PackageConfig {
                    name: "test-sdk".into(),
                    version: version.into(),
                    import_name: "test_sdk".into()
                }
            )
            .is_ok(),
            "{version}"
        );
    }
    let files = emit_http(
        &plan,
        &PackageConfig {
            name: "test-sdk".into(),
            version: "0.0.0".into(),
            import_name: "test_sdk".into(),
        },
    )
    .unwrap();
    assert!(
        files
            .iter()
            .any(|file| file.path == "python/src/test_sdk/py.typed")
    );
    assert!(
        files
            .iter()
            .any(|file| file.path == "python/src/test_sdk/model_codecs.py")
    );
    assert_eq!(
        files
            .iter()
            .map(|file| &file.path)
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
        files.len()
    );
}

#[test]
#[ignore = "requires Python build/httpx/mypy tools and a native Python interpreter"]
fn installed_wheel_sync_async_types_and_wire_contract() {
    let root = tempfile::tempdir().unwrap().keep();
    let config = PackageConfig {
        name: "m3-native-sdk".into(),
        version: "0.0.0".into(),
        import_name: "m3_native_sdk".into(),
    };
    suspect_codegen::write_files(&emit_http(&plan(), &config).unwrap(), &root).unwrap();
    let tools = std::env::var_os("SUSPECT_PYTHON_TOOLS")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../target/sdk-native-python-tools/bin/python")
        });
    checked(
        Command::new(&tools)
            .args(["-m", "build", "--wheel", "--no-isolation"])
            .current_dir(root.join("python")),
        &root,
    );
    let wheel = std::fs::read_dir(root.join("python/dist"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| path.extension().is_some_and(|ext| ext == "whl"))
        .unwrap();
    let interpreter = std::env::var_os("SUSPECT_PYTHON_BIN").unwrap_or_else(|| "python3".into());
    checked(
        Command::new("uv")
            .args(["venv", "--python"])
            .arg(&interpreter)
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
    std::fs::write(root.join("consumer.py"), CONSUMER).unwrap();
    checked(
        Command::new(&python).arg("consumer.py").current_dir(&root),
        &root,
    );
    std::fs::write(root.join("consumer_types.py"), TYPES).unwrap();
    checked(
        Command::new(&tools)
            .args([
                "-m",
                "mypy",
                "--strict",
                "--python-version",
                "3.11",
                "--python-executable",
            ])
            .arg(&python)
            .arg("--cache-dir")
            .arg(root.join("mypy-cache"))
            .arg("consumer_types.py")
            .current_dir(&root),
        &root,
    );
    checked(
        Command::new(&tools)
            .args([
                "-m",
                "mypy",
                "--strict",
                "--python-version",
                "3.11",
                "--cache-dir",
            ])
            .arg(root.join("source-mypy-cache"))
            .arg(root.join("python/src/m3_native_sdk")),
        &root,
    );
    checked(
        Command::new(&python)
            .args([
                "-m",
                "pydoc",
                "-w",
                "m3_native_sdk",
                "m3_native_sdk.models",
                "m3_native_sdk.operations",
            ])
            .current_dir(&root),
        &root,
    );
    assert!(
        std::fs::read_to_string(root.join("m3_native_sdk.operations.html"))
            .unwrap()
            .contains("CreateWidgetStatus200")
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
#[ignore = "requires the native Python wheel build tools"]
fn admitted_python_versions_produce_native_wheel_metadata() {
    let tools = std::env::var_os("SUSPECT_PYTHON_TOOLS")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../target/sdk-native-python-tools/bin/python")
        });
    for (version, normalized) in [
        ("1.2.3-rc.1", "1.2.3rc1"),
        ("1.2.3+build.7", "1.2.3+build.7"),
    ] {
        let root = tempfile::tempdir().unwrap().keep();
        let files = emit_http(
            &plan(),
            &PackageConfig {
                name: "version-probe".into(),
                version: version.into(),
                import_name: "version_probe".into(),
            },
        )
        .unwrap();
        suspect_codegen::write_files(&files, &root).unwrap();
        checked(
            Command::new(&tools)
                .args(["-m", "build", "--wheel", "--no-isolation"])
                .current_dir(root.join("python")),
            &root,
        );
        checked(Command::new(&tools).args(["-c","import email.parser,pathlib,sys,zipfile; wheel=next(pathlib.Path('dist').glob('*.whl')); archive=zipfile.ZipFile(wheel); name=next(n for n in archive.namelist() if n.endswith('.dist-info/METADATA')); metadata=email.parser.Parser().parsestr(archive.read(name).decode()); assert metadata['Version']==sys.argv[1], metadata['Version']",normalized]).current_dir(root.join("python")),&root);
        std::fs::remove_dir_all(root).unwrap();
    }
}

const TYPES: &str = r#"
from m3_native_sdk import Client, AsyncClient, models, UNSET
from m3_native_sdk._client import CreateWidgetStatus200
def use(client: Client) -> str:
    response: CreateWidgetStatus200 = client.create_widget(body=models.WidgetInput(name="alpha"))
    if isinstance(response.data.payload,models.StandardPayload):
        return response.data.payload.text
    return response.data.payload.vault
async def use_async(client: AsyncClient) -> None:
    async with client as opened:
        result = await opened.get_widget(widget_id="id")
        result.data.amount.token
def negatives(client: Client) -> None:
    client.create_widget()  # type: ignore[call-arg]
    models.WidgetInput(name=None)  # type: ignore[arg-type]
    models.WidgetInput(name=1)  # type: ignore[arg-type]
"#;

const CONSUMER: &str = r#"
import asyncio, json, threading, httpx
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from m3_native_sdk import Client, AsyncClient, models, codecs, UNSET, SdkError, ApiError
from m3_native_sdk.codec_runtime import CodecError
from m3_native_sdk._client import CreateWidgetStatus422
WIDGET=b'{"id":"w1","amount":9007199254740993.000000000000000001,"meta":null,"payload":{"kind":"standard","text":"plain"},"child":{"label":"root"}}'
PAGE=b'{"items":[{"id":"w2","amount":1e-400,"payload":{"kind":"secure","vault":"v1"}}]}'
seen=[]
class Handler(BaseHTTPRequestHandler):
    def log_message(self,*args): pass
    def handle_request(self):
        body=self.rfile.read(int(self.headers.get('Content-Length','0')))
        seen.append((self.command,self.path,self.headers.get('Authorization'),body))
        rejected=body==b'{"name":"deny"}'
        payload=b'{"message":"rejected"}' if rejected else PAGE if self.path.startswith('/api/v1/widgets?') else WIDGET
        self.send_response(422 if rejected else 200)
        self.send_header('Content-Type','Application/JSON; charset="utf-8"')
        self.send_header('Content-Length',str(len(payload)))
        self.end_headers();self.wfile.write(payload)
    do_GET=handle_request
    do_POST=handle_request
    do_PATCH=handle_request
server=ThreadingHTTPServer(('127.0.0.1',0),Handler)
thread=threading.Thread(target=server.serve_forever,daemon=True);thread.start()
base=f'http://127.0.0.1:{server.server_port}/api/v1'
try:
    with Client(auth={'apiKey':'test-key'},server_url=base) as client:
        created=client.create_widget(body=models.WidgetInput(name='alpha'))
        assert created.status==200 and created.data.amount.token=='9007199254740993.000000000000000001'
        assert created.data.meta is None and created.data.child.child is UNSET
        assert isinstance(created.data.payload,models.StandardPayload)
        page=client.list_widgets(tag='a',tags=['x','y'],labels=['a,b','c'],limit=2)
        assert page.data.items[0].amount.token=='1e-400'
        assert page.data.items[0].meta is UNSET
        client.get_widget(widget_id="a/b 雪!'()*")
        client.update_widget(widget_id='w1',body=models.WidgetPatch())
        assert seen==[
            ('POST','/api/v1/widgets','Bearer test-key',b'{"name":"alpha"}'),
            ('GET','/api/v1/widgets?tag=a&tags=x&tags=y&labels=a%2Cb,c&limit=2','Bearer test-key',b''),
            ('GET','/api/v1/widgets/a%2Fb%20%E9%9B%AA%21%27%28%29%2A','Bearer test-key',b''),
            ('PATCH','/api/v1/widgets/w1','Bearer test-key',b'{}'),
        ],seen
        invalid=models.WidgetInput(name='a');invalid.name=''
        try: client.create_widget(body=invalid)
        except CodecError: pass
        else: raise AssertionError('mutated invalid model sent')
        assert len(seen)==4
        try: client.create_widget(body=models.WidgetInput(name='deny'))
        except CreateWidgetStatus422 as error:
            assert error.status==422 and error.data.message=='rejected'
            assert 'rejected' not in str(error)
        else: raise AssertionError('declared API failure accepted as success')
    async def run():
        async with AsyncClient(auth={'apiKey':'test-key'},server_url=base) as client:
            result=await client.get_widget(widget_id='w1')
            assert result.data.id=='w1'
            task=asyncio.create_task(client.get_widget(widget_id='cancelled'))
            task.cancel()
            try: await task
            except asyncio.CancelledError: pass
            else: raise AssertionError('cancelled task completed')
    asyncio.run(run())
    assert len(seen)==6
    # A caller-owned httpx MockTransport commonly returns an already-buffered
    # Response. Consume its raw byte stream without HTTPX's decoded cache.
    mock=httpx.MockTransport(lambda request:httpx.Response(200,headers={'Content-Type':'application/json'},content=WIDGET))
    with Client(auth={'apiKey':'test-key'},transport=mock) as client:
        assert client.get_widget(widget_id='mock').data.id=='w1'
    mock.close()
    async def cancel_mid_body():
        started=asyncio.Event();closed=asyncio.Event()
        class Stream(httpx.AsyncByteStream):
            async def __aiter__(self):
                yield b'{'
                started.set()
                await asyncio.Event().wait()
            async def aclose(self): closed.set()
        class Transport(httpx.AsyncBaseTransport):
            async def handle_async_request(self,request):
                return httpx.Response(200,headers={'Content-Type':'application/json'},stream=Stream())
        async with AsyncClient(auth={'apiKey':'test-key'},transport=Transport()) as client:
            task=asyncio.create_task(client.get_widget(widget_id='stream'))
            await asyncio.wait_for(started.wait(),2)
            task.cancel()
            try: await task
            except asyncio.CancelledError: pass
            else: raise AssertionError('mid-body cancellation was swallowed')
            assert closed.is_set(),'cancelled response body did not close'
    asyncio.run(cancel_mid_body())
    with Client(auth={'apiKey':'test-key'},server_url=base,max_response_bytes=8,max_capture_bytes=4) as client:
        try: client.get_widget(widget_id='w1')
        except SdkError as error:
            assert error.kind=='resource-limit' and error.status==200 and len(error.capture)<=4 and error.truncated
        else: raise AssertionError('response ceiling ignored')
finally:
    server.shutdown();server.server_close();thread.join()
"#;
