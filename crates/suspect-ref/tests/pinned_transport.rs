//! Real loopback HTTP peers exercise acquisition policy through public inputs.

use std::fs;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use suspect_ref::acquire::{
    AcquireErrorKind, AcquireOptions, CredentialHeader, Credentials, CredentialsError, acquire,
};
use suspect_source::Uri;

mod pinned_support;
use pinned_support::{
    Action, Server, options, pin, redirected_pin, response, tempdir, write_manifest,
};

#[test]
fn cleartext_requires_an_exact_numeric_loopback_test_exception() {
    let root = tempdir();
    let server = Server::new(|_| panic!("HTTP denied before transport I/O"));
    let entry = server.uri("/root.json");
    let manifest = write_manifest(root.path(), &entry, vec![pin(&entry, b"{}")]);
    let mut config = options(root.path(), &server);
    config.insecure_test_origins.clear();
    assert_eq!(
        acquire(&manifest, config.clone()).unwrap_err().kind(),
        &AcquireErrorKind::InsecureScheme
    );
    for origin in [
        "http://example.com",
        "http://localhost",
        "http://127.0.0.1/path",
        "https://127.0.0.1",
    ] {
        config.insecure_test_origins = vec![origin.into()];
        assert!(matches!(
            acquire(&manifest, config.clone()).unwrap_err().kind(),
            AcquireErrorKind::InvalidOptions { .. }
        ));
    }
    assert!(server.requests().is_empty());
}

#[test]
fn cross_authority_redirects_are_recorded_then_denied_without_contacting_the_target() {
    let root = tempdir();
    let target = Server::new(|_| panic!("denied redirect target must not receive a request"));
    let effective = target.uri("/final.json");
    let location = effective.to_string();
    let source = Server::new(move |_| response(302, &[("Location", &location)], b"ignored"));
    let entry = source.uri("/root.json");
    let manifest = write_manifest(
        root.path(),
        &entry,
        vec![redirected_pin(
            &entry,
            &effective,
            b"{}",
            &[(&entry, &effective, 302)],
        )],
    );
    let mut config = options(root.path(), &source);
    config.insecure_test_origins.push(target.origin.clone());
    let error = acquire(&manifest, config).unwrap_err();
    assert_eq!(error.kind(), &AcquireErrorKind::RedirectDenied);
    assert_eq!(error.redirects().len(), 1);
    assert_eq!(error.redirects()[0].to_uri(), &effective);
    assert_eq!(source.requests().len(), 1);
    assert!(target.requests().is_empty());
}

struct FirstOriginCredentials {
    first: String,
    observed: Arc<Mutex<Vec<(Uri, String)>>>,
}

impl Credentials for FirstOriginCredentials {
    fn headers(
        &self,
        requested: &Uri,
        origin: &str,
    ) -> Result<Vec<CredentialHeader>, CredentialsError> {
        self.observed
            .lock()
            .unwrap()
            .push((requested.clone(), origin.into()));
        Ok(if origin == self.first {
            vec![CredentialHeader::new(
                "Authorization",
                "Bearer first-origin-only",
            )]
        } else {
            vec![]
        })
    }
}

#[test]
fn allowed_cross_authority_redirects_reselect_credentials_and_never_forward_authorization() {
    let root = tempdir();
    let target = Server::new(|_| response(200, &[("Content-Type", "application/json")], b"{}"));
    let effective = target.uri("/final.json");
    let location = effective.to_string();
    let source = Server::new(move |_| response(307, &[("Location", &location)], b"ignored"));
    let entry = source.uri("/root.json");
    let manifest = write_manifest(
        root.path(),
        &entry,
        vec![redirected_pin(
            &entry,
            &effective,
            b"{}",
            &[(&entry, &effective, 307)],
        )],
    );
    let observed = Arc::new(Mutex::new(Vec::new()));
    let mut config = options(root.path(), &source);
    config.insecure_test_origins.push(target.origin.clone());
    config.allowed_redirect_origins.push(target.origin.clone());
    config.credentials = Some(Arc::new(FirstOriginCredentials {
        first: source.origin.clone(),
        observed: observed.clone(),
    }));
    let closure = acquire(&manifest, config).unwrap();
    assert_eq!(closure.records()[0].attempts(), 2);
    assert_eq!(
        *observed.lock().unwrap(),
        vec![
            (entry.clone(), source.origin.clone()),
            (entry, target.origin.clone())
        ]
    );
    assert_eq!(
        source.requests()[0]
            .headers
            .get("authorization")
            .map(String::as_str),
        Some("Bearer first-origin-only")
    );
    assert!(!target.requests()[0].headers.contains_key("authorization"));
    assert_eq!(
        target.requests()[0]
            .headers
            .get("accept-encoding")
            .map(String::as_str),
        Some("identity")
    );
}

#[test]
fn unpinned_redirects_and_redirect_hop_limits_fail_before_next_request() {
    for cap in [0, 3] {
        let root = tempdir();
        let server = Server::new(|request| {
            assert_eq!(request.path, "/root.json");
            response(302, &[("Location", "/undeclared.json")], b"ignored")
        });
        let entry = server.uri("/root.json");
        let manifest = write_manifest(root.path(), &entry, vec![pin(&entry, b"{}")]);
        let mut config = options(root.path(), &server);
        config.max_redirects = cap;
        let error = acquire(&manifest, config).unwrap_err();
        if cap == 0 {
            assert_eq!(
                error.kind(),
                &AcquireErrorKind::TooManyRedirects { limit: 0 }
            );
        } else {
            assert_eq!(error.kind(), &AcquireErrorKind::RedirectDrift);
        }
        assert_eq!(error.redirects().len(), 1);
        assert_eq!(server.requests().len(), 1);
    }
}

#[test]
fn declared_length_and_chunked_streams_are_bounded_before_snapshot_publication() {
    for chunked in [false, true] {
        let root = tempdir();
        let server = Server::new(move |_| {
            if chunked {
                vec![Action::Bytes(b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n40\r\n                                                                \r\n0\r\n\r\n".to_vec())]
            } else {
                response(200, &[("Content-Type", "application/json")], &[b' '; 64])
            }
        });
        let entry = server.uri("/root.json");
        let manifest = write_manifest(root.path(), &entry, vec![pin(&entry, b"{}")]);
        let mut config = options(root.path(), &server);
        config.max_bytes = 8;
        let cache = config.cache_dir.clone();
        assert_eq!(
            acquire(&manifest, config).unwrap_err().kind(),
            &AcquireErrorKind::TooLarge { limit: 8 },
            "chunked={chunked}"
        );
        assert!(!cache.exists());
    }
}

#[test]
fn response_header_bytes_and_field_counts_are_bounded_independently() {
    for many in [false, true] {
        let root = tempdir();
        let server = Server::new(move |_| {
            let mut bytes = b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n".to_vec();
            if many {
                for _ in 0..20 {
                    bytes.extend_from_slice(b"X-Test: yes\r\n");
                }
            } else {
                bytes.extend_from_slice(format!("X-Large: {}\r\n", "a".repeat(2048)).as_bytes());
            }
            bytes.extend_from_slice(b"Content-Length: 2\r\n\r\n{}");
            vec![Action::Bytes(bytes)]
        });
        let entry = server.uri("/root.json");
        let manifest = write_manifest(root.path(), &entry, vec![pin(&entry, b"{}")]);
        let mut config = options(root.path(), &server);
        if many {
            config.max_header_count = 8;
        } else {
            config.max_header_bytes = 128;
        }
        let error = acquire(&manifest, config).unwrap_err();
        if many {
            assert_eq!(error.kind(), &AcquireErrorKind::TooManyHeaders { limit: 8 });
        } else {
            assert_eq!(
                error.kind(),
                &AcquireErrorKind::HeadersTooLarge { limit: 128 }
            );
        }
    }
}

#[test]
fn chunked_bytes_are_exact_and_chunk_extensions_share_the_header_budget() {
    for oversized in [false, true] {
        let root = tempdir();
        let server = Server::new(move |_| {
            let extension = if oversized {
                format!(";padding={}", "x".repeat(1024))
            } else {
                ";name=value".into()
            };
            vec![Action::Bytes(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n1{extension}\r\n{{\r\n1\r\n}}\r\n0\r\nX-Trailer: bounded\r\n\r\n").into_bytes())]
        });
        let entry = server.uri("/root.json");
        let manifest = write_manifest(root.path(), &entry, vec![pin(&entry, b"{}")]);
        let mut config = options(root.path(), &server);
        config.max_bytes = 2;
        config.max_header_bytes = 256;
        let result = acquire(&manifest, config);
        if oversized {
            assert_eq!(
                result.unwrap_err().kind(),
                &AcquireErrorKind::HeadersTooLarge { limit: 256 }
            );
        } else {
            let closure = result.unwrap();
            assert_eq!(closure.provider().document(&entry).unwrap().bytes(), b"{}");
            let workspace = closure.workspace_builder().build().unwrap();
            assert_eq!(
                workspace
                    .open(entry.as_str())
                    .unwrap()
                    .doc()
                    .inner()
                    .bytes(),
                b"{}"
            );
        }
    }
}

#[test]
fn unsupported_media_compression_and_non_utf8_are_rejected_without_lossy_substitution() {
    let cases = [
        (vec![], b"{}".as_slice(), AcquireErrorKind::BadMediaType),
        (
            vec![("Content-Type", "text/html")],
            b"{}".as_slice(),
            AcquireErrorKind::BadMediaType,
        ),
        (
            vec![("Content-Type", "application/yaml")],
            b"{}".as_slice(),
            AcquireErrorKind::BadMediaType,
        ),
        (
            vec![("Content-Type", "application/json; charset=utf-16")],
            b"{}".as_slice(),
            AcquireErrorKind::BadMediaType,
        ),
        (
            vec![
                ("Content-Type", "application/json"),
                ("Content-Encoding", "gzip"),
            ],
            b"{}".as_slice(),
            AcquireErrorKind::UnsupportedEncoding,
        ),
        (
            vec![("Content-Type", "application/json")],
            b"\xff\xfe{}".as_slice(),
            AcquireErrorKind::InvalidUtf8,
        ),
    ];
    for (headers, body, expected) in cases {
        let root = tempdir();
        let server = Server::new(move |_| response(200, &headers, body));
        let entry = server.uri("/root.json");
        let manifest = write_manifest(root.path(), &entry, vec![pin(&entry, body)]);
        let config = options(root.path(), &server);
        let cache = config.cache_dir.clone();
        assert_eq!(acquire(&manifest, config).unwrap_err().kind(), &expected);
        assert!(!cache.exists());
    }
}

#[test]
fn ambiguous_http_framing_and_duplicate_media_are_rejected() {
    for extra in [
        "Content-Length: 2\r\nContent-Length: 2\r\n",
        "Content-Length: 2\r\nTransfer-Encoding: chunked\r\n",
        "Content-Type: application/json\r\nContent-Length: 2\r\n",
    ] {
        let root = tempdir();
        let server = Server::new(move |_| {
            vec![Action::Bytes(
                format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n{extra}\r\n{{}}")
                    .into_bytes(),
            )]
        });
        let entry = server.uri("/root.json");
        let manifest = write_manifest(root.path(), &entry, vec![pin(&entry, b"{}")]);
        let error = acquire(&manifest, options(root.path(), &server)).unwrap_err();
        assert!(
            matches!(
                error.kind(),
                AcquireErrorKind::InvalidResponse | AcquireErrorKind::Transport { .. }
            ),
            "{error:?}"
        );
    }
}

struct StaticCredentials(Vec<CredentialHeader>);
impl Credentials for StaticCredentials {
    fn headers(&self, _: &Uri, _: &str) -> Result<Vec<CredentialHeader>, CredentialsError> {
        Ok(self.0.clone())
    }
}

#[test]
fn credential_header_injection_reserved_headers_duplicates_and_limits_fail_before_io() {
    let root = tempdir();
    let server = Server::new(|_| panic!("invalid credential headers must not start HTTP"));
    let entry = server.uri("/root.json?token=uri-secret");
    let manifest = write_manifest(root.path(), &entry, vec![pin(&entry, b"{}")]);
    for headers in [
        vec![CredentialHeader::new(
            "Authorization",
            "value-secret\r\nX-Leak: value-secret",
        )],
        vec![CredentialHeader::new("Host", "value-secret")],
        vec![
            CredentialHeader::new("Authorization", "value-secret"),
            CredentialHeader::new("authorization", "value-secret"),
        ],
        vec![CredentialHeader::new(
            "Authorization",
            "value-secret".repeat(4096),
        )],
    ] {
        let mut config = options(root.path(), &server);
        config.credentials = Some(Arc::new(StaticCredentials(headers)));
        let error = acquire(&manifest, config).unwrap_err();
        assert!(matches!(
            error.kind(),
            AcquireErrorKind::InvalidHeader | AcquireErrorKind::HeadersTooLarge { .. }
        ));
        let shown = format!("{error} {error:?}");
        assert!(!shown.contains("value-secret"));
        assert!(!shown.contains("uri-secret"));
    }
    assert!(server.requests().is_empty());
}

#[test]
fn failures_do_not_retry_or_echo_response_bodies_headers_or_credentials() {
    let root = tempdir();
    let server = Server::new(|_| response(503, &[("X-Echo", "header-secret")], b"response-secret"));
    let entry = server.uri("/root.json?key=uri-secret");
    let manifest = write_manifest(root.path(), &entry, vec![pin(&entry, b"{}")]);
    let mut config = options(root.path(), &server);
    config.credentials = Some(Arc::new(StaticCredentials(vec![CredentialHeader::new(
        "Authorization",
        "auth-secret",
    )])));
    let error = acquire(&manifest, config).unwrap_err();
    assert_eq!(error.kind(), &AcquireErrorKind::HttpStatus { status: 503 });
    let shown = format!("{error} {error:?}");
    for secret in [
        "header-secret",
        "response-secret",
        "auth-secret",
        "uri-secret",
    ] {
        assert!(!shown.contains(secret));
    }
    assert_eq!(server.requests().len(), 1);
}

#[test]
fn timeout_interrupts_stalled_headers_and_cancellation_interrupts_stalled_bodies() {
    for cancel in [false, true] {
        let root = tempdir();
        let server = Server::new(move |_| {
            let mut actions = Vec::new();
            if cancel {
                actions.push(Action::Bytes(b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 2\r\n\r\n{".to_vec()));
            }
            actions.push(Action::Pause(Duration::from_millis(600)));
            actions
        });
        let entry = server.uri("/root.json");
        let manifest = write_manifest(root.path(), &entry, vec![pin(&entry, b"{}")]);
        let mut config = options(root.path(), &server);
        let token = config.cancellation.clone();
        let worker = if cancel {
            Some(std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(80));
                token.cancel();
            }))
        } else {
            config.timeout = Duration::from_millis(80);
            None
        };
        let start = Instant::now();
        let error = acquire(&manifest, config).unwrap_err();
        assert_eq!(
            error.kind(),
            if cancel {
                &AcquireErrorKind::Cancelled
            } else {
                &AcquireErrorKind::Timeout
            }
        );
        assert!(
            start.elapsed() < Duration::from_millis(500),
            "operation waited for the stalled peer"
        );
        if let Some(worker) = worker {
            worker.join().unwrap();
        }
        assert_eq!(server.requests().len(), 1);
    }
}

#[test]
fn cancellation_precedes_even_manifest_io() {
    let root = tempdir();
    let config = AcquireOptions::default();
    config.cancellation.cancel();
    assert_eq!(
        acquire(&root.path().join("does-not-exist.json"), config)
            .unwrap_err()
            .kind(),
        &AcquireErrorKind::Cancelled
    );
}

#[test]
fn concurrent_acquisitions_produce_independent_verified_providers() {
    let barrier = Arc::new(std::sync::Barrier::new(16));
    let outcomes = std::thread::scope(|scope| {
        let mut workers = Vec::new();
        for _ in 0..16 {
            let barrier = barrier.clone();
            workers.push(scope.spawn(move || {
                let root = tempdir();
                let server =
                    Server::new(|_| response(200, &[("Content-Type", "application/json")], b"{}"));
                let entry = server.uri("/root.json");
                let manifest = write_manifest(root.path(), &entry, vec![pin(&entry, b"{}")]);
                let config = options(root.path(), &server);
                barrier.wait();
                let outcome = acquire(&manifest, config).map(|closure| {
                    let workspace = closure.workspace_builder().build().unwrap();
                    assert_eq!(
                        workspace
                            .open(entry.as_str())
                            .unwrap()
                            .doc()
                            .inner()
                            .bytes(),
                        b"{}"
                    );
                });
                (outcome, server.requests().len())
            }));
        }
        workers
            .into_iter()
            .map(|worker| worker.join().unwrap())
            .collect::<Vec<_>>()
    });
    for (outcome, requests) in outcomes {
        assert!(
            outcome.is_ok(),
            "{outcome:?}; peer received {requests} requests"
        );
    }
}

#[test]
fn ordinary_contract_compilation_has_no_network_or_neighbor_document_acquisition() {
    use suspect_ir::contract::{Contract, ContractReader, ContractSeverity, SourceId};
    use suspect_low::Pointer;
    use suspect_ref::{RefError, WorkspaceBuilder, WorkspaceError};
    let root = tempdir();
    let server = Server::new(|_| panic!("ordinary compilation must not access HTTP"));
    let remote = server.uri("/unacquired.json");
    let entry_path = root.path().join("openapi.json");
    fs::write(
        &entry_path,
        serde_json::to_vec(&serde_json::json!({
            "openapi": "3.1.0", "info": {"title": "Offline", "version": "1"}, "paths": {},
            "components": {"schemas": {"Use": {"$ref": format!("{remote}#/Pet")}}}
        }))
        .unwrap(),
    )
    .unwrap();
    fs::write(
        root.path().join("unrelated.json"),
        b"not valid JSON and not a source input",
    )
    .unwrap();
    let entry = Uri::from_path(&entry_path).unwrap();
    let neighbor = Uri::from_path(&root.path().join("unrelated.json")).unwrap();
    for reader in [ContractReader::Lossless, ContractReader::Fast] {
        let workspace = Arc::new(WorkspaceBuilder::new().build().unwrap());
        let contract = Contract::from_workspace_with_reader(&workspace, &entry, reader).unwrap();
        let use_source = SourceId::new(
            entry.clone(),
            Pointer::parse("/components/schemas/Use").unwrap(),
        );
        assert_eq!(
            contract.diagnostics().len(),
            1,
            "{:?}",
            contract.diagnostics()
        );
        let diagnostic = &contract.diagnostics()[0];
        assert_eq!(diagnostic.code, "invalid-reference");
        assert_eq!(diagnostic.severity, ContractSeverity::Error);
        assert_eq!(diagnostic.source, use_source);
        assert_eq!(
            diagnostic.at,
            contract.source_span(&use_source.child("$ref")).unwrap()
        );
        assert!(!diagnostic.at.is_empty());
        assert_eq!(
            diagnostic.message,
            format!("resource `{remote}` is not registered in the supplied source closure")
        );
        assert_eq!(
            contract
                .source(&use_source.child("$ref"))
                .and_then(serde_json::Value::as_str),
            Some(format!("{remote}#/Pet").as_str())
        );
        assert!(contract.reference_target(&use_source).is_none());
        assert_eq!(contract.documents().count(), 1);
        assert_eq!(workspace.uris(), vec![entry.clone()]);
        assert!(workspace.get(&remote).is_none());
        assert!(workspace.get(&neighbor).is_none());
        assert!(contract.document(&neighbor).is_none());
        assert!(
            workspace.failed_document_uris().is_empty(),
            "Contract does not call Workspace::open for an unprovided remote URI"
        );
        assert!(server.requests().is_empty());

        // A deliberate load request still records its failed URI. This checks
        // the bookkeeping separately from Contract's intentional no-open path.
        assert!(
            matches!(workspace.open(remote.as_str()), Err(WorkspaceError::Ref(RefError::RemoteDenied { uri })) if uri == remote.as_str())
        );
        assert_eq!(workspace.failed_document_uris(), vec![remote.clone()]);
        assert_eq!(workspace.uris(), vec![entry.clone()]);
        assert!(workspace.get(&neighbor).is_none());
        assert!(
            server.requests().is_empty(),
            "even the explicit denied load must stop before HTTP"
        );
    }
}

#[test]
fn https_verification_cannot_be_disabled_by_curlrc_or_ambient_certificate_settings() {
    use pinned_support::ChildGuard;
    use std::io::{BufRead, Read};
    use std::process::{Command, Stdio};
    let root = tempdir();
    let certificate = root.path().join("self-signed.pem");
    let key = root.path().join("test-key.pem");
    let generated = Command::new("openssl")
        .args([
            "req",
            "-newkey",
            "rsa:2048",
            "-x509",
            "-sha256",
            "-nodes",
            "-days",
            "1",
            "-subj",
            "/CN=127.0.0.1",
        ])
        .arg("-keyout")
        .arg(&key)
        .arg("-out")
        .arg(&certificate)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .unwrap();
    assert!(
        generated.success(),
        "generate an independent loopback test certificate"
    );
    let script = r#"
import socket, ssl, sys
ctx = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
ctx.load_cert_chain(sys.argv[1], sys.argv[2])
listener = socket.socket()
listener.bind(('127.0.0.1', 0))
listener.listen(1)
listener.settimeout(5)
print(listener.getsockname()[1], flush=True)
peer, _ = listener.accept()
try:
    with ctx.wrap_socket(peer, server_side=True) as tls:
        if tls.recv(16384):
            print('HTTP_REQUEST_RECEIVED', flush=True)
            tls.sendall(b'HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}')
except ssl.SSLError:
    pass
listener.close()
"#;
    let mut server = ChildGuard(
        Command::new("python3")
            .args(["-u", "-c", script])
            .arg(&certificate)
            .arg(&key)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    let mut stdout = std::io::BufReader::new(server.0.stdout.take().unwrap());
    let mut port = String::new();
    stdout.read_line(&mut port).unwrap();
    let port: u16 = port.trim().parse().expect("independent TLS server port");
    let origin = format!("https://127.0.0.1:{port}");
    let entry = Uri::parse(&format!("{origin}/root.json")).unwrap();
    let manifest = write_manifest(root.path(), &entry, vec![pin(&entry, b"{}")]);
    fs::write(root.path().join(".curlrc"), "insecure\n").unwrap();
    let result = Command::new(std::env::current_exe().unwrap())
        .args(["--ignored", "--exact", "environment_child"])
        .env("SUSPECT_PIN_CHILD_MANIFEST", &manifest)
        .env("SUSPECT_PIN_CHILD_ORIGIN", &origin)
        .env("SUSPECT_PIN_CHILD_CACHE", root.path().join("cache"))
        .env("SUSPECT_PIN_CHILD_EXPECT_TLS", "1")
        .env("CURL_HOME", root.path())
        .env("HOME", root.path())
        .env("CURL_CA_BUNDLE", &certificate)
        .env("SSL_CERT_FILE", &certificate)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{} {}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
    let mut observed = String::new();
    stdout.read_to_string(&mut observed).unwrap();
    assert!(
        !observed.contains("HTTP_REQUEST_RECEIVED"),
        "verification must fail before HTTP or credentials are sent"
    );
}

#[test]
fn ambient_proxies_curlrc_and_credentials_are_ignored_by_the_real_transport() {
    let root = tempdir();
    let server = Server::new(|_| response(200, &[("Content-Type", "application/json")], b"{}"));
    let proxy = Server::new(|_| response(502, &[], b"ambient proxy was used"));
    let entry = server.uri("/root.json");
    let manifest = write_manifest(root.path(), &entry, vec![pin(&entry, b"{}")]);
    fs::write(
        root.path().join(".curlrc"),
        format!(
            "proxy = \"{}\"\nheader = \"Authorization: ambient-secret\"\nretry = 3\n",
            proxy.origin
        ),
    )
    .unwrap();
    fs::write(
        root.path().join(".netrc"),
        "default login ambient-user password ambient-secret\n",
    )
    .unwrap();
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--ignored", "--exact", "environment_child"])
        .env("SUSPECT_PIN_CHILD_MANIFEST", &manifest)
        .env("SUSPECT_PIN_CHILD_ORIGIN", &server.origin)
        .env("SUSPECT_PIN_CHILD_CACHE", root.path().join("cache"))
        .env("CURL_HOME", root.path())
        .env("HOME", root.path())
        .env("XDG_CONFIG_HOME", root.path())
        .env("http_proxy", &proxy.origin)
        .env("HTTP_PROXY", &proxy.origin)
        .env("https_proxy", &proxy.origin)
        .env("HTTPS_PROXY", &proxy.origin)
        .env("all_proxy", &proxy.origin)
        .env("ALL_PROXY", &proxy.origin)
        .env("NO_PROXY", "")
        .env("no_proxy", "")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{} {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(proxy.requests().is_empty());
    let requests = server.requests();
    assert_eq!(requests.len(), 1);
    assert!(!requests[0].headers.contains_key("authorization"));
    assert!(!requests[0].headers.contains_key("proxy-authorization"));
    assert!(!requests[0].headers.contains_key("cookie"));
}

#[test]
#[ignore = "invoked in a subprocess with isolated hostile environment by the parent test"]
fn environment_child() {
    let manifest = std::env::var_os("SUSPECT_PIN_CHILD_MANIFEST").unwrap();
    let origin = std::env::var("SUSPECT_PIN_CHILD_ORIGIN").unwrap();
    let cache = std::env::var_os("SUSPECT_PIN_CHILD_CACHE").unwrap();
    let tls = std::env::var_os("SUSPECT_PIN_CHILD_EXPECT_TLS").is_some();
    let result = acquire(
        std::path::Path::new(&manifest),
        AcquireOptions {
            cache_dir: cache.into(),
            insecure_test_origins: if tls { vec![] } else { vec![origin] },
            timeout: Duration::from_secs(3),
            ..AcquireOptions::default()
        },
    );
    if tls {
        assert_eq!(result.unwrap_err().kind(), &AcquireErrorKind::Tls);
        return;
    }
    let closure = result.unwrap();
    let workspace = closure.workspace_builder().build().unwrap();
    assert_eq!(
        workspace
            .open(closure.entry().as_str())
            .unwrap()
            .doc()
            .inner()
            .bytes(),
        b"{}"
    );
}
