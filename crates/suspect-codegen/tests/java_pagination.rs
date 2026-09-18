//! Emitted-only pagination for the Java HTTP backend: one generated
//! `Pagination.java` with closeable page/item iterators and a next-input
//! builder per paginated operation, a javac compile gate over the whole
//! package, and native behavior against a stubbed `HttpClient`. Static runtime
//! files are never modified, and no-policy generation emits no new file at all.
#![cfg(all(feature = "java-sdk", feature = "http-protocol"))]

use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::Arc,
};

use serde_json::{Value, json};
use suspect_codegen::{
    OutFile,
    backend::{self, Backend, GenerationOptions, TargetConfig},
    sdk_defaults::SdkDefaults,
};
use suspect_ir::contract::Contract;
use suspect_ref::{DocumentProvider, ProvidedDocument, WorkspaceBuilder};
use suspect_source::Uri;

fn document() -> Value {
    json!({
        "openapi": "3.1.0",
        "info": {"title": "Pagination", "version": "1"},
        "servers": [{"url": "https://api.pagination.test/v1"}],
        "paths": {
            "/widgets": {"get": {
                "operationId": "listWidgets",
                "parameters": [
                    {"name": "limit", "in": "query", "schema": {"type": "integer"}},
                    {"name": "offset", "in": "query", "schema": {"type": "integer"}},
                    {"name": "filter", "in": "query", "schema": {"type": "string"}}
                ],
                "responses": {"200": {"description": "Page", "content": {"application/json": {"schema": {
                    "type": "object",
                    "properties": {
                        "data": {"type": "array", "items": {"$ref": "#/components/schemas/Widget"}},
                        "total": {"type": "integer"},
                        "has_more": {"type": "boolean"}
                    }
                }}}}}
            }},
            "/events": {"get": {
                "operationId": "listEvents",
                "parameters": [
                    {"name": "cursor", "in": "query", "schema": {"type": "string"}}
                ],
                "responses": {"200": {"description": "Page", "content": {"application/json": {"schema": {
                    "type": "object",
                    "properties": {
                        "items": {"type": "array", "items": {"type": "string"}},
                        "next_page_token": {"type": "string"}
                    }
                }}}}}
            }},
            "/keys": {"get": {
                "operationId": "scanKeys",
                "parameters": [
                    {"name": "cursor", "in": "query", "schema": {"type": "string"}}
                ],
                "responses": {"200": {"description": "Page", "content": {"application/json": {"schema": {"type": "object", "properties": {"next_page_token": {"type": "string"}}}}}}}
            }},
        },
        "components": {"schemas": {"Widget": {
            "type": "object",
            "properties": {"id": {"type": "string"}}
        }}}
    })
}

fn contract() -> Arc<Contract> {
    let entry = Uri::parse("https://source.pagination.test/java-pagination.json").unwrap();
    let provider = Arc::new(
        DocumentProvider::new([ProvidedDocument::new(
            entry.clone(),
            entry.clone(),
            serde_json::to_vec(&document()).unwrap(),
        )
        .unwrap()])
        .unwrap(),
    );
    let workspace = Arc::new(
        WorkspaceBuilder::new()
            .allowed_documents(provider.logical_uris())
            .document_provider(provider)
            .build()
            .unwrap(),
    );
    Arc::new(Contract::from_workspace(&workspace, &entry).unwrap())
}

fn generate(options: &GenerationOptions) -> Vec<OutFile> {
    let contract = contract();
    let selected = contract
        .operations()
        .map(|operation| operation.source().clone())
        .collect::<Vec<_>>();
    backend::generate_with_options(
        contract,
        &selected,
        &TargetConfig {
            backend: Backend::JavaHttp,
            package_name: "test.suspect:pagination-java".into(),
            package_version: "1.0.0".into(),
            import_name: None,
        },
        options,
    )
    .unwrap()
}

fn configured() -> GenerationOptions {
    GenerationOptions {
        sdk_defaults: Some(SdkDefaults::v1()),
        ..GenerationOptions::default()
    }
}

fn pagination_file(files: &[OutFile]) -> &OutFile {
    files
        .iter()
        .find(|file| file.path == "java/src/main/java/test/suspect/Pagination.java")
        .expect("generated Pagination.java")
}

fn sorted(files: &[OutFile]) -> std::collections::BTreeSet<&str> {
    files.iter().map(|file| file.path.as_str()).collect()
}

#[test]
fn pagination_class_emits_only_under_sdk_defaults() {
    let files = generate(&configured());
    let source = pagination_file(&files).content.clone();
    for expected in [
        // Per-operation API: closeable page iterator, item iterator and
        // next-input builder.
        "public static final class ListWidgetsPages implements java.util.Iterator<Client.ListWidgetsStatus200>, AutoCloseable {",
        "public static final class ListWidgetsItems implements java.util.Iterator<Widget>, AutoCloseable {",
        "public static ListWidgetsPages listWidgetsPages(Client client, Client.ListWidgetsInput input)",
        "public static ListWidgetsItems listWidgetsItems(Client client, Client.ListWidgetsInput input)",
        "public static java.util.Optional<Client.ListWidgetsInput> listWidgetsNextPage(Client client, Client.ListWidgetsInput input)",
        "public static final class ListEventsPages implements java.util.Iterator<Client.ListEventsStatus200>, AutoCloseable {",
        "public static final class ListEventsItems implements java.util.Iterator<String>, AutoCloseable {",
        // Typed pointer readers over generated chains.
        "static java.util.List<Widget> items(Client.ListWidgetsStatus200 page)",
        "static String cursor(Client.ListEventsStatus200 page)",
        // Traversal semantics.
        "if (pending != null) return true;",
        "next = continuation(page, next);",
        "client.listWidgets(next);",
        "client.listEvents(next);",
        "if (token == null || token.isEmpty()) return null;",
        "if (token.equals(previous)) throw stalled(SOURCE);",
        "if (count == 0) return null;",
        "base.add(java.math.BigInteger.valueOf(count))",
        "throw new java.util.NoSuchElementException(\"the pagination walk has ended\");",
        "private static SdkException stalled(String source)",
        "new SdkException(\"pagination-stalled\", source, 0, new byte[0], false)",
    ] {
        assert!(
            source.contains(expected),
            "Pagination.java is missing:\n{expected}\n--- emitted: ---\n{source}"
        );
    }
    // Without configured defaults the walk file is absent and every other
    // emitted file stays byte-identical.
    let control = generate(&GenerationOptions::default());
    assert!(
        !control
            .iter()
            .any(|file| file.path.ends_with("Pagination.java")),
        "no-policy output must not carry pagination walks"
    );
    assert_eq!(
        files.len(),
        control.len() + 1,
        "the configured policy may add exactly one file"
    );
    for file in &control {
        let emitted = files
            .iter()
            .find(|candidate| candidate.path == file.path)
            .unwrap_or_else(|| panic!("{} disappeared under the policy", file.path));
        assert_eq!(emitted.content, file.content, "{} changed", file.path);
    }
}

/// A manual mapping may declare a has-more indicator; every emitted walk must
/// then stop on `false` even when the page still returned items. The runtime
/// behavior of the identical stop rule is verified by the PHP suite and by the
/// stubbed probe above for the shared rules.
#[test]
fn manual_has_more_mapping_emits_the_false_stop_rule() {
    let defaults: SdkDefaults = serde_json::from_value(json!({
        "version": "v1",
        "pagination": {
            "operations": {
                "listWidgets": {
                    "pattern": "limit-offset",
                    "request": {"limit": "limit", "offset": "offset"},
                    "response": {"items": "/data", "has-more": "/has_more"},
                    "initial_offset": 0,
                    "advance": "items-returned"
                }
            }
        }
    }))
    .unwrap();
    let files = generate(&GenerationOptions {
        sdk_defaults: Some(defaults),
        ..GenerationOptions::default()
    });
    let source = pagination_file(&files).content.clone();
    assert_eq!(
        source
            .matches("Boolean.FALSE.equals(hasMore(page))")
            .count(),
        1,
        "the shared continuation must stop on a false has-more indicator; the item walk inherits it:\n{source}"
    );
}

/// A detected cursor walk without a recognized items pointer (scanKeys) still
/// emits the page walk and the next-input builder; only the flattening item
/// walk is omitted.
#[test]
fn cursor_without_items_emits_the_page_walk_only() {
    let files = generate(&configured());
    let source = pagination_file(&files).content.clone();
    assert!(
        source.contains("public static final class ScanKeysPages implements java.util.Iterator<Client.ScanKeysStatus200>, AutoCloseable {"),
        "{source}"
    );
    assert!(
        source.contains("public static java.util.Optional<Client.ScanKeysInput> scanKeysNextPage(Client client, Client.ScanKeysInput input)"),
        "{source}"
    );
    assert!(!source.contains("ScanKeysItems"), "{source}");
    assert!(!source.contains("scanKeysItems"), "{source}");
}

#[test]
fn policy_without_paginated_operations_emits_nothing_new() {
    let off = GenerationOptions {
        sdk_defaults: Some(
            serde_json::from_value(json!({"version": "v1", "pagination": "off"})).unwrap(),
        ),
        ..GenerationOptions::default()
    };
    let disabled = generate(&off);
    let control = generate(&GenerationOptions::default());
    assert_eq!(sorted(&disabled), sorted(&control));
    assert!(
        !disabled
            .iter()
            .any(|file| file.path.ends_with("Pagination.java"))
    );
}

/// A JDK 21+ toolchain, mirroring the other Java acceptance tests.
fn java_home() -> PathBuf {
    std::env::var_os("SUSPECT_JAVA_HOME")
        .or_else(|| std::env::var_os("JAVA_HOME"))
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            "/Users/luke/.local/share/mise/installs/java/temurin-21.0.12+101.0.LTS".into()
        })
}

/// Compile the whole emitted package; the classes directory doubles as the
/// behavioral probe's classpath, including the generated resources.
fn compiled_package(root: &Path) -> PathBuf {
    let home = java_home();
    assert!(
        home.join("bin/javac").is_file(),
        "required JDK: {}",
        home.display()
    );
    let mut sources = fs::read_dir(root.join("java/src/main/java/test/suspect"))
        .unwrap()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().and_then(|extension| extension.to_str()) == Some("java"))
        .collect::<Vec<_>>();
    sources.sort();
    let classes = root.join("classes");
    fs::create_dir_all(&classes).unwrap();
    let list = root.join("sources.txt");
    fs::write(
        &list,
        sources
            .iter()
            .map(|path| path.display().to_string())
            .collect::<Vec<_>>()
            .join("\n"),
    )
    .unwrap();
    let output = Command::new(home.join("bin/javac"))
        .args(["--release", "21", "-Xlint:all", "-Werror", "-d"])
        .arg(&classes)
        .arg(format!("@{}", list.display()))
        .current_dir(root)
        .output()
        .unwrap();
    fs::write(root.join("javac.stdout.log"), &output.stdout).unwrap();
    fs::write(root.join("javac.stderr.log"), &output.stderr).unwrap();
    assert!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    for entry in fs::read_dir(root.join("java/src/main/resources/test/suspect"))
        .unwrap()
        .flatten()
    {
        fs::copy(
            entry.path(),
            classes.join("test/suspect").join(entry.file_name()),
        )
        .unwrap();
    }
    classes
}

#[test]
fn generated_pagination_compiles_strictly_with_the_package() {
    let root = tempfile::tempdir().unwrap();
    suspect_codegen::write_files(&generate(&configured()), root.path()).unwrap();
    compiled_package(root.path());
}

#[test]
fn pagination_walks_drive_stubbed_pages_in_java() {
    let root = tempfile::tempdir().unwrap();
    let files = generate(&configured());
    suspect_codegen::write_files(&files, root.path()).unwrap();
    let classes = compiled_package(root.path());
    fs::write(root.path().join("PaginationProbe.java"), PROBE).unwrap();
    let home = java_home();
    let output = Command::new(home.join("bin/javac"))
        .args(["--release", "21", "-Xlint:all", "-Werror", "-cp"])
        .arg(&classes)
        .arg("-d")
        .arg(root.path().join("probe"))
        .arg(root.path().join("PaginationProbe.java"))
        .current_dir(root.path())
        .output()
        .unwrap();
    fs::write(root.path().join("probe-javac.stderr.log"), &output.stderr).unwrap();
    assert!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let mut classpath = classes.into_os_string();
    classpath.push(":");
    classpath.push(root.path().join("probe").into_os_string());
    let runtime = Command::new(home.join("bin/java"))
        .args(["-ea", "-cp"])
        .arg(&classpath)
        .arg("PaginationProbe")
        .current_dir(root.path())
        .output()
        .unwrap();
    fs::write(root.path().join("probe.stdout.log"), &runtime.stdout).unwrap();
    fs::write(root.path().join("probe.stderr.log"), &runtime.stderr).unwrap();
    assert!(
        runtime.status.success(),
        "{}{}",
        String::from_utf8_lossy(&runtime.stdout),
        String::from_utf8_lossy(&runtime.stderr)
    );
}

const PROBE: &str = r#"
import java.net.*;
import java.net.http.*;
import java.nio.ByteBuffer;
import java.nio.charset.StandardCharsets;
import java.time.Duration;
import java.util.*;
import java.util.concurrent.*;
import javax.net.ssl.*;

import test.suspect.*;
import static test.suspect.JsonRuntime.*;

/** Independent pagination acceptance over a stubbed JDK transport. */
public class PaginationProbe {
    static void check(boolean value, String message) { if (!value) throw new AssertionError(message); }

    static final class Stub extends HttpClient {
        final List<String> queries = Collections.synchronizedList(new ArrayList<>());
        private final List<String> pages;
        Stub(String[] pages) { this.pages = List.of(pages); }
        @Override public Optional<CookieHandler> cookieHandler() { return Optional.empty(); }
        @Override public Optional<Duration> connectTimeout() { return Optional.empty(); }
        @Override public Redirect followRedirects() { return Redirect.NEVER; }
        @Override public Optional<ProxySelector> proxy() { return Optional.empty(); }
        @Override public SSLContext sslContext() { try { return SSLContext.getDefault(); } catch (Exception error) { throw new AssertionError(error); } }
        @Override public SSLParameters sslParameters() { return new SSLParameters(); }
        @Override public Optional<Authenticator> authenticator() { return Optional.empty(); }
        @Override public Version version() { return Version.HTTP_1_1; }
        @Override public Optional<Executor> executor() { return Optional.empty(); }
        @Override public <T> HttpResponse<T> send(HttpRequest request, HttpResponse.BodyHandler<T> handler) throws InterruptedException, java.io.IOException {
            try { return sendAsync(request, handler).get(); }
            catch (ExecutionException error) { throw new java.io.IOException("stub failure"); }
        }
        @Override public <T> CompletableFuture<HttpResponse<T>> sendAsync(HttpRequest request, HttpResponse.BodyHandler<T> handler, HttpResponse.PushPromiseHandler<T> push) { return sendAsync(request, handler); }
        @Override public <T> CompletableFuture<HttpResponse<T>> sendAsync(HttpRequest request, HttpResponse.BodyHandler<T> handler) {
            int index = queries.size();
            queries.add(Objects.requireNonNullElse(request.uri().getRawQuery(), ""));
            check(index < pages.size(), "unexpected extra request: " + request.uri());
            HttpHeaders headers = HttpHeaders.of(Map.of("Content-Type", List.of("application/json")), (key, value) -> true);
            HttpResponse.BodySubscriber<T> subscriber = handler.apply(new HttpResponse.ResponseInfo() {
                @Override public int statusCode() { return 200; }
                @Override public HttpHeaders headers() { return headers; }
                @Override public Version version() { return Version.HTTP_1_1; }
            });
            CompletableFuture<HttpResponse<T>> result = new CompletableFuture<>();
            subscriber.onSubscribe(new Flow.Subscription() {
                boolean delivered;
                @Override public void request(long count) {
                    if (delivered) return;
                    delivered = true;
                    subscriber.onNext(List.of(ByteBuffer.wrap(pages.get(index).getBytes(StandardCharsets.UTF_8))));
                    subscriber.onComplete();
                }
                @Override public void cancel() {}
            });
            subscriber.getBody().whenComplete((body, error) -> {
                if (error != null) result.completeExceptionally(error);
                else result.complete(new Response<>(request, 200, headers, body));
            });
            return result;
        }
    }

    private record Response<T>(HttpRequest request, int statusCode, HttpHeaders headers, T body) implements HttpResponse<T> {
        @Override public Optional<HttpResponse<T>> previousResponse() { return Optional.empty(); }
        @Override public Optional<SSLSession> sslSession() { return Optional.empty(); }
        @Override public URI uri() { return request.uri(); }
        @Override public HttpClient.Version version() { return HttpClient.Version.HTTP_1_1; }
    }

    private static Client client(Stub stub) {
        return new Client(HttpRuntime.Options.builder()
            .httpClient(stub)
            .serverUrl(URI.create("https://api.pagination.test/v1"))
            .build());
    }

    public static void main(String[] args) {
        // Limit/offset walk: the first page is the direct call's result, later
        // requests change only the offset, and the filter member is preserved.
        Stub stub = new Stub(new String[] {
            "{\"data\":[{\"id\":\"a\"},{\"id\":\"b\"}],\"total\":9}",
            "{\"data\":[{\"id\":\"c\"}],\"total\":9}",
            "{\"data\":[],\"total\":9}"
        });
        try (Client client = client(stub)) {
            List<Client.ListWidgetsStatus200> pages = new ArrayList<>();
            try (Pagination.ListWidgetsPages walk = Pagination.listWidgetsPages(client,
                    Client.ListWidgetsInput.builder().limit(JsonNumber.of(2)).filter("red").build())) {
                for (java.util.Iterator<Client.ListWidgetsStatus200> iterator = walk; iterator.hasNext();) {
                    pages.add(iterator.next());
                }
            }
            check(pages.size() == 3, "three pages walked: " + pages.size());
            check(pages.get(0).data().data().value().get(1).id().value().equals("b"), "first page is the direct result");
            check(pages.get(2).data().data().value().isEmpty(), "zero-item page ends the walk");
            check(stub.queries.get(0).contains("limit=2") && stub.queries.get(0).contains("offset=0") && stub.queries.get(0).contains("filter=red"),
                "first page query: " + stub.queries.get(0));
            check(stub.queries.get(1).contains("offset=2") && stub.queries.get(1).contains("filter=red"),
                "second page query: " + stub.queries.get(1));
            check(stub.queries.get(2).contains("offset=3"), "third page query: " + stub.queries.get(2));

            // A caller-supplied offset wins for page 1 and is replaced afterwards.
            stub = new Stub(new String[] {
                "{\"data\":[{\"id\":\"a\"},{\"id\":\"b\"}],\"total\":9}",
                "{\"data\":[],\"total\":9}"
            });
            try (Client caller = client(stub)) {
                try (Pagination.ListWidgetsPages walk = Pagination.listWidgetsPages(caller,
                        Client.ListWidgetsInput.builder().limit(JsonNumber.of(2)).offset(JsonNumber.of(10)).build())) {
                    while (walk.hasNext()) walk.next();
                }
                check(stub.queries.get(0).contains("offset=10"), "caller offset honored: " + stub.queries.get(0));
                check(stub.queries.get(1).contains("offset=12"), "offset advances from the caller value: " + stub.queries.get(1));
            }

            // Items flatten across pages in order.
            stub = new Stub(new String[] {
                "{\"data\":[{\"id\":\"a\"},{\"id\":\"b\"}],\"total\":9}",
                "{\"data\":[{\"id\":\"c\"}],\"total\":9}",
                "{\"data\":[],\"total\":9}"
            });
            try (Client flattener = client(stub)) {
                List<String> items = new ArrayList<>();
                try (Pagination.ListWidgetsItems walk = Pagination.listWidgetsItems(flattener,
                        Client.ListWidgetsInput.builder().limit(JsonNumber.of(2)).build())) {
                    while (walk.hasNext()) items.add(walk.next().id().value());
                }
                check(items.equals(List.of("a", "b", "c")), "items flatten: " + items);
                check(stub.queries.size() == 3, "three requests: " + stub.queries.size());
            }

            // Early break never starts the next request; close before the first
            // hasNext() issues no request at all.
            stub = new Stub(new String[] {
                "{\"data\":[{\"id\":\"a\"}],\"total\":9}",
                "{\"data\":[{\"id\":\"b\"}],\"total\":9}"
            });
            try (Client breaker = client(stub)) {
                try (Pagination.ListWidgetsPages walk = Pagination.listWidgetsPages(breaker,
                        Client.ListWidgetsInput.builder().limit(JsonNumber.of(1)).build())) {
                    for (java.util.Iterator<Client.ListWidgetsStatus200> iterator = walk; iterator.hasNext();) {
                        Client.ListWidgetsStatus200 page = iterator.next();
                        check(page.data().data().value().size() == 1, "one item");
                        break;
                    }
                }
                check(stub.queries.size() == 1, "early break issues no second request: " + stub.queries.size());
                Pagination.ListWidgetsPages closed = Pagination.listWidgetsPages(breaker,
                    Client.ListWidgetsInput.builder().limit(JsonNumber.of(1)).build());
                closed.close();
                check(stub.queries.size() == 1, "close before the first hasNext() issues no request");
            }

            // Cursor walk: the cursor is absent on page 1 and resolved afterwards.
            stub = new Stub(new String[] {
                "{\"items\":[\"i1\",\"i2\"],\"next_page_token\":\"c2\"}",
                "{\"items\":[\"i3\"]}"
            });
            try (Client cursorClient = client(stub)) {
                List<String> walked = new ArrayList<>();
                try (Pagination.ListEventsItems walk = Pagination.listEventsItems(cursorClient,
                        Client.ListEventsInput.builder().build())) {
                    while (walk.hasNext()) walked.add(walk.next());
                }
                check(walked.equals(List.of("i1", "i2", "i3")), "cursor items: " + walked);
                check(!stub.queries.get(0).contains("cursor="), "first cursor page omits the cursor: " + stub.queries.get(0));
                check(stub.queries.get(1).contains("cursor=c2"), "second cursor page: " + stub.queries.get(1));

                // A caller-supplied cursor wins for page 1.
                stub = new Stub(new String[] {
                    "{\"items\":[\"x\"],\"next_page_token\":\"c9\"}",
                    "{\"items\":[\"y\"]}"
                });
                Stub overrideStub = stub;
                try (Client overrideClient = new Client(HttpRuntime.Options.builder()
                        .httpClient(overrideStub).serverUrl(URI.create("https://api.pagination.test/v1")).build())) {
                    try (Pagination.ListEventsPages walk = Pagination.listEventsPages(overrideClient,
                            Client.ListEventsInput.builder().cursor("c1").build())) {
                        while (walk.hasNext()) walk.next();
                    }
                    check(stub.queries.get(0).contains("cursor=c1"), "caller cursor honored");
                    check(stub.queries.get(1).contains("cursor=c9"), "caller cursor replaced: " + stub.queries.get(1));
                }

                // An empty cursor token stops the walk.
                stub = new Stub(new String[] {
                    "{\"items\":[\"x\"],\"next_page_token\":\"\"}",
                    "{\"items\":[\"loop\"]}"
                });
                Stub emptyStub = stub;
                try (Client emptyClient = new Client(HttpRuntime.Options.builder()
                        .httpClient(emptyStub).serverUrl(URI.create("https://api.pagination.test/v1")).build())) {
                    try (Pagination.ListEventsPages walk = Pagination.listEventsPages(emptyClient,
                            Client.ListEventsInput.builder().build())) {
                        while (walk.hasNext()) walk.next();
                    }
                    check(stub.queries.size() == 1, "empty cursor issues one request: " + stub.queries.size());
                }

                // A repeated identical continuation value throws the typed error.
                stub = new Stub(new String[] {
                    "{\"items\":[\"x\"],\"next_page_token\":\"again\"}",
                    "{\"items\":[\"x\"],\"next_page_token\":\"again\"}",
                    "{\"items\":[\"x\"],\"next_page_token\":\"again\"}"
                });
                Stub stallStub = stub;
                try (Client stallClient = new Client(HttpRuntime.Options.builder()
                        .httpClient(stallStub).serverUrl(URI.create("https://api.pagination.test/v1")).build())) {
                    try {
                        try (Pagination.ListEventsPages walk = Pagination.listEventsPages(stallClient,
                                Client.ListEventsInput.builder().build())) {
                            while (walk.hasNext()) walk.next();
                        }
                        throw new AssertionError("expected a pagination-stalled failure");
                    } catch (SdkException error) {
                        check(error.kind().equals("pagination-stalled"), "stall kind: " + error.kind());
                    }
                    check(stub.queries.size() == 2, "the walk stops at the repeated continuation: " + stub.queries.size());
                }
            }

            // NextPage fetches one page and returns the rebuilt input.
            stub = new Stub(new String[] {
                "{\"data\":[{\"id\":\"a\"},{\"id\":\"b\"}],\"total\":9}",
                "{\"data\":[{\"id\":\"c\"}],\"total\":9}",
                "{\"data\":[],\"total\":9}"
            });
            try (Client manual = client(stub)) {
                Client.ListWidgetsInput input = Client.ListWidgetsInput.builder()
                    .limit(JsonNumber.of(2)).filter("red").build();
                int followed = 0;
                for (;;) {
                    Optional<Client.ListWidgetsInput> next = Pagination.listWidgetsNextPage(manual, input);
                    if (next.isEmpty()) break;
                    input = next.get();
                    followed += 1;
                }
                check(followed == 2, "manual walk follows: " + followed);
                check(input.filter().value().equals("red"), "manual walk preserves the filter");
                check(input.offset().value().token().equals("3"), "manual walk offset: " + input.offset().value().token());
                check(stub.queries.size() == 3, "manual walk requests: " + stub.queries.size());
            }
        }
        // A detected cursor walk without a recognized items pointer still
        // walks pages; only the flattening item walk is absent.
        stub = new Stub(new String[] {
            "{\"next_page_token\":\"k2\"}",
            "{\"next_page_token\":\"k9\"}",
            "{}"
        });
        try (Client keysClient = client(stub)) {
            int walked = 0;
            try (Pagination.ScanKeysPages walk = Pagination.scanKeysPages(keysClient,
                    Client.ScanKeysInput.builder().build())) {
                while (walk.hasNext()) {
                    walk.next();
                    walked += 1;
                }
            }
            check(walked == 3 && stub.queries.size() == 3, "items-less cursor walk: " + walked + "/" + stub.queries.size());
            boolean items = java.util.Arrays.stream(Pagination.class.getDeclaredClasses())
                .anyMatch(candidate -> candidate.getSimpleName().equals("ScanKeysItems"));
            check(!items, "no item walk without a mapped items pointer");
        }

        System.out.println("pagination behavior verified");
    }
}
"#;
