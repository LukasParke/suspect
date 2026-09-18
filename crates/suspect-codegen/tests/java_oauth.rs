//! Emitted-only OAuth 2.0 lifecycle for the Java HTTP backend: one generated
//! `OAuth.java` with compiled scheme descriptors, an immutable `TokenSet`, the
//! caller-implementable `TokenStore` plus a synchronized instance-owned
//! `MemoryTokenStore`, client-credentials acquisition with skew-aware caching
//! and per-instance single-flight, explicit refresh, PKCE authorization-code,
//! device polling, revocation and introspection. Static runtime files are
//! never modified; plans without a configured policy — or with only the
//! deprecated implicit and password flows — emit no new file at all, so
//! no-policy output stays byte-identical.
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
};
use suspect_ir::contract::Contract;
use suspect_ref::{DocumentProvider, ProvidedDocument, WorkspaceBuilder};
use suspect_source::Uri;

/// One scheme carrying every executable flow kind, plus configured
/// supplemental endpoints: one emission covers the whole conditional surface.
/// The device-authorization flow is an OAS 3.2 declaration.
fn service_document() -> Value {
    json!({
        "openapi": "3.2.0",
        "info": {"title": "OAuth", "version": "1"},
        "servers": [{"url": "https://api.oauth.test/v1"}],
        "paths": {
            "/widgets": {"get": {
                "operationId": "listWidgets",
                "security": [{"service": ["read"]}],
                "responses": {"200": {"description": "Ok", "content": {"application/json": {"schema": {"type": "string"}}}}}
            }}
        },
        "components": {"securitySchemes": {"service": {
            "type": "oauth2",
            "flows": {
                "clientCredentials": {
                    "tokenUrl": "https://auth.oauth.test/token",
                    "refreshUrl": "https://auth.oauth.test/token-refresh",
                    "scopes": {"read": "Read access"}
                },
                "authorizationCode": {
                    "authorizationUrl": "https://auth.oauth.test/authorize",
                    "tokenUrl": "https://auth.oauth.test/token",
                    "scopes": {"read": "Read access"}
                },
                "deviceAuthorization": {
                    "deviceAuthorizationUrl": "https://auth.oauth.test/device",
                    "tokenUrl": "https://auth.oauth.test/token",
                    "scopes": {"read": "Read access"}
                }
            }
        }}}
    })
}

/// The same shape without the OAuth scheme: the no-scheme control.
fn control_document() -> Value {
    let mut document = service_document();
    document["paths"]["/widgets"]["get"]
        .as_object_mut()
        .unwrap()
        .remove("security");
    document["components"]
        .as_object_mut()
        .unwrap()
        .remove("securitySchemes");
    document
}

/// A scheme whose only declared flows are the deprecated implicit and password
/// grants: represented by the plan, never executed.
fn deprecated_only_document() -> Value {
    json!({
        "openapi": "3.1.0",
        "info": {"title": "OAuth deprecated", "version": "1"},
        "servers": [{"url": "https://api.oauth.test/v1"}],
        "paths": {
            "/widgets": {"get": {
                "operationId": "listWidgets",
                "security": [{"legacy": ["read"]}],
                "responses": {"200": {"description": "Ok", "content": {"application/json": {"schema": {"type": "string"}}}}}
            }}
        },
        "components": {"securitySchemes": {"legacy": {
            "type": "oauth2",
            "flows": {
                "implicit": {
                    "authorizationUrl": "https://auth.oauth.test/implicit-authorize",
                    "scopes": {"read": "Read access"}
                },
                "password": {
                    "tokenUrl": "https://auth.oauth.test/token",
                    "scopes": {"read": "Read access"}
                }
            }
        }}}
    })
}

fn contract(document: &Value) -> Arc<Contract> {
    let entry = Uri::parse("https://source.oauth.test/java-oauth.json").unwrap();
    let provider = Arc::new(
        DocumentProvider::new([ProvidedDocument::new(
            entry.clone(),
            entry.clone(),
            serde_json::to_vec(document).unwrap(),
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

fn generate(document: &Value, options: &GenerationOptions) -> Vec<OutFile> {
    let contract = contract(document);
    let selected = contract
        .operations()
        .map(|operation| operation.source().clone())
        .collect::<Vec<_>>();
    backend::generate_with_options(
        contract,
        &selected,
        &TargetConfig {
            backend: Backend::JavaHttp,
            package_name: "test.suspect:oauth-java".into(),
            package_version: "1.0.0".into(),
            import_name: None,
        },
        options,
    )
    .unwrap()
}

fn configured() -> GenerationOptions {
    GenerationOptions {
        sdk_defaults: Some(
            serde_json::from_value(json!({
                "version": "v1",
                "oauth": {"schemes": {"service": {
                    "client_id_env": "JAVA_OAUTH_CLIENT_ID",
                    "client_secret_env": "JAVA_OAUTH_CLIENT_SECRET",
                    "refresh_skew_seconds": 30,
                    "revocation_endpoint": "https://auth.oauth.test/revoke",
                    "introspection_endpoint": "https://auth.oauth.test/introspect"
                }}}
            }))
            .unwrap(),
        ),
        ..Default::default()
    }
}

fn oauth_file(files: &[OutFile]) -> &OutFile {
    files
        .iter()
        .find(|file| file.path == "java/src/main/java/test/suspect/OAuth.java")
        .expect("generated OAuth.java")
}

fn sorted(files: &[OutFile]) -> std::collections::BTreeSet<&str> {
    files.iter().map(|file| file.path.as_str()).collect()
}

#[test]
fn oauth_class_emits_only_under_a_usable_scheme() {
    let configured = generate(&service_document(), &configured());
    let plain = generate(&service_document(), &GenerationOptions::default());
    let control = generate(&control_document(), &GenerationOptions::default());
    assert!(
        !plain.iter().any(|file| file.path.ends_with("OAuth.java")),
        "no-policy output must not carry the OAuth lifecycle runtime"
    );
    assert!(
        !control.iter().any(|file| file.path.ends_with("OAuth.java")),
        "the control document must not carry the OAuth lifecycle runtime"
    );
    assert_eq!(
        plain
            .iter()
            .map(|file| file.path.as_str())
            .collect::<Vec<_>>(),
        control
            .iter()
            .map(|file| file.path.as_str())
            .collect::<Vec<_>>(),
        "the control document must not add or remove artifact paths"
    );
    assert_eq!(
        configured.len(),
        plain.len() + 1,
        "the configured policy may add exactly one file"
    );
    for file in &plain {
        let emitted = configured
            .iter()
            .find(|candidate| candidate.path == file.path)
            .unwrap_or_else(|| panic!("{} disappeared under the policy", file.path));
        assert_eq!(emitted.content, file.content, "{} changed", file.path);
    }
    let source = oauth_file(&configured).content.clone();
    for expected in [
        "public final class OAuth",
        // Value types and the instance-owned, key-partitioned store.
        "public static final class TokenSet",
        "public interface TokenStore",
        "public static final class MemoryTokenStore implements TokenStore",
        "synchronized TokenSet load(String key)",
        "public static final class AuthException extends RuntimeException",
        // Compiled descriptors as constants: env names compiled, never values.
        "private static final Map<String, Scheme> SCHEMES;",
        "schemes.put(\"service\", new Scheme(",
        "\"JAVA_OAUTH_CLIENT_ID\"",
        "\"JAVA_OAUTH_CLIENT_SECRET\"",
        "\"https://auth.oauth.test/revoke\"",
        "\"https://auth.oauth.test/introspect\"",
        "\"https://auth.oauth.test/token\"",
        "\"https://auth.oauth.test/token-refresh\"",
        "\"https://auth.oauth.test/device\"",
        "\"client-secret-basic\"",
        // Lifecycle methods.
        "public TokenSet clientCredentialsToken(String scheme)",
        "public HttpRuntime.CredentialProvider clientCredentialsProvider(String scheme)",
        "public TokenSet refreshToken(String scheme, TokenSet tokenSet)",
        "public AuthorizationTransaction beginAuthorization(String scheme, URI redirectUri)",
        "public TokenSet completeAuthorization(AuthorizationTransaction transaction, String code, String state)",
        "public DeviceGrant beginDeviceAuthorization(String scheme)",
        "public TokenSet pollDeviceAuthorization(DeviceGrant grant)",
        "public void revoke(String scheme, String token)",
        "public JsonRuntime.JsonObject introspect(String scheme, String token)",
        // Traversal and acquisition semantics.
        "Re-check the store after acquiring the per-key gate",
        "\"grant_type\", \"client_credentials\"",
        "\"grant_type\", \"refresh_token\"",
        "\"grant_type\", \"authorization_code\"",
        "\"urn:ietf:params:oauth:grant-type:device_code\"",
        "\"code_challenge_method\", \"S256\"",
        "new SecureRandom()",
        "MessageDigest.getInstance(\"SHA-256\")",
        "System.getenv(variable)",
        "the device code expired before authorization completed",
    ] {
        assert!(
            source.contains(expected),
            "OAuth.java is missing:\n{expected}\n--- emitted: ---\n{source}"
        );
    }

    // A scheme with only the deprecated implicit and password flows emits
    // nothing: byte-identical to the no-policy package.
    let deprecated = generate(
        &deprecated_only_document(),
        &GenerationOptions {
            sdk_defaults: Some(
                serde_json::from_value(json!({
                    "version": "v1",
                    "oauth": {"schemes": {"legacy": {"client_id_env": "JAVA_OAUTH_CLIENT_ID"}}}
                }))
                .unwrap(),
            ),
            ..Default::default()
        },
    );
    let deprecated_plain = generate(&deprecated_only_document(), &GenerationOptions::default());
    assert!(
        !deprecated
            .iter()
            .any(|file| file.path.ends_with("OAuth.java")),
        "deprecated-only flows must not emit OAuth.java"
    );
    assert_eq!(sorted(&deprecated), sorted(&deprecated_plain));
    for file in &deprecated_plain {
        let emitted = deprecated
            .iter()
            .find(|candidate| candidate.path == file.path)
            .expect("same file set");
        assert_eq!(emitted.content, file.content, "{} changed", file.path);
    }

    // An explicit `off` policy emits nothing as well.
    let off = generate(
        &service_document(),
        &GenerationOptions {
            sdk_defaults: Some(
                serde_json::from_value(json!({"version": "v1", "oauth": "off"})).unwrap(),
            ),
            ..Default::default()
        },
    );
    assert!(!off.iter().any(|file| file.path.ends_with("OAuth.java")));
    assert_eq!(sorted(&off), sorted(&plain));
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
fn generated_oauth_compiles_strictly_with_the_package() {
    let root = tempfile::tempdir().unwrap();
    suspect_codegen::write_files(&generate(&service_document(), &configured()), root.path())
        .unwrap();
    compiled_package(root.path());
}

#[test]
fn oauth_lifecycle_drives_stubbed_transport_in_java() {
    let root = tempfile::tempdir().unwrap();
    let files = generate(&service_document(), &configured());
    suspect_codegen::write_files(&files, root.path()).unwrap();
    let classes = compiled_package(root.path());
    fs::write(root.path().join("OAuthProbe.java"), PROBE).unwrap();
    let home = java_home();
    let output = Command::new(home.join("bin/javac"))
        .args(["--release", "21", "-Xlint:all", "-Werror", "-cp"])
        .arg(&classes)
        .arg("-d")
        .arg(root.path().join("probe"))
        .arg(root.path().join("OAuthProbe.java"))
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
        .arg("OAuthProbe")
        // Compiled environment variable NAMES, real values supplied by the
        // environment at call time.
        .env("JAVA_OAUTH_CLIENT_ID", "cli-id")
        .env("JAVA_OAUTH_CLIENT_SECRET", "cli-secret")
        .current_dir(root.path())
        .output()
        .unwrap();
    fs::write(root.path().join("probe.stdout.log"), &runtime.stdout).unwrap();
    fs::write(root.path().join("probe.stderr.log"), &runtime.stderr).unwrap();
    assert!(
        runtime.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&runtime.stdout),
        String::from_utf8_lossy(&runtime.stderr)
    );
}

const PROBE: &str = r#"
import java.net.Authenticator;
import java.net.CookieHandler;
import java.net.ProxySelector;
import java.net.URI;
import java.net.URLDecoder;
import java.net.http.HttpClient;
import java.net.http.HttpHeaders;
import java.net.http.HttpRequest;
import java.net.http.HttpResponse;
import java.nio.ByteBuffer;
import java.nio.charset.StandardCharsets;
import java.io.ByteArrayOutputStream;
import java.security.MessageDigest;
import java.time.Duration;
import java.util.*;
import java.util.concurrent.*;
import java.util.concurrent.Flow;
import javax.net.ssl.*;

import test.suspect.*;

import static test.suspect.JsonRuntime.*;

/** Independent OAuth lifecycle acceptance over a stubbed JDK transport. */
public class OAuthProbe {
    static void check(boolean value, String message) { if (!value) throw new AssertionError(message); }

    static final String TOKEN = "https://auth.oauth.test/token";
    static final String REFRESH = "https://auth.oauth.test/token-refresh";
    static final String REVOKE = "https://auth.oauth.test/revoke";
    static final String INTROSPECT = "https://auth.oauth.test/introspect";
    static final String DEVICE = "https://auth.oauth.test/device";
    static final String AUTHORIZE = "https://auth.oauth.test/authorize";

    static HttpHeaders jsonHeaders() {
        return HttpHeaders.of(Map.of("Content-Type", List.of("application/json")), (key, value) -> true);
    }

    /** A bytestring response carrying a status and JSON content type. */
    static final class Answer<T> implements HttpResponse<T> {
        private final HttpRequest request;
        private final int status;
        private final HttpHeaders headers;
        private final byte[] body;

        Answer(HttpRequest request, int status, String body) {
            this.request = request;
            this.status = status;
            this.headers = jsonHeaders();
            this.body = body.getBytes(StandardCharsets.UTF_8);
        }

        @SuppressWarnings("unchecked")
        @Override public T body() { return (T) body; }
        @Override public int statusCode() { return status; }
        @Override public HttpRequest request() { return request; }
        @Override public Optional<HttpResponse<T>> previousResponse() { return Optional.empty(); }
        @Override public HttpHeaders headers() { return headers; }
        @Override public Optional<javax.net.ssl.SSLSession> sslSession() { return Optional.empty(); }
        @Override public URI uri() { return request.uri(); }
        @Override public HttpClient.Version version() { return HttpClient.Version.HTTP_1_1; }
    }

    /** Routes requests by URL; a list of bodies answers in request order, then repeats its last entry. */
    static class Stub extends HttpClient {
        final List<HttpRequest> requests = Collections.synchronizedList(new ArrayList<>());
        final Map<String, List<String>> responses = new ConcurrentHashMap<>();
        final Map<String, Integer> counts = new ConcurrentHashMap<>();

        Stub put(String url, String... bodies) { responses.put(url, List.of(bodies)); return this; }

        @Override public Optional<CookieHandler> cookieHandler() { return Optional.empty(); }
        @Override public Optional<Duration> connectTimeout() { return Optional.empty(); }
        @Override public Redirect followRedirects() { return Redirect.NEVER; }
        @Override public Optional<ProxySelector> proxy() { return Optional.empty(); }
        @Override public SSLContext sslContext() { try { return SSLContext.getDefault(); } catch (Exception error) { throw new AssertionError(error); } }
        @Override public SSLParameters sslParameters() { return new SSLParameters(); }
        @Override public Optional<Authenticator> authenticator() { return Optional.empty(); }
        @Override public Version version() { return Version.HTTP_1_1; }
        @Override public Optional<Executor> executor() { return Optional.empty(); }
        @Override public <T> HttpResponse<T> send(HttpRequest request, HttpResponse.BodyHandler<T> handler) {
            return route(request);
        }
        @Override public <T> CompletableFuture<HttpResponse<T>> sendAsync(HttpRequest request, HttpResponse.BodyHandler<T> handler, HttpResponse.PushPromiseHandler<T> push) { return sendAsync(request, handler); }
        @Override public <T> CompletableFuture<HttpResponse<T>> sendAsync(HttpRequest request, HttpResponse.BodyHandler<T> handler) {
            return CompletableFuture.completedFuture(route(request));
        }

        /** The routing hook subclasses override for stateful responses. */
        <T> HttpResponse<T> route(HttpRequest request) {
            requests.add(request);
            List<String> bodies = responses.get(request.uri().toString());
            check(bodies != null || !responses.isEmpty(), "unexpected request: " + request.uri());
            if (bodies == null) {
                // Stateful subclasses answer without a pre-registered body list.
                return new Answer<>(request, 200, "{}");
            }
            int index = counts.merge(request.uri().toString(), 1, Integer::sum) - 1;
            String answer = bodies.get(Math.min(index, bodies.size() - 1));
            int status = 200;
            String body = answer;
            if (answer.length() > 4 && answer.charAt(3) == '|' && answer.chars().limit(3).allMatch(c -> c >= '0' && c <= '9')) {
                status = Integer.parseInt(answer.substring(0, 3));
                body = answer.substring(4);
            }
            return new Answer<>(request, status, body);
        }
    }

    static String form(HttpRequest request) {
        if (request.bodyPublisher().isEmpty()) return "";
        ByteArrayOutputStream out = new ByteArrayOutputStream();
        request.bodyPublisher().get().subscribe(new Flow.Subscriber<>() {
            @Override public void onSubscribe(Flow.Subscription subscription) { subscription.request(Long.MAX_VALUE); }
            @Override public void onNext(ByteBuffer item) { byte[] bytes = new byte[item.remaining()]; item.get(bytes); out.writeBytes(bytes); }
            @Override public void onError(Throwable error) { throw new RuntimeException(error); }
            @Override public void onComplete() {}
        });
        return new String(out.toByteArray(), StandardCharsets.UTF_8);
    }

    static Map<String, String> fields(String form) {
        Map<String, String> values = new LinkedHashMap<>();
        for (String pair : form.split("&")) {
            if (pair.isEmpty()) continue;
            int equals = pair.indexOf('=');
            values.put(URLDecoder.decode(pair.substring(0, equals), StandardCharsets.UTF_8),
                URLDecoder.decode(pair.substring(equals + 1), StandardCharsets.UTF_8));
        }
        return values;
    }

    static String basic(HttpRequest request) {
        String value = request.headers().firstValue("authorization").orElse("");
        check(value.startsWith("Basic "), "expected Basic authentication: " + value);
        return new String(Base64.getDecoder().decode(value.substring(6)), StandardCharsets.UTF_8);
    }

    static String base64Url(byte[] bytes) {
        return Base64.getUrlEncoder().withoutPadding().encodeToString(bytes);
    }

    public static void main(String[] args) throws Exception {
        // Controllable clock and recorded sleeper.
        long[] now = {1_000_000L};
        java.util.function.LongSupplier clock = () -> now[0];
        List<Long> sleeps = Collections.synchronizedList(new ArrayList<>());
        java.util.function.LongConsumer sleeper = sleeps::add;

        // ---- Client credentials: acquisition, caching, env identity, Basic auth ----
        Stub stub = new Stub().put(TOKEN,
            "{\"access_token\":\"at-1\",\"token_type\":\"Bearer\",\"expires_in\":3600,\"refresh_token\":\"rt-1\",\"scope\":\"read\"}",
            "{\"access_token\":\"at-2\",\"token_type\":\"Bearer\",\"expires_in\":3600,\"refresh_token\":\"rt-2\"}",
            "{\"access_token\":\"at-3\",\"token_type\":\"bearer\",\"expires_in\":3600}");
        OAuth oauth = new OAuth(null, stub, clock, sleeper);
        OAuth.TokenSet set = oauth.clientCredentialsToken("service");
        check(set.accessToken.equals("at-1"), "acquired token: " + set.accessToken);
        check(set.tokenType.equals("Bearer"), "token type");
        check(set.expiresAt == now[0] + 3600_000L, "expiry: " + set.expiresAt);
        check("rt-1".equals(set.refreshToken), "rotated refresh token adopted on acquisition");
        check("read".equals(set.scope), "scope");
        check(stub.requests.size() == 1, "exactly one token request: " + stub.requests.size());
        check(basic(stub.requests.get(0)).equals("cli-id:cli-secret"), "client-secret-basic sends RFC 6749 2.3.1 Basic credentials");
        Map<String, String> first = fields(form(stub.requests.get(0)));
        check("client_credentials".equals(first.get("grant_type")), "grant type");
        check(!first.containsKey("client_id"), "confidential clients never send a body client id");
        check(!first.containsKey("scope"), "no scope is inferred from operations");

        // Cache hit: a fresh stored set is served without another request.
        check(oauth.clientCredentialsToken("service").accessToken.equals("at-1") && stub.requests.size() == 1, "skew-aware cache hit");

        // Expiry past the compiled skew triggers re-acquisition; the previous
        // refresh token is retained when the response carries none.
        now[0] += 3_560_000L; // Forty seconds before expiry: still fresh beyond the 30s skew.
        check(oauth.clientCredentialsToken("service").accessToken.equals("at-1") && stub.requests.size() == 1, "a token outliving the skew stays cached");
        now[0] += 11_000L; // Twenty-nine seconds before expiry: inside the 30s skew, stale.
        OAuth.TokenSet rotated = oauth.clientCredentialsToken("service");
        check(rotated.accessToken.equals("at-2"), "re-acquired past the skew");
        check(stub.requests.size() == 2, "one new request: " + stub.requests.size());
        check("rt-2".equals(rotated.refreshToken), "rotated refresh token adopted");
        now[0] += 3_601_000L;
        OAuth.TokenSet retained = oauth.clientCredentialsToken("service");
        check(retained.accessToken.equals("at-3"), "re-acquired again");
        check("rt-2".equals(retained.refreshToken), "previous refresh token retained when none is issued");
        check(retained.tokenType.equals("bearer"), "declared token type is kept");

        // Explicit client identity wins over the compiled environment variables.
        stub.put(TOKEN, "{\"access_token\":\"at-4\",\"expires_in\":3600}");
        oauth.clientCredentialsToken("service", "explicit-id", "explicit-secret", null);
        check(basic(stub.requests.get(stub.requests.size() - 1)).equals("explicit-id:explicit-secret"), "explicit identity wins");

        // Unknown scheme is a typed failure.
        try {
            oauth.clientCredentialsToken("nope");
            throw new AssertionError("expected an unknown-scheme failure");
        } catch (OAuth.AuthException error) {
            check(error.kind.equals("unknown-scheme"), "unknown scheme kind: " + error.kind);
        }

        // ---- Single-flight: a caller arriving on a pending gate re-checks the store ----
        OAuth.TokenStore doubleCheck = new OAuth.TokenStore() {
            boolean first = true;
            @Override public OAuth.TokenSet load(String key) {
                if (first) { first = false; return null; }
                return new OAuth.TokenSet("double-checked");
            }
            @Override public void replace(String key, OAuth.TokenSet tokenSet) {}
            @Override public void clear(String key) {}
        };
        Stub gateStub = new Stub().put(TOKEN, "{\"access_token\":\"never\",\"expires_in\":3600}");
        OAuth gated = new OAuth(doubleCheck, gateStub, clock, sleeper);
        check(gated.clientCredentialsToken("service").accessToken.equals("double-checked"), "the gate re-check serves the store without a duplicate request");
        check(gateStub.requests.isEmpty(), "single-flight re-check issues no token request: " + gateStub.requests.size());

        // ---- Concurrent callers share one acquisition per store key ----
        Stub concurrentStub = new Stub().put(TOKEN,
            "{\"access_token\":\"race-1\",\"expires_in\":3600}",
            "{\"access_token\":\"race-2\",\"expires_in\":3600}");
        OAuth raced = new OAuth(null, concurrentStub, clock, sleeper);
        List<String> winners = Collections.synchronizedList(new ArrayList<>());
        Runnable worker = () -> winners.add(raced.clientCredentialsToken("service").accessToken);
        Thread left = new Thread(worker);
        Thread right = new Thread(worker);
        left.start();
        right.start();
        left.join();
        right.join();
        check(winners.size() == 2 && concurrentStub.requests.size() == 1,
            "concurrent callers share one acquisition: " + winners + " / " + concurrentStub.requests.size());

        // ---- Credential attach path ----
        Stub hookStub = new Stub().put(TOKEN, "{\"access_token\":\"at-hook\",\"expires_in\":3600}");
        OAuth hooked = new OAuth(null, hookStub, clock, sleeper);
        HttpRuntime.CredentialProvider provider = hooked.clientCredentialsProvider("service");
        HttpRuntime.Authorization authorization = provider.provide(new HttpRuntime.CredentialContext(
            "service", "https://source/oauth#/components/securitySchemes/service",
            JsonRuntime.parse("{}"), List.of("read"), true));
        check(authorization != null, "the hook answers with an Authorization attachment");
        check(authorization.toString().equals("Authorization"), "attachment toString is redacted");

        // ---- Explicit refresh: declared refresh URL, rotation and retention ----
        Stub refreshStub = new Stub().put(REFRESH,
            "{\"access_token\":\"rt-at-1\",\"expires_in\":100,\"refresh_token\":\"rt-rotated\"}",
            "{\"access_token\":\"rt-at-2\",\"expires_in\":100}",
            "{\"access_token\":\"rt-at-3\",\"expires_in\":100,\"refresh_token\":\"rt-3\"}");
        OAuth refresher = new OAuth(null, refreshStub, clock, sleeper);
        OAuth.TokenSet refreshed = refresher.refreshToken("service", new OAuth.TokenSet("stale", "Bearer", null, "rt-current", null));
        check(refreshed.accessToken.equals("rt-at-1"), "refreshed access token");
        check("rt-rotated".equals(refreshed.refreshToken), "rotated refresh token adopted");
        check(refreshStub.requests.get(0).uri().toString().equals(REFRESH), "the declared refresh URL serves refreshes");
        Map<String, String> refreshFields = fields(form(refreshStub.requests.get(0)));
        check("refresh_token".equals(refreshFields.get("grant_type")) && "rt-current".equals(refreshFields.get("refresh_token")), "refresh form");
        OAuth.TokenSet keep = refresher.refreshToken("service", new OAuth.TokenSet("stale", "Bearer", null, "rt-keep", null));
        check("rt-keep".equals(keep.refreshToken), "current refresh token retained when none is issued");
        try {
            refresher.refreshToken("service", new OAuth.TokenSet("no-refresh"));
            throw new AssertionError("expected a no-refresh-token failure");
        } catch (OAuth.AuthException error) {
            check(error.kind.equals("no-refresh-token"), "refresh without a token is typed: " + error.kind);
        }

        // Store replacement: refresh replaces the store entry under the partition key.
        OAuth.MemoryTokenStore store = new OAuth.MemoryTokenStore();
        refresher.refreshToken("service", new OAuth.TokenSet("stale", "Bearer", null, "rt-x", null), null, null, store);
        OAuth.TokenSet storedSet = store.load("service|" + REFRESH + "|cli-id");
        check(storedSet != null && storedSet.accessToken.equals("rt-at-3"), "store replacement on refresh");

        // ---- Token endpoint failure mapping: typed, secret-free ----
        Stub failing = new Stub().put(TOKEN,
            "400|{\"error\":\"invalid_client\",\"error_description\":\"server says: cli-secret\"}",
            "500|{\"boom\":true}");
        OAuth failingOauth = new OAuth(null, failing, clock, sleeper);
        try {
            failingOauth.clientCredentialsToken("service");
            throw new AssertionError("expected a typed failure");
        } catch (OAuth.AuthException error) {
            check(error.kind.equals("invalid-client"), "server code maps to a kind: " + error.kind);
            check(error.status != null && error.status == 400, "status metadata");
            check("invalid_client".equals(error.serverError), "server error metadata");
            check(!error.getMessage().contains("cli-secret") && !error.getMessage().contains("server says"), "message carries no server description or secret: " + error.getMessage());
        }
        try {
            failingOauth.clientCredentialsToken("service");
            throw new AssertionError("expected a typed failure");
        } catch (OAuth.AuthException error) {
            check(error.kind.equals("server-error") && error.status == 500, "unmapped server errors stay typed: " + error.kind);
        }

        // ---- Authorization code + PKCE S256 ----
        Stub codeStub = new Stub().put(TOKEN, "{\"access_token\":\"code-at\",\"expires_in\":3600}");
        OAuth coder = new OAuth(null, codeStub, clock, sleeper);
        OAuth.AuthorizationTransaction transaction = coder.beginAuthorization("service", URI.create("https://app.test/callback"), List.of("read"), null);
        check(transaction.authorizationUrl.startsWith(AUTHORIZE + "?"), "redirect target: " + transaction.authorizationUrl);
        Map<String, String> query = new LinkedHashMap<>();
        for (String pair : transaction.authorizationUrl.substring(transaction.authorizationUrl.indexOf('?') + 1).split("&")) {
            String[] parts = pair.split("=", 2);
            query.put(URLDecoder.decode(parts[0], StandardCharsets.UTF_8), URLDecoder.decode(parts[1], StandardCharsets.UTF_8));
        }
        check("code".equals(query.get("response_type")), "response type");
        check("cli-id".equals(query.get("client_id")), "client id from the compiled environment variable");
        check("https://app.test/callback".equals(query.get("redirect_uri")), "redirect uri");
        check("S256".equals(query.get("code_challenge_method")), "S256 challenge method");
        check(query.get("code_challenge").equals(base64Url(MessageDigest.getInstance("SHA-256").digest(transaction.codeVerifier.getBytes(StandardCharsets.US_ASCII)))), "the challenge is the S256 hash of the verifier");
        check(codeStub.requests.isEmpty(), "beginning an authorization makes no request");
        try {
            coder.beginAuthorization("service", URI.create("https://app.test/callback#fragment"));
            throw new AssertionError("expected an invalid-request failure");
        } catch (OAuth.AuthException error) {
            check(error.kind.equals("invalid-request"), "redirect fragments are refused");
        }
        // State mismatch is a typed failure that consumes the transaction.
        try {
            coder.completeAuthorization(transaction, "the-code", "wrong-state");
            throw new AssertionError("expected a state-mismatch failure");
        } catch (OAuth.AuthException error) {
            check(error.kind.equals("state-mismatch"), "state mismatch kind");
        }
        try {
            coder.completeAuthorization(transaction, "the-code", transaction.state);
            throw new AssertionError("expected a transaction-consumed failure");
        } catch (OAuth.AuthException error) {
            check(error.kind.equals("transaction-consumed"), "consumed by any attempt: " + error.kind);
        }
        transaction = coder.beginAuthorization("service", URI.create("https://app.test/callback"), null, null);
        OAuth.TokenSet completed = coder.completeAuthorization(transaction, "the-code", transaction.state, null, "cli-secret", null);
        check(completed.accessToken.equals("code-at"), "code exchanged");
        Map<String, String> exchange = fields(form(codeStub.requests.get(0)));
        check("authorization_code".equals(exchange.get("grant_type")), "code grant");
        check("the-code".equals(exchange.get("code")), "code");
        check("https://app.test/callback".equals(exchange.get("redirect_uri")), "redirect uri");
        check(base64Url(MessageDigest.getInstance("SHA-256").digest(transaction.codeVerifier.getBytes(StandardCharsets.US_ASCII))).equals(transaction.codeChallenge), "the exchanged verifier matches the bound challenge");
        check(codeStub.requests.size() == 1, "one exchange request");

        // ---- Device authorization with polling, slow-down and expiry ----
        Stub deviceStub = new Stub().put(DEVICE,
            "{\"device_code\":\"dev-1\",\"user_code\":\"ABCD-EFGH\",\"verification_uri\":\"https://auth.oauth.test/activate\",\"verification_uri_complete\":\"https://auth.oauth.test/activate?code=ABCD-EFGH\",\"expires_in\":300,\"interval\":1}");
        OAuth deviceOauth = new OAuth(null, deviceStub, clock, sleeper);
        OAuth.DeviceGrant grant = deviceOauth.beginDeviceAuthorization("service");
        check(grant.deviceCode.equals("dev-1") && grant.userCode.equals("ABCD-EFGH"), "grant fields");
        check(grant.expiresAt == now[0] + 300_000L && grant.intervalSeconds == 1, "grant policy fields");
        check(basic(deviceStub.requests.get(0)).equals("cli-id:cli-secret"), "the confidential device request authenticates with Basic");
        check(!grant.toString().contains("dev-1"), "device grant toString is redacted");
        // authorization_pending waits the declared interval, then success.
        Stub pendingStub = new Stub() {
            final int[] pending = {1};
            @Override <T> HttpResponse<T> route(HttpRequest request) {
                requests.add(request);
                if (pending[0] > 0) { pending[0] -= 1; return new Answer<>(request, 400, "{\"error\":\"authorization_pending\"}"); }
                return new Answer<>(request, 200, "{\"access_token\":\"device-at\",\"expires_in\":3600}");
            }
        };
        OAuth pendingOauth = new OAuth(null, pendingStub, clock, sleeper);
        OAuth.MemoryTokenStore deviceStore = new OAuth.MemoryTokenStore();
        OAuth.TokenSet deviceToken = pendingOauth.pollDeviceAuthorization(grant, null, null, deviceStore);
        check(deviceToken.accessToken.equals("device-at"), "device polling resolves a token set");
        OAuth.TokenSet deviceStored = deviceStore.load("service|" + TOKEN + "|cli-id");
        check(deviceStored != null && deviceStored.accessToken.equals("device-at"), "device token stored under the partition key");
        check(sleeps.equals(List.of(1_000L)), "authorization_pending waits the declared interval: " + sleeps);
        // slow_down grows the interval by five seconds.
        sleeps.clear();
        Stub slowStub = new Stub() {
            final int[] calls = {0};
            @Override <T> HttpResponse<T> route(HttpRequest request) {
                requests.add(request);
                if (calls[0]++ == 0) return new Answer<>(request, 400, "{\"error\":\"slow_down\"}");
                return new Answer<>(request, 200, "{\"access_token\":\"device-at-2\",\"expires_in\":3600}");
            }
        };
        OAuth slowOauth = new OAuth(null, slowStub, clock, sleeper);
        check(slowOauth.pollDeviceAuthorization(grant).accessToken.equals("device-at-2"), "slow-down polling resolves");
        check(sleeps.equals(List.of(6_000L)), "slow_down backs off five extra seconds: " + sleeps);
        // Expiry ends polling with a typed failure.
        OAuth.DeviceGrant expired = new OAuth.DeviceGrant("service", "dev-2", "ABCD-EFGH", "https://auth.oauth.test/activate", null, now[0], 1);
        now[0] += 1;
        try {
            slowOauth.pollDeviceAuthorization(expired);
            throw new AssertionError("expected a device-code-expired failure");
        } catch (OAuth.AuthException error) {
            check(error.kind.equals("device-code-expired"), "expiry kind: " + error.kind);
        }
        // A terminal server refusal is typed and stops polling.
        Stub deniedStub = new Stub().put(TOKEN, "400|{\"error\":\"access_denied\"}");
        OAuth deniedOauth = new OAuth(null, deniedStub, clock, sleeper);
        try {
            deniedOauth.pollDeviceAuthorization(grant);
            throw new AssertionError("expected a typed refusal");
        } catch (OAuth.AuthException error) {
            check(error.kind.equals("server-error") && "access_denied".equals(error.serverError), "terminal refusal: " + error.kind);
        }

        // ---- Revocation and introspection over the configured endpoints ----
        Stub supplemental = new Stub()
            .put(REVOKE, "{}")
            .put(INTROSPECT, "{\"active\":true,\"scope\":\"read\"}");
        OAuth supplementalOauth = new OAuth(null, supplemental, clock, sleeper);
        supplementalOauth.revoke("service", "revoke-me", "access_token", null, null);
        Map<String, String> revokeFields = fields(form(supplemental.requests.get(0)));
        check("revoke-me".equals(revokeFields.get("token")) && "access_token".equals(revokeFields.get("token_type_hint")), "revocation form");
        JsonRuntime.JsonObject active = supplementalOauth.introspect("service", "peek-me");
        check(((JsonRuntime.JsonBoolean) active.values().get("active")).value(), "introspection response");
        Map<String, String> introspectFields = fields(form(supplemental.requests.get(1)));
        check("peek-me".equals(introspectFields.get("token")), "introspection form");

        // ---- Store ownership: instances never share a global store ----
        Stub shared = new Stub().put(TOKEN, "{\"access_token\":\"at-shared\",\"expires_in\":3600}");
        OAuth leftOauth = new OAuth(null, shared, clock, sleeper);
        OAuth rightOauth = new OAuth(null, shared, clock, sleeper);
        leftOauth.clientCredentialsToken("service");
        check(rightOauth.clientCredentialsToken("service") != null && shared.requests.size() == 2,
            "two instances acquire independently: " + shared.requests.size());

        // ---- TokenSet never leaks values through toString ----
        OAuth.TokenSet secret = new OAuth.TokenSet("secret-access", "Bearer", null, "secret-refresh", null);
        check(!secret.toString().contains("secret-access") && !secret.toString().contains("secret-refresh"), "token set toString is redacted: " + secret);

        System.out.println("oauth behavior verified");
    }
}
"#;

/// One OpenID Connect scheme (whose endpoints the discovery document defines
/// at runtime) plus one OAuth2 scheme with a compiled token endpoint, each
/// used by an operation.
fn discovery_document() -> Value {
    json!({
        "openapi": "3.1.0",
        "info": {"title": "OAuth discovery", "version": "1"},
        "servers": [{"url": "https://api.oauth.test/v1"}],
        "paths": {
            "/widgets": {"get": {
                "operationId": "listWidgets",
                "security": [{"identity": []}],
                "responses": {"200": {"description": "Ok", "content": {"application/json": {"schema": {"type": "string"}}}}}
            }},
            "/gadgets": {"get": {
                "operationId": "listGadgets",
                "security": [{"service": ["read"]}],
                "responses": {"200": {"description": "Ok", "content": {"application/json": {"schema": {"type": "string"}}}}}
            }}
        },
        "components": {"securitySchemes": {
            "identity": {"type": "openIdConnect", "openIdConnectUrl": "https://authority.oauth.test/.well-known/openid-configuration"},
            "service": {"type": "oauth2", "flows": {"clientCredentials": {
                "tokenUrl": "https://auth.oauth.test/token",
                "scopes": {"read": "Read access"}
            }}}
        }}
    })
}

fn discovery_options() -> GenerationOptions {
    GenerationOptions {
        sdk_defaults: Some(
            serde_json::from_value(json!({
                "version": "v1",
                "oauth": {"schemes": {
                    "identity": {
                        "client_id_env": "JAVA_OAUTH_DISCOVERY_CLIENT_ID",
                        "client_secret_env": "JAVA_OAUTH_DISCOVERY_CLIENT_SECRET"
                    },
                    "service": {
                        "client_id_env": "JAVA_OAUTH_CLIENT_ID",
                        "client_secret_env": "JAVA_OAUTH_CLIENT_SECRET",
                        "revocation_endpoint": "https://auth.oauth.test/revoke",
                        "introspection_endpoint": "https://auth.oauth.test/introspect"
                    }
                }}
            }))
            .unwrap(),
        ),
        ..Default::default()
    }
}

#[test]
fn discovery_schemes_emit_the_discovery_engine_and_per_instance_cache() {
    let discovery_configured = generate(&discovery_document(), &discovery_options());
    let plain = generate(&discovery_document(), &GenerationOptions::default());
    // The discovery policy adds exactly one file and changes no other byte.
    assert_eq!(
        discovery_configured.len(),
        plain.len() + 1,
        "the discovery policy may add exactly one file ({} vs {})",
        discovery_configured.len(),
        plain.len()
    );
    for file in &plain {
        let emitted = discovery_configured
            .iter()
            .find(|candidate| candidate.path == file.path)
            .unwrap_or_else(|| panic!("{} disappeared under the discovery policy", file.path));
        assert_eq!(emitted.content, file.content, "{} changed", file.path);
    }
    let source = oauth_file(&discovery_configured).content.clone();
    for expected in [
        // The compiled ceiling, the per-instance cache and its single-flight
        // gates, keyed by scheme.
        "private static final int DISCOVERY_MAX_BYTES = 1 << 20;",
        "private final Map<String, JsonRuntime.JsonObject> discovered = new ConcurrentHashMap<>();",
        "private final Map<String, Object> discoveryGates = new ConcurrentHashMap<>();",
        // The engine: the typed fetch with the issuer-origin rule, the
        // per-instance single-flight gate and the resolution precedence.
        "private JsonRuntime.JsonObject discover(Scheme compiled) {",
        "the discovery document issuer does not share the discovery URL origin",
        "the compiled plan carries no discovery URL, so endpoint resolution cannot fall back to discovery",
        "private String resolveEndpoint(Scheme compiled, String compiledEndpoint, String member)",
        "compiledFlowOrNull(compiled, \"client-credentials\")",
        "refreshFlowOrNull(compiled)",
        "discoveryBasicAuth(compiled, identity)",
        // The compiled discovery URL lands in the discovery-defined scheme's
        // frozen descriptor, whose endpoint fields stay empty.
        "schemes.put(\"identity\", new Scheme(\"identity\", \"open-id-connect\", 30, \"https://authority.oauth.test/.well-known/openid-configuration\", \"JAVA_OAUTH_DISCOVERY_CLIENT_ID\", \"JAVA_OAUTH_DISCOVERY_CLIENT_SECRET\", null, null, flows())",
        // The compiled scheme keeps its endpoints; discovery only supplements.
        "schemes.put(\"service\", new Scheme(\"service\", \"oauth2\", 30, null, \"JAVA_OAUTH_CLIENT_ID\", \"JAVA_OAUTH_CLIENT_SECRET\", \"https://auth.oauth.test/revoke\", \"https://auth.oauth.test/introspect\", flows(new Flow(",
        // The discovery-resolved form POST carries the client id only in the public profile.
        "if (basic == null && identity[0] != null) sent.put(\"client_id\", identity[0]);",
    ] {
        assert!(
            source.contains(expected),
            "OAuth.java is missing:\n{expected}\n--- emitted: ---\n{source}"
        );
    }

    // Control: a plan whose schemes carry no discovery URL emits no discovery
    // engine and the plain byte-exact descriptor shape.
    let undiscovered = oauth_file(&generate(&service_document(), &configured()))
        .content
        .clone();
    for absent in [
        "discovery-failed",
        "discoveryGates",
        "DISCOVERY_MAX_BYTES",
        "resolveEndpoint",
        "compiledFlowOrNull",
        "discoveryBasicAuth",
    ] {
        assert!(
            !undiscovered.contains(absent),
            "a plan without a discovery URL must not carry {absent}"
        );
    }
    assert!(
        undiscovered
            .contains("schemes.put(\"service\", new Scheme(\"service\", \"oauth2\", 30, \"JAVA_OAUTH_CLIENT_ID\""),
        "the plain descriptor keeps its exact pre-discovery shape: {undiscovered}"
    );
}

/// A scheme whose only usability is its discovery URL — an OpenID Connect
/// declaration with no flows — still emits the lifecycle.
#[test]
fn discovery_only_schemes_emit_the_lifecycle() {
    let mut document = discovery_document();
    document["paths"]
        .as_object_mut()
        .unwrap()
        .remove("/gadgets");
    document["components"]["securitySchemes"]
        .as_object_mut()
        .unwrap()
        .remove("service");
    let options = GenerationOptions {
        sdk_defaults: Some(
            serde_json::from_value(json!({
                "version": "v1",
                "oauth": {"schemes": {"identity": {
                    "client_id_env": "JAVA_OAUTH_DISCOVERY_CLIENT_ID",
                    "client_secret_env": "JAVA_OAUTH_DISCOVERY_CLIENT_SECRET"
                }}}
            }))
            .unwrap(),
        ),
        ..Default::default()
    };
    let configured = generate(&document, &options);
    let plain = generate(&document, &GenerationOptions::default());
    assert!(
        !plain.iter().any(|file| file.path.ends_with("OAuth.java")),
        "the unconfigured OIDC-only package must not emit the lifecycle"
    );
    let source = oauth_file(&configured).content.clone();
    for expected in [
        "schemes.put(\"identity\", new Scheme(\"identity\", \"open-id-connect\", 30, \"https://authority.oauth.test/.well-known/openid-configuration\", \"JAVA_OAUTH_DISCOVERY_CLIENT_ID\", \"JAVA_OAUTH_DISCOVERY_CLIENT_SECRET\", null, null, flows())",
        "public TokenSet clientCredentialsToken(String scheme, String clientId, String clientSecret, String scope)",
        "the discovery document issuer does not share the discovery URL origin",
    ] {
        assert!(
            source.contains(expected),
            "the discovery-only lifecycle is missing:\n{expected}\n--- emitted: ---\n{source}"
        );
    }
}

#[test]
fn discovery_lifecycle_drives_stubbed_transport_in_java() {
    let root = tempfile::tempdir().unwrap();
    let files = generate(&discovery_document(), &discovery_options());
    suspect_codegen::write_files(&files, root.path()).unwrap();
    let classes = compiled_package(root.path());
    fs::write(root.path().join("DiscoveryProbe.java"), DISCOVERY_PROBE).unwrap();
    let home = java_home();
    let output = Command::new(home.join("bin/javac"))
        .args(["--release", "21", "-Xlint:all", "-Werror", "-cp"])
        .arg(&classes)
        .arg("-d")
        .arg(root.path().join("probe"))
        .arg(root.path().join("DiscoveryProbe.java"))
        .current_dir(root.path())
        .output()
        .unwrap();
    fs::write(
        root.path().join("discovery-probe-javac.stderr.log"),
        &output.stderr,
    )
    .unwrap();
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
        .arg("DiscoveryProbe")
        // Compiled environment variable NAMES, real values supplied by the
        // environment at call time.
        .env("JAVA_OAUTH_DISCOVERY_CLIENT_ID", "cli-id")
        .env("JAVA_OAUTH_DISCOVERY_CLIENT_SECRET", "cli-secret")
        .current_dir(root.path())
        .output()
        .unwrap();
    fs::write(
        root.path().join("discovery-probe.stdout.log"),
        &runtime.stdout,
    )
    .unwrap();
    fs::write(
        root.path().join("discovery-probe.stderr.log"),
        &runtime.stderr,
    )
    .unwrap();
    assert!(
        runtime.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&runtime.stdout),
        String::from_utf8_lossy(&runtime.stderr)
    );
}

const DISCOVERY_PROBE: &str = r#"
import java.net.*;
import java.net.http.HttpClient;
import java.net.http.HttpHeaders;
import java.net.http.HttpRequest;
import java.net.http.HttpResponse;
import java.nio.ByteBuffer;
import java.nio.charset.StandardCharsets;
import java.io.ByteArrayOutputStream;
import java.time.Duration;
import java.util.*;
import java.util.concurrent.*;
import java.util.concurrent.Flow;
import javax.net.ssl.*;

import test.suspect.*;

import static test.suspect.JsonRuntime.*;

/** Independent OAuth discovery acceptance over a stubbed JDK transport. */
public class DiscoveryProbe {
    static void check(boolean value, String message) { if (!value) throw new AssertionError(message); }

    static final String DISCOVERY = "https://authority.oauth.test/.well-known/openid-configuration";
    static final String DISCOVERED_TOKEN = "https://authority.oauth.test/oauth/token";
    static final String DISCOVERED_REVOKE = "https://authority.oauth.test/oauth/revoke";
    static final String DISCOVERED_INTROSPECT = "https://authority.oauth.test/oauth/introspect";
    static final String COMPILED_TOKEN = "https://auth.oauth.test/token";

    static HttpHeaders jsonHeaders() {
        return HttpHeaders.of(Map.of("Content-Type", List.of("application/json")), (key, value) -> true);
    }

    /** A bytestring response carrying a status and JSON content type. */
    static final class Answer<T> implements HttpResponse<T> {
        private final HttpRequest request;
        private final int status;
        private final HttpHeaders headers;
        private final byte[] body;

        Answer(HttpRequest request, int status, String body) {
            this.request = request;
            this.status = status;
            this.headers = jsonHeaders();
            this.body = body.getBytes(StandardCharsets.UTF_8);
        }

        @SuppressWarnings("unchecked")
        @Override public T body() { return (T) body; }
        @Override public int statusCode() { return status; }
        @Override public HttpRequest request() { return request; }
        @Override public Optional<HttpResponse<T>> previousResponse() { return Optional.empty(); }
        @Override public HttpHeaders headers() { return headers; }
        @Override public Optional<javax.net.ssl.SSLSession> sslSession() { return Optional.empty(); }
        @Override public URI uri() { return request.uri(); }
        @Override public HttpClient.Version version() { return HttpClient.Version.HTTP_1_1; }
    }

    /** Routes requests by URL; discovery answers are stateful, token bodies answer in order. */
    static class Stub extends HttpClient {
        final List<HttpRequest> requests = Collections.synchronizedList(new ArrayList<>());
        final Map<String, List<String>> responses = new ConcurrentHashMap<>();
        final Map<String, Integer> counts = new ConcurrentHashMap<>();
        String issuer = "https://authority.oauth.test";
        int failStatus = 0;

        Stub put(String url, String... bodies) { responses.put(url, List.of(bodies)); return this; }

        int hits(String url) { return counts.getOrDefault(url, 0); }

        @Override public Optional<CookieHandler> cookieHandler() { return Optional.empty(); }
        @Override public Optional<Duration> connectTimeout() { return Optional.empty(); }
        @Override public Redirect followRedirects() { return Redirect.NEVER; }
        @Override public Optional<ProxySelector> proxy() { return Optional.empty(); }
        @Override public SSLContext sslContext() { try { return SSLContext.getDefault(); } catch (Exception error) { throw new AssertionError(error); } }
        @Override public SSLParameters sslParameters() { return new SSLParameters(); }
        @Override public Optional<Authenticator> authenticator() { return Optional.empty(); }
        @Override public Version version() { return Version.HTTP_1_1; }
        @Override public Optional<Executor> executor() { return Optional.empty(); }
        @Override public <T> HttpResponse<T> send(HttpRequest request, HttpResponse.BodyHandler<T> handler) {
            return route(request);
        }
        @Override public <T> CompletableFuture<HttpResponse<T>> sendAsync(HttpRequest request, HttpResponse.BodyHandler<T> handler, HttpResponse.PushPromiseHandler<T> push) { return sendAsync(request, handler); }
        @Override public <T> CompletableFuture<HttpResponse<T>> sendAsync(HttpRequest request, HttpResponse.BodyHandler<T> handler) {
            return CompletableFuture.completedFuture(route(request));
        }

        @SuppressWarnings("unchecked")
        <T> HttpResponse<T> route(HttpRequest request) {
            requests.add(request);
            String url = request.uri().toString();
            if (url.equals(DISCOVERY)) {
                counts.merge(DISCOVERY, 1, Integer::sum);
                if (failStatus != 0) return (HttpResponse<T>) new Answer<>(request, failStatus, "{\"boom\":true}");
                String document = "{\"issuer\":" + (issuer == null ? "null" : "\"" + issuer + "\"")
                    + ",\"token_endpoint\":\"" + DISCOVERED_TOKEN + "\""
                    + ",\"revocation_endpoint\":\"" + DISCOVERED_REVOKE + "\""
                    + ",\"introspection_endpoint\":\"" + DISCOVERED_INTROSPECT + "\"}";
                return (HttpResponse<T>) new Answer<>(request, 200, document);
            }
            List<String> bodies = responses.get(url);
            if (bodies == null) throw new AssertionError("unexpected request: " + url);
            int index = counts.merge(url, 1, Integer::sum) - 1;
            return (HttpResponse<T>) new Answer<>(request, 200, bodies.get(Math.min(index, bodies.size() - 1)));
        }
    }

    static String form(HttpRequest request) {
        if (request.bodyPublisher().isEmpty()) return "";
        ByteArrayOutputStream out = new ByteArrayOutputStream();
        request.bodyPublisher().get().subscribe(new Flow.Subscriber<>() {
            @Override public void onSubscribe(Flow.Subscription subscription) { subscription.request(Long.MAX_VALUE); }
            @Override public void onNext(ByteBuffer item) { byte[] bytes = new byte[item.remaining()]; item.get(bytes); out.writeBytes(bytes); }
            @Override public void onError(Throwable error) { throw new RuntimeException(error); }
            @Override public void onComplete() {}
        });
        return new String(out.toByteArray(), StandardCharsets.UTF_8);
    }

    static Map<String, String> fields(String form) {
        Map<String, String> values = new LinkedHashMap<>();
        for (String pair : form.split("&")) {
            if (pair.isEmpty()) continue;
            int equals = pair.indexOf('=');
            values.put(URLDecoder.decode(pair.substring(0, equals), StandardCharsets.UTF_8),
                URLDecoder.decode(pair.substring(equals + 1), StandardCharsets.UTF_8));
        }
        return values;
    }

    static String basic(HttpRequest request) {
        String value = request.headers().firstValue("authorization").orElse("");
        check(value.startsWith("Basic "), "expected Basic authentication: " + value);
        return new String(Base64.getDecoder().decode(value.substring(6)), StandardCharsets.UTF_8);
    }

    /** The GET discovery request: the injected transport, JSON acceptance. */
    static void checkDiscoveryRequest(HttpRequest request) {
        check(request.method().equals("GET"), "the discovery request is a GET: " + request.method());
        check(request.headers().firstValue("accept").orElse("").equals("application/json"),
            "the discovery request accepts JSON");
        check(request.uri().toString().equals(DISCOVERY), "the discovery URL: " + request.uri());
    }

    public static void main(String[] args) throws Exception {
        // ---- 1. One-shot acquisition resolves the discovered token endpoint ----
        Stub stub = new Stub().put(DISCOVERED_TOKEN,
            "{\"access_token\":\"discovered-1\",\"token_type\":\"Bearer\",\"expires_in\":3600,\"refresh_token\":\"rotated-1\"}",
            "{\"access_token\":\"discovered-2\",\"token_type\":\"Bearer\",\"expires_in\":3600}");
        OAuth oauth = new OAuth(null, stub, () -> 1_000_000L, ignore -> {});
        OAuth.TokenSet set = oauth.clientCredentialsToken("identity");
        check(set.accessToken.equals("discovered-1"), "acquired through discovery: " + set.accessToken);
        check(stub.hits(DISCOVERY) == 1, "one discovery fetch: " + stub.hits(DISCOVERY));
        checkDiscoveryRequest(stub.requests.get(0));
        check(stub.hits(DISCOVERED_TOKEN) == 1, "one token acquisition at the discovered endpoint: " + stub.hits(DISCOVERED_TOKEN));
        check(basic(stub.requests.get(1)).equals("cli-id:cli-secret"),
            "the discovery-defined client authenticates with Basic from the compiled environment variables");
        Map<String, String> tokenForm = fields(form(stub.requests.get(1)));
        check("client_credentials".equals(tokenForm.get("grant_type")), "grant type");
        check(!tokenForm.containsKey("client_id"), "confidential discovery clients never send a body client id");
        check(stub.requests.stream().noneMatch(request -> request.uri().toString().equals(COMPILED_TOKEN)),
            "the compiled fallback endpoint of the other scheme is never contacted here");

        // ---- 2. The per-instance cache: the second call refetches nothing ----
        check(oauth.clientCredentialsToken("identity").accessToken.equals("discovered-1"), "cached token served");
        check(stub.hits(DISCOVERY) == 1 && stub.hits(DISCOVERED_TOKEN) == 1, "the cached document and token served both calls");

        // ---- 3. Concurrent callers share one discovery fetch and one acquisition ----
        Stub race = new Stub().put(DISCOVERED_TOKEN, "{\"access_token\":\"race-1\",\"expires_in\":3600}");
        OAuth raced = new OAuth(null, race, () -> 1_000_000L, ignore -> {});
        List<String> winners = Collections.synchronizedList(new ArrayList<>());
        Runnable worker = () -> winners.add(raced.clientCredentialsToken("identity").accessToken);
        Thread left = new Thread(worker);
        Thread right = new Thread(worker);
        left.start();
        right.start();
        left.join();
        right.join();
        check(winners.size() == 2, "both callers resolved a token");
        check(race.hits(DISCOVERY) == 1, "concurrent callers share one discovery fetch: " + race.hits(DISCOVERY));
        check(race.hits(DISCOVERED_TOKEN) == 1, "concurrent callers share one acquisition: " + race.hits(DISCOVERED_TOKEN));

        // ---- 4. An issuer from another origin is a typed failure carrying no body text ----
        Stub mismatch = new Stub();
        OAuth misOauth = new OAuth(null, mismatch, () -> 1_000_000L, ignore -> {});
        mismatch.issuer = "https://elsewhere.oauth.test";
        try {
            misOauth.clientCredentialsToken("identity");
            throw new AssertionError("an issuer from another origin must fail");
        } catch (OAuth.AuthException error) {
            check(error.kind.equals("discovery-failed"), "issuer mismatch kind: " + error.kind);
            check(!error.getMessage().contains("elsewhere") && !error.getMessage().contains("boom"),
                "the typed error carries no body text: " + error.getMessage());
        }
        check(mismatch.requests.size() == 1, "the failed fetch made exactly one request: " + mismatch.requests.size());
        // A failed fetch is never cached: the next call retries and recovers.
        mismatch.issuer = "https://authority.oauth.test";
        mismatch.put(DISCOVERED_TOKEN, "{\"access_token\":\"after-mismatch\",\"expires_in\":3600}");
        check(misOauth.clientCredentialsToken("identity").accessToken.equals("after-mismatch"), "the next call retries discovery");
        check(mismatch.hits(DISCOVERY) == 2, "the failed fetch was retried: " + mismatch.hits(DISCOVERY));
        check(misOauth.clientCredentialsToken("identity").accessToken.equals("after-mismatch"), "the recovered document is cached");
        check(mismatch.hits(DISCOVERY) == 2, "the recovered fetch is cached: " + mismatch.hits(DISCOVERY));

        // ---- 5. A failing discovery request is typed with its status, and retried next call ----
        Stub failing = new Stub();
        OAuth failOauth = new OAuth(null, failing, () -> 1_000_000L, ignore -> {});
        failing.failStatus = 500;
        try {
            failOauth.clientCredentialsToken("identity");
            throw new AssertionError("a failing discovery request must fail");
        } catch (OAuth.AuthException error) {
            check(error.kind.equals("discovery-failed") && error.status != null && error.status == 500,
                "the failed request is typed with its status: " + error.kind + " " + error.status);
        }
        failing.failStatus = 0;
        failing.put(DISCOVERED_TOKEN, "{\"access_token\":\"repaired\",\"expires_in\":3600}");
        check(failOauth.clientCredentialsToken("identity").accessToken.equals("repaired"), "the repaired fetch recovers");
        check(failing.hits(DISCOVERY) == 2, "the failed fetch was retried: " + failing.hits(DISCOVERY));

        // ---- 6. Compiled precedence: the compiled token URL wins and never fetches discovery ----
        Stub compiled = new Stub().put(COMPILED_TOKEN, "{\"access_token\":\"compiled-1\",\"expires_in\":3600}");
        OAuth compiledOauth = new OAuth(null, compiled, () -> 1_000_000L, ignore -> {});
        check(compiledOauth.clientCredentialsToken("service", "explicit-id", "explicit-secret", null).accessToken.equals("compiled-1"),
            "the compiled token endpoint serves the compiled scheme");
        check(compiled.hits(COMPILED_TOKEN) == 1 && compiled.requests.stream().noneMatch(request -> request.uri().toString().equals(DISCOVERY)),
            "a scheme with a compiled token URL never fetches discovery");

        // ---- 7. Refresh and revocation resolve through the discovery document ----
        Stub supplemental = new Stub()
            .put(DISCOVERED_TOKEN, "{\"access_token\":\"seed-1\",\"expires_in\":3600,\"refresh_token\":\"rt-1\"}",
                "{\"access_token\":\"refreshed-1\",\"expires_in\":3600}")
            .put(DISCOVERED_REVOKE, "{}")
            .put(DISCOVERED_INTROSPECT, "{\"active\":true,\"scope\":\"read\"}");
        OAuth supplementalOauth = new OAuth(null, supplemental, () -> 1_000_000L, ignore -> {});
        OAuth.MemoryTokenStore store = new OAuth.MemoryTokenStore();
        OAuth.TokenSet seeded = supplementalOauth.clientCredentialsToken("identity", null, null, null);
        check(seeded.refreshToken != null, "the seeded set carries a refresh token");
        OAuth.TokenSet refreshed = supplementalOauth.refreshToken("identity", seeded, null, null, store);
        check(refreshed.accessToken.equals("refreshed-1"), "refresh resolved the discovered token endpoint: " + refreshed.accessToken);
        Map<String, String> refreshForm = fields(form(supplemental.requests.get(supplemental.requests.size() - 1)));
        check("refresh_token".equals(refreshForm.get("grant_type")), "refresh grant");
        check(store.load("identity|" + DISCOVERED_TOKEN + "|cli-id") != null, "the refreshed set is stored under the discovered endpoint key");
        supplementalOauth.revoke("identity", "revoke-me");
        HttpRequest revokeRequest = supplemental.requests.get(supplemental.requests.size() - 1);
        check(revokeRequest.uri().toString().equals(DISCOVERED_REVOKE), "revocation resolved the discovered endpoint: " + revokeRequest.uri());
        check(fields(form(revokeRequest)).get("token").equals("revoke-me"), "revocation form");
        check(basic(revokeRequest).equals("cli-id:cli-secret"), "discovery revocation authenticates with Basic");
        JsonRuntime.JsonObject active = supplementalOauth.introspect("identity", "peek-me");
        check(((JsonRuntime.JsonBoolean) active.values().get("active")).value(), "introspection resolved the discovered endpoint");

        System.out.println("discovery behavior verified");
    }
}
"#;

/// One client-credentials scheme over a JSON operation and one over a
/// streaming operation, each with its own token endpoint.
fn replay_document() -> Value {
    json!({
        "openapi": "3.2.0",
        "info": {"title": "OAuth replay", "version": "1"},
        "servers": [{"url": "https://api.oauth.test/v1"}],
        "paths": {
            "/widgets": {"get": {
                "operationId": "listWidgets",
                "security": [{"serviceOAuth": ["read"]}],
                "responses": {"200": {"description": "Ok"}}
            }},
            "/events": {"get": {
                "operationId": "streamEvents",
                "security": [{"feedOAuth": ["read"]}],
                "responses": {"200": {"description": "Events", "content": {"text/event-stream": {
                    "itemSchema": {"type": "object", "properties": {"data": {"type": "string"}}}
                }}}}
            }}
        },
        "components": {"securitySchemes": {
            "serviceOAuth": {"type": "oauth2", "flows": {"clientCredentials": {
                "tokenUrl": "https://auth.oauth.test/token",
                "scopes": {"read": "Read access"}
            }}},
            "feedOAuth": {"type": "oauth2", "flows": {"clientCredentials": {
                "tokenUrl": "https://auth.oauth.test/feed-token",
                "scopes": {"read": "Read access"}
            }}}
        }}
    })
}

fn replay_options() -> GenerationOptions {
    GenerationOptions {
        sdk_defaults: Some(
            serde_json::from_value(json!({
                "version": "v1",
                "oauth": {"schemes": {
                    "serviceOAuth": {
                        "client_id_env": "JAVA_OAUTH_REPLAY_CLIENT_ID",
                        "client_secret_env": "JAVA_OAUTH_REPLAY_CLIENT_SECRET"
                    },
                    "feedOAuth": {
                        "client_id_env": "JAVA_OAUTH_REPLAY_FEED_ID",
                        "client_secret_env": "JAVA_OAUTH_REPLAY_FEED_SECRET"
                    }
                }}
            }))
            .unwrap(),
        ),
        ..Default::default()
    }
}

/// An authorization-code-only scheme (no client-credentials flow): the
/// control for the replay emission gate.
fn code_only_document() -> Value {
    json!({
        "openapi": "3.2.0",
        "info": {"title": "OAuth code only", "version": "1"},
        "servers": [{"url": "https://api.oauth.test/v1"}],
        "security": [{"userOAuth": ["read"]}],
        "paths": {"/widgets": {"get": {"operationId": "listWidgets", "responses": {"200": {"description": "Ok"}}}}},
        "components": {"securitySchemes": {"userOAuth": {"type": "oauth2", "flows": {"authorizationCode": {
            "authorizationUrl": "https://auth.oauth.test/authorize",
            "tokenUrl": "https://auth.oauth.test/token",
            "scopes": {"read": "Read access"}
        }}}}}
    })
}

fn code_only_options() -> GenerationOptions {
    GenerationOptions {
        sdk_defaults: Some(
            serde_json::from_value(json!({
                "version": "v1",
                "oauth": {"schemes": {"userOAuth": {
                    "client_id_env": "JAVA_OAUTH_CODE_ONLY_ID",
                    "client_secret_env": "JAVA_OAUTH_CODE_ONLY_SECRET"
                }}}
            }))
            .unwrap(),
        ),
        ..Default::default()
    }
}

/// The replaying credential wrapper is emitted only with an executable
/// client-credentials flow, wraps exactly that provider, and compiles the
/// stream-protection addresses of its scheme's operations.
#[test]
fn replaying_credentials_emit_conditionally_with_stream_protection() {
    let configured = generate(&replay_document(), &replay_options());
    let source = oauth_file(&configured).content.clone();
    for expected in [
        "public ReplayingCredentials replayingCredentials(String scheme)",
        "public static final class ReplayingCredentials",
        "public static final class ReplayHttpClient extends HttpClient",
        "private static final Map<String, Set<String>> REPLAY_NO_REPLAY",
        "#/paths/~1events/get/security/0/feedOAuth\"",
        "one coordinated refresh",
        "stream-protected",
        "one refresh plus one replay",
    ] {
        assert!(
            source.contains(expected),
            "OAuth.java is missing:\n{expected}\n--- emitted: ---\n{source}"
        );
    }
    // The plain JSON operation's requirement is absent: its attaches replay.
    assert!(!source.contains("~1widgets/get/security/0/serviceOAuth"));

    // A package without any executable client-credentials flow compiles
    // exactly the pre-replay bytes: no wrapper, no stream-protection table,
    // and no other artifact changes a byte.
    let code_only = generate(&code_only_document(), &code_only_options());
    let plain = generate(&code_only_document(), &GenerationOptions::default());
    assert_eq!(
        code_only.len(),
        plain.len() + 1,
        "the code-only policy may add exactly one file"
    );
    for file in &plain {
        let emitted = code_only
            .iter()
            .find(|candidate| candidate.path == file.path)
            .unwrap_or_else(|| panic!("{} disappeared under the policy", file.path));
        assert_eq!(emitted.content, file.content, "{} changed", file.path);
    }
    let plain_oauth = oauth_file(&code_only).content.clone();
    for absent in [
        "ReplayingCredentials",
        "ReplayHttpClient",
        "REPLAY_NO_REPLAY",
    ] {
        assert!(
            !plain_oauth.contains(absent),
            "an authorization-code-only plan must not carry {absent}"
        );
    }
}

#[test]
fn emitted_replay_oauth_compiles_strictly_with_the_package() {
    let root = tempfile::tempdir().unwrap();
    suspect_codegen::write_files(
        &generate(&replay_document(), &replay_options()),
        root.path(),
    )
    .unwrap();
    compiled_package(root.path());
}

/// The replay lifecycle (a)–(f) over a stubbed JDK transport, mirroring the
/// TypeScript, Python, Go and Rust acceptance scenarios.
#[test]
fn replay_lifecycle_drives_stubbed_httpclient_in_java() {
    let root = tempfile::tempdir().unwrap();
    let files = generate(&replay_document(), &replay_options());
    suspect_codegen::write_files(&files, root.path()).unwrap();
    let classes = compiled_package(root.path());
    fs::write(root.path().join("ReplayProbe.java"), REPLAY_PROBE).unwrap();
    let home = java_home();
    let output = Command::new(home.join("bin/javac"))
        .args(["--release", "21", "-Xlint:all", "-Werror", "-cp"])
        .arg(&classes)
        .arg("-d")
        .arg(root.path().join("probe"))
        .arg(root.path().join("ReplayProbe.java"))
        .current_dir(root.path())
        .output()
        .unwrap();
    fs::write(
        root.path().join("replay-probe-javac.stderr.log"),
        &output.stderr,
    )
    .unwrap();
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
        .arg("ReplayProbe")
        // The plain provider scenario resolves the compiled environment
        // variables; the replaying scenarios pass explicit identity.
        .env("JAVA_OAUTH_REPLAY_CLIENT_ID", "cid")
        .env("JAVA_OAUTH_REPLAY_CLIENT_SECRET", "csecret")
        .env("JAVA_OAUTH_REPLAY_FEED_ID", "fid")
        .env("JAVA_OAUTH_REPLAY_FEED_SECRET", "fsecret")
        .current_dir(root.path())
        .output()
        .unwrap();
    fs::write(root.path().join("replay-probe.stdout.log"), &runtime.stdout).unwrap();
    fs::write(root.path().join("replay-probe.stderr.log"), &runtime.stderr).unwrap();
    assert!(
        runtime.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&runtime.stdout),
        String::from_utf8_lossy(&runtime.stderr)
    );
}

const REPLAY_PROBE: &str = r#"
import java.net.Authenticator;
import java.net.CookieHandler;
import java.net.ProxySelector;
import java.net.URI;
import java.net.http.HttpClient;
import java.net.http.HttpHeaders;
import java.net.http.HttpRequest;
import java.net.http.HttpResponse;
import java.nio.ByteBuffer;
import java.nio.charset.StandardCharsets;
import java.time.Duration;
import java.util.*;
import java.util.concurrent.*;
import javax.net.ssl.*;

import test.suspect.*;
import test.suspect.Client.*;

/** Independent replay lifecycle acceptance over a stubbed JDK transport. */
public class ReplayProbe {
    static void check(boolean value, String message) { if (!value) throw new AssertionError(message); }

    static final String WIDGETS = "https://api.oauth.test/v1/widgets";
    static final String EVENTS = "https://api.oauth.test/v1/events";
    static final String TOKEN = "https://auth.oauth.test/token";
    static final String FEED_TOKEN = "https://auth.oauth.test/feed-token";

    static HttpHeaders jsonHeaders() {
        return HttpHeaders.of(Map.of("Content-Type", List.of("application/json")), (key, value) -> true);
    }

    /** A response carrying the status, headers and the handler's decoded body. */
    static final class Answer<T> implements HttpResponse<T> {
        private final HttpRequest request;
        private final int status;
        private final HttpHeaders headers;
        private final T body;

        Answer(HttpRequest request, int status, HttpHeaders headers, T body) {
            this.request = request;
            this.status = status;
            this.headers = headers;
            this.body = body;
        }

        @Override public T body() { return body; }
        @Override public int statusCode() { return status; }
        @Override public HttpRequest request() { return request; }
        @Override public Optional<HttpResponse<T>> previousResponse() { return Optional.empty(); }
        @Override public HttpHeaders headers() { return headers; }
        @Override public Optional<javax.net.ssl.SSLSession> sslSession() { return Optional.empty(); }
        @Override public URI uri() { return request.uri(); }
        @Override public HttpClient.Version version() { return HttpClient.Version.HTTP_1_1; }
    }

    /** Fake API and token endpoints. armStale() makes the next issued token answer 401 on the API, so the driver counts exactly one refresh. */
    static class ReplayStub extends HttpClient {
        final List<HttpRequest> requests = Collections.synchronizedList(new ArrayList<>());
        volatile String mode = "ok";
        volatile boolean staleNext = false;
        volatile String staleToken = null;
        volatile int failFrom = Integer.MAX_VALUE;
        int serviceTokens = 0;
        int feedTokens = 0;

        synchronized void armStale() { staleNext = true; mode = "ok"; }

        synchronized void always401() { mode = "always-401"; }

        synchronized int hits(String url) {
            int count = 0;
            for (HttpRequest request : requests) { if (request.uri().toString().equals(url)) count += 1; }
            return count;
        }

        synchronized List<String> tokensFor(String url) {
            List<String> values = new ArrayList<>();
            for (HttpRequest request : requests) {
                if (request.uri().toString().equals(url)) values.add(request.headers().firstValue("authorization").orElse(""));
            }
            return values;
        }

        @Override public Optional<CookieHandler> cookieHandler() { return Optional.empty(); }
        @Override public Optional<Duration> connectTimeout() { return Optional.empty(); }
        @Override public Redirect followRedirects() { return Redirect.NEVER; }
        @Override public Optional<ProxySelector> proxy() { return Optional.empty(); }
        @Override public SSLContext sslContext() { try { return SSLContext.getDefault(); } catch (Exception error) { throw new AssertionError(error); } }
        @Override public SSLParameters sslParameters() { return new SSLParameters(); }
        @Override public Optional<Authenticator> authenticator() { return Optional.empty(); }
        @Override public Version version() { return Version.HTTP_1_1; }
        @Override public Optional<Executor> executor() { return Optional.empty(); }
        @Override public <T> HttpResponse<T> send(HttpRequest request, HttpResponse.BodyHandler<T> handler) throws java.io.IOException, InterruptedException {
            try { return sendAsync(request, handler).get(); }
            catch (ExecutionException error) { throw new java.io.IOException("stub transport failure", error.getCause()); }
        }
        @Override public <T> CompletableFuture<HttpResponse<T>> sendAsync(HttpRequest request, HttpResponse.BodyHandler<T> handler, HttpResponse.PushPromiseHandler<T> push) { return sendAsync(request, handler); }
        @Override public <T> CompletableFuture<HttpResponse<T>> sendAsync(HttpRequest request, HttpResponse.BodyHandler<T> handler) {
            Decision decision = decide(request);
            CompletableFuture<HttpResponse<T>> result = new CompletableFuture<>();
            HttpHeaders headers = jsonHeaders();
            HttpResponse.BodySubscriber<T> subscriber = handler.apply(new HttpResponse.ResponseInfo() {
                @Override public int statusCode() { return decision.status; }
                @Override public HttpHeaders headers() { return headers; }
                @Override public Version version() { return Version.HTTP_1_1; }
            });
            subscriber.getBody().whenComplete((body, error) -> {
                if (error != null) result.completeExceptionally(error);
                else result.complete(new Answer<>(request, decision.status, headers, body));
            });
            subscriber.onSubscribe(new Flow.Subscription() {
                @Override public void request(long count) {
                    byte[] bytes = decision.body.getBytes(StandardCharsets.UTF_8);
                    if (bytes.length > 0) subscriber.onNext(List.of(ByteBuffer.wrap(bytes)));
                    subscriber.onComplete();
                }
                @Override public void cancel() {}
            });
            return result;
        }

        /** One scripted answer: the status and body the endpoint returns. */
        private record Decision(int status, String body) {}

        private synchronized Decision decide(HttpRequest request) {
            requests.add(request);
            String url = request.uri().toString();
            String presented = request.headers().firstValue("authorization").orElse(null);
            if (url.equals(TOKEN)) {
                serviceTokens += 1;
                String token = "svc-" + serviceTokens;
                if (staleNext) { staleNext = false; staleToken = token; }
                if (serviceTokens >= failFrom) return new Decision(500, "{\"error\":\"server_error\"}");
                return new Decision(200, "{\"access_token\":\"" + token + "\",\"token_type\":\"Bearer\",\"expires_in\":3600}");
            }
            if (url.equals(FEED_TOKEN)) {
                feedTokens += 1;
                return new Decision(200, "{\"access_token\":\"feed-" + feedTokens + "\",\"token_type\":\"Bearer\",\"expires_in\":3600}");
            }
            if (url.equals(WIDGETS)) {
                if ("always-401".equals(mode) || (staleToken != null && presented != null && presented.equals("Bearer " + staleToken))) {
                    return new Decision(401, "{\"error\":\"stale\"}");
                }
                return new Decision(200, "");
            }
            if (url.equals(EVENTS)) return new Decision(401, "");
            throw new AssertionError("unexpected request: " + url);
        }
    }

    public static void main(String[] args) throws Exception {
        // ---- (a) 401 then success: one refresh, one replay, and the caller sees 200 ----
        ReplayStub stub = new ReplayStub();
        stub.armStale();
        OAuth oauth = new OAuth(null, stub, () -> 1_000_000L, ignore -> {});
        check(client(oauth, "serviceOAuth", stub).listWidgets(ListWidgetsInput.builder().build()).status() == 200,
            "(a) the replayed call resolves 200");
        check(stub.hits(WIDGETS) == 2, "(a) one refresh, one replay: " + stub.hits(WIDGETS));
        check(stub.hits(TOKEN) == 2, "(a) exactly one forced acquisition: " + stub.hits(TOKEN));
        List<String> sent = stub.tokensFor(WIDGETS);
        check(sent.get(0).equals("Bearer svc-1"), "(a) the original attach carried the stale token: " + sent.get(0));
        check(sent.get(1).equals("Bearer svc-2"), "(a) the replay carried the fresh token: " + sent.get(1));

        // ---- (b) 401 then 401: the second 401 surfaces and exactly one refresh ran ----
        ReplayStub denied = new ReplayStub();
        denied.always401();
        OAuth deniedOauth = new OAuth(null, denied, () -> 1_000_000L, ignore -> {});
        try {
            client(deniedOauth, "serviceOAuth", denied).listWidgets(ListWidgetsInput.builder().build());
            throw new AssertionError("(b) expected the second 401 to surface");
        } catch (SdkException error) {
            check(error.status() == 401, "(b) the second 401 surfaces as the declared error: " + error);
        }
        check(denied.hits(WIDGETS) == 2, "(b) one replay, no loops: " + denied.hits(WIDGETS));
        check(denied.hits(TOKEN) == 2, "(b) exactly one refresh: " + denied.hits(TOKEN));

        // ---- (c) concurrent 401s across two threads: ONE refresh, two replays ----
        ReplayStub shared = new ReplayStub();
        shared.armStale();
        OAuth sharedOauth = new OAuth(null, shared, () -> 1_000_000L, ignore -> {});
        OAuth.ReplayingCredentials replay = sharedOauth.replayingCredentials("serviceOAuth", "cid", "csecret", null);
        replay.provider().provide(new HttpRuntime.CredentialContext(
            "serviceOAuth", "https://source.oauth.test/java-oauth-replay.json#/paths/~1widgets/get/security/0/serviceOAuth",
            JsonRuntime.parse("{}"), List.of("read"), true));
        String stale = "Bearer " + shared.staleToken;
        HttpClient wrapper = replay.transport(shared);
        ExecutorService pool = Executors.newFixedThreadPool(2);
        List<Future<Integer>> races = new ArrayList<>();
        for (int at = 0; at < 2; at++) {
            HttpRequest request = HttpRequest.newBuilder(URI.create(WIDGETS))
                .header("authorization", stale).GET().build();
            races.add(pool.submit(() -> wrapper.sendAsync(request, HttpResponse.BodyHandlers.ofByteArray()).get().statusCode()));
        }
        for (Future<Integer> race : races) check(race.get() == 200, "(c) both replays resolve 200");
        check(shared.hits(WIDGETS) == 4, "(c) two originals and two replays: " + shared.hits(WIDGETS));
        check(shared.hits(TOKEN) == 2, "(c) one shared refresh: " + shared.hits(TOKEN));
        List<String> replays = shared.tokensFor(WIDGETS);
        check(replays.stream().filter(stale::equals).count() == 2
                && replays.stream().filter(value -> !value.equals(stale)).distinct().count() == 1,
            "(c) both replays carried the one fresh token: " + replays);
        pool.shutdown();

        // ---- (d) a stream-protected operation surfaces the typed 401 without any replay or refresh ----
        ReplayStub streamStub = new ReplayStub();
        OAuth streamOauth = new OAuth(null, streamStub, () -> 1_000_000L, ignore -> {});
        try {
            client(streamOauth, "feedOAuth", streamStub).streamEvents(StreamEventsInput.builder().build());
            throw new AssertionError("(d) expected the stream 401 to surface");
        } catch (SdkException error) {
            check(error.status() == 401, "(d) the stream 401 surfaces typed: " + error);
        }
        check(streamStub.hits(EVENTS) == 1, "(d) no replay for the streaming operation: " + streamStub.hits(EVENTS));
        check(streamStub.hits(FEED_TOKEN) == 1, "(d) no refresh for the streaming operation: " + streamStub.hits(FEED_TOKEN));

        // ---- (e) replay disabled by default: the plain provider surfaces the 401 without any refresh ----
        ReplayStub plainStub = new ReplayStub();
        plainStub.armStale();
        OAuth plainOauth = new OAuth(null, plainStub, () -> 1_000_000L, ignore -> {});
        Client plainClient = new Client(HttpRuntime.Options.builder()
            .authorization("serviceOAuth", plainOauth.clientCredentialsProvider("serviceOAuth"))
            .httpClient(plainStub).build());
        try {
            plainClient.listWidgets(ListWidgetsInput.builder().build());
            throw new AssertionError("(e) expected the 401 to surface");
        } catch (SdkException error) {
            check(error.status() == 401, "(e) the plain provider surfaces the 401: " + error);
        }
        check(plainStub.hits(WIDGETS) == 1, "(e) no replay: " + plainStub.hits(WIDGETS));
        check(plainStub.hits(TOKEN) == 1, "(e) no refresh: " + plainStub.hits(TOKEN));

        // ---- (f) refresh failure: the typed auth failure surfaces instead of a replay ----
        ReplayStub failing = new ReplayStub();
        failing.armStale();
        failing.failFrom = 2;
        OAuth failingOauth = new OAuth(null, failing, () -> 1_000_000L, ignore -> {});
        OAuth.ReplayingCredentials replaying = failingOauth.replayingCredentials("serviceOAuth", "cid", "csecret", null);
        replaying.provider().provide(new HttpRuntime.CredentialContext(
            "serviceOAuth", "https://source.oauth.test/java-oauth-replay.json#/paths/~1widgets/get/security/0/serviceOAuth",
            JsonRuntime.parse("{}"), List.of("read"), true));
        HttpRequest request = HttpRequest.newBuilder(URI.create(WIDGETS))
            .header("authorization", "Bearer " + failing.staleToken).GET().build();
        try {
            replaying.transport(failing).sendAsync(request, HttpResponse.BodyHandlers.ofByteArray()).join();
            throw new AssertionError("(f) expected the refresh failure to surface");
        } catch (CompletionException error) {
            check(error.getCause() instanceof OAuth.AuthException auth && auth.status != null && auth.status == 500,
                "(f) the refresh failure surfaces typed: " + error.getCause());
        }
        check(failing.hits(WIDGETS) == 1, "(f) no replay after a failed refresh: " + failing.hits(WIDGETS));
        check(failing.hits(TOKEN) == 2, "(f) the refresh was attempted exactly once: " + failing.hits(TOKEN));

        System.out.println("replay behavior verified");
    }

    /** One wired replaying client over a stub. */
    static Client client(OAuth oauth, String scheme, ReplayStub stub) {
        OAuth.ReplayingCredentials replay = oauth.replayingCredentials(scheme, "cid", "csecret", null);
        return new Client(HttpRuntime.Options.builder()
            .authorization(scheme, replay.provider())
            .httpClient(replay.transport(stub)).build());
    }
}
"#;
