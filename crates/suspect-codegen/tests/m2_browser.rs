//! Real isolated headless-browser consumer of the generated platform-only SDK.

use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    path::{Path, PathBuf},
    process::Command,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};
use suspect_codegen::typescript::{
    http::{HttpConfig, plan_http},
    package::{PackageConfig, emit_http},
};
use suspect_ir::contract::Contract;
use suspect_ref::WorkspaceBuilder;
use suspect_source::Uri;

fn checked(command: &mut Command, root: &Path) -> std::process::Output {
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "browser fixture {}\n{command:?}\n{}{}",
        root.display(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

fn serve(mut stream: TcpStream, root: &Path, calls: &AtomicUsize) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let mut request = Vec::new();
    let mut chunk = [0u8; 4096];
    while !request.windows(4).any(|window| window == b"\r\n\r\n") {
        let Ok(count) = stream.read(&mut chunk) else {
            return;
        };
        if count == 0 {
            return;
        };
        request.extend_from_slice(&chunk[..count]);
        if request.len() > 65536 {
            return;
        };
    }
    let text = String::from_utf8_lossy(&request);
    let path = text
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .unwrap_or("/");
    let (status, media, body) = if path == "/api/v1/credits" {
        calls.fetch_add(1, Ordering::SeqCst);
        let auth = text
            .lines()
            .any(|line| line.eq_ignore_ascii_case("authorization: Bearer browser-fixture"));
        if !auth {
            (400, "text/plain", b"wrong credential header".to_vec())
        } else {
            (
                200,
                "application/json",
                br#"{"data":{"total_credits":9007199254740993.25,"total_usage":1e-400}}"#.to_vec(),
            )
        }
    } else if path == "/" {
        (200, "text/html", fs_read(&root.join("index.html")))
    } else if path.starts_with("/dist/") && !path.contains("..") && !path.contains(['?', '#', '\\'])
    {
        let file = root.join(path.trim_start_matches('/'));
        match std::fs::read(&file) {
            Ok(body) => (200, "text/javascript", body),
            Err(_) => (404, "text/plain", Vec::new()),
        }
    } else {
        (404, "text/plain", Vec::new())
    };
    let header = format!(
        "HTTP/1.1 {status} Fixture\r\nContent-Type: {media}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let _ = stream.write_all(header.as_bytes());
    let _ = stream.write_all(&body);
}
fn fs_read(path: &Path) -> Vec<u8> {
    std::fs::read(path).unwrap()
}

#[test]
#[ignore = "requires the tracked corpus, pinned Node/TypeScript and an isolated Chromium executable"]
fn generated_sdk_runs_in_a_real_browser_with_exact_numbers_and_abort() {
    let temporary = tempfile::tempdir().unwrap();
    let retained = temporary.keep();
    let source = std::env::var_os("OPENROUTER_WEB_ROOT")
        .map(PathBuf::from)
        .expect("set OPENROUTER_WEB_ROOT");
    let path = source.join("projects/docs/openapi/openapi.yaml");
    let workspace = Arc::new(
        WorkspaceBuilder::new()
            .root(path.parent().unwrap())
            .build()
            .unwrap(),
    );
    let contract =
        Arc::new(Contract::from_workspace(&workspace, &Uri::from_path(&path).unwrap()).unwrap());
    let op = contract
        .operations()
        .find(|op| op.operation_id() == Some("getCredits"))
        .unwrap()
        .source()
        .clone();
    let plan = plan_http(contract, &[op], HttpConfig::default()).unwrap();
    suspect_codegen::write_files(
        &emit_http(
            &plan,
            &PackageConfig {
                name: "@fixture/browser-sdk".into(),
                version: "0.0.0".into(),
            },
        )
        .unwrap(),
        &retained,
    )
    .unwrap();
    let root = retained.join("typescript");
    let node = std::env::var_os("SUSPECT_DOCS_NODE").unwrap_or_else(|| "node".into());
    let tsc = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tools/typescript-docs/node_modules/typescript/bin/tsc");
    checked(
        Command::new(&node)
            .arg(tsc)
            .args(["--project", "tsconfig.json"])
            .current_dir(&root),
        &retained,
    );
    std::fs::write(root.join("index.html"),r#"<!doctype html><html><head><meta charset="utf-8"><title>SDK native browser gate</title></head><body data-sdk-status="pending"><pre id="result"></pre><script type="module">
import {createClient,isSdkError} from './dist/operations.js';
const result=document.getElementById('result');
try {
    const client=createClient({auth:{apiKey:'browser-fixture'},serverURL:location.origin+'/api/v1'});
    const response=await client.getCredits({});
    if(response.status!==200 || response.data.data.total_credits.toString()!=='9007199254740993.25' || response.data.data.total_usage.toString()!=='1e-400') throw new Error('exact response values changed');
    const aborted=new AbortController();aborted.abort();
    let cancelled=false;try {await client.getCredits({}, {signal:aborted.signal});} catch(error) {cancelled=isSdkError(error) && error.kind==='cancelled';}
    if(!cancelled) throw new Error('caller abort did not remain an SDK cancellation');
    document.body.dataset.sdkStatus='passed';result.textContent=JSON.stringify({status:'passed',agent:navigator.userAgent,exactNumbers:true,abort:true});
} catch(error) {document.body.dataset.sdkStatus='failed';result.textContent=String(error?.stack||error);}
</script></body></html>"#).unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    listener.set_nonblocking(true).unwrap();
    let stopped = Arc::new(AtomicBool::new(false));
    let calls = Arc::new(AtomicUsize::new(0));
    let stop = stopped.clone();
    let count = calls.clone();
    let served = root.clone();
    let server = std::thread::spawn(move || {
        let mut handlers = Vec::new();
        while !stop.load(Ordering::SeqCst) {
            match listener.accept() {
                Ok((stream, _)) => {
                    let root = served.clone();
                    let count = count.clone();
                    handlers.push(std::thread::spawn(move || serve(stream, &root, &count)));
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(5))
                }
                Err(_) => break,
            }
        }
        for handler in handlers {
            let _ = handler.join();
        }
    });
    let browser = std::env::var_os("SUSPECT_CHROMIUM")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from("/Applications/Google Chrome.app/Contents/MacOS/Google Chrome")
        });
    assert!(
        browser.is_file(),
        "set SUSPECT_CHROMIUM to a real Chromium executable"
    );
    let stdout = std::fs::File::create(retained.join("browser.stdout.html")).unwrap();
    let stderr = std::fs::File::create(retained.join("browser.stderr.log")).unwrap();
    let mut child = Command::new(&browser)
        .args([
            "--headless=new",
            "--disable-gpu",
            "--no-first-run",
            "--no-default-browser-check",
            "--disable-background-networking",
            "--disable-extensions",
            "--password-store=basic",
            "--use-mock-keychain",
            "--dump-dom",
            "--virtual-time-budget=5000",
        ])
        .arg(format!(
            "--user-data-dir={}",
            retained.join("browser-profile").display()
        ))
        .arg(format!("http://{address}/"))
        .stdout(stdout)
        .stderr(stderr)
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut completed = false;
    let status = loop {
        let page =
            std::fs::read_to_string(retained.join("browser.stdout.html")).unwrap_or_default();
        if page.contains("data-sdk-status=\"passed\"")
            || page.contains("data-sdk-status=\"failed\"")
        {
            completed = true;
            let _ = child.kill();
            break child.wait().unwrap();
        }
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if Instant::now() >= deadline {
            child.kill().unwrap();
            break child.wait().unwrap();
        }
        std::thread::sleep(Duration::from_millis(25));
    };
    stopped.store(true, Ordering::SeqCst);
    server.join().unwrap();
    let html = std::fs::read_to_string(retained.join("browser.stdout.html")).unwrap();
    let stderr = std::fs::read_to_string(retained.join("browser.stderr.log")).unwrap();
    assert!(
        (completed || status.success()) && html.contains("data-sdk-status=\"passed\""),
        "browser fixture {}\n{}\n{}",
        retained.display(),
        html,
        stderr
    );
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "pre-aborted request must not reach the recording server"
    );
    std::fs::remove_dir_all(retained).unwrap();
}
