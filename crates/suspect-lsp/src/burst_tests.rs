//! End-to-end burst tests that drive the real server through `LspService`.

use super::*;
use std::time::Duration;
use tower::Service;

/// One request, with the timeout that turns a wedge into a failure
/// instead of a hung test run.
/// Hover repeatedly until it answers, or `budget` seconds run out.
/// A wedged server never answers no matter how long you wait; a slow
/// one answers on a later try. One sample cannot tell those apart.
async fn hover_within(
    service: &mut LspService<Backend>,
    td: &serde_json::Value,
    at: &serde_json::Value,
    budget: f64,
) -> Option<tower_lsp::jsonrpc::Response> {
    let deadline = std::time::Instant::now() + Duration::from_secs_f64(budget);
    // Ids come from a process-wide counter and never repeat. tower-lsp
    // registers every request id when it arrives and only unregisters
    // it when the handler completes; abandoning a request by timeout
    // leaves the id registered, and a later request that reuses it is
    // answered `InvalidRequest`. A per-call counter was not enough —
    // each hover_within call restarted it, so a leaked id from one
    // notification's hover collided with the next one's first attempt.
    // Reproduced deterministically by shrinking the per-attempt timeout
    // (HOVER_ATTEMPT_MS) until attempts get abandoned.
    static NEXT_ID: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(20_000);
    // Each abandoned attempt doubles the next one's budget, so a slow
    // first attempt cannot starve: given enough retries, one attempt is
    // always given more time than the request needs. Abandoning also
    // cancels — a dropped request leaves its id registered in the
    // server's cancellation map, and cancelling is both the correct
    // protocol move and what keeps that map clean.
    let mut attempt_ms = std::env::var("HOVER_ATTEMPT_MS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(250u64);
    loop {
        let left = deadline.saturating_duration_since(std::time::Instant::now());
        if left.is_zero() {
            return None;
        }
        let id = NEXT_ID.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let answer = in_flight(
            service,
            id,
            "textDocument/hover",
            serde_json::json!({"textDocument": td, "position": at}),
        );
        let budget = left.min(Duration::from_millis(attempt_ms));
        match tokio::time::timeout(budget, answer).await {
            Ok(Some(response)) => return Some(response),
            _ => {
                let _ =
                    in_flight(service, 0, "$/cancelRequest", serde_json::json!({"id": id})).await;
                attempt_ms = attempt_ms.saturating_mul(2).min(left.as_millis() as u64);
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        }
    }
}

fn in_flight(
    service: &mut LspService<Backend>,
    id: i64,
    method: &str,
    params: serde_json::Value,
) -> Answer {
    // Each request needs its own id: the server tracks in-flight ids
    // for cancellation, and a duplicate is rejected as invalid. An id
    // of zero means a notification — and a notification method sent
    // *with* an id is silently dropped by tower-lsp, so this is not
    // optional.
    let mut builder = tower_lsp::jsonrpc::Request::build(method.to_owned()).params(params);
    if id != 0 {
        builder = builder.id(id);
    }
    let request = builder.finish();
    // `call` hands back a `'static` future, so the borrow of the
    // service ends here and the request stays in flight while the
    // next one is issued.
    let answer = service.call(request);
    Box::pin(async move {
        tokio::time::timeout(std::time::Duration::from_secs(20), answer)
            .await
            .ok()
            .and_then(|result| result.ok())
            .flatten()
    })
}

/// One answer, boxed so a batch of in-flight requests borrows nothing.
type Answer = std::pin::Pin<
    Box<dyn std::future::Future<Output = Option<tower_lsp::jsonrpc::Response>> + Send>,
>;

/// A specification with enough references that the parts of the server
/// which walk the workspace have something to walk.
fn spec_with_refs(operations: usize) -> String {
    let mut out = String::from("openapi: 3.1.1\ninfo: {title: Burst, version: '1'}\npaths:\n");
    for index in 0..operations {
        out.push_str(&format!(
                "  /resource{index}:\n    get:\n      operationId: op{index}\n      responses:\n        '200':\n          description: ok\n          content:\n            application/json:\n              schema: {{$ref: '#/components/schemas/Model'}}\n"
            ));
    }
    out.push_str(
            "components:\n  schemas:\n    Model:\n      type: object\n      properties:\n        id: {type: string}\n",
        );
    out
}

/// Everything an editor sends when it opens a YAML document, issued
/// together the way an editor issues it.
///
/// This is the test a request-by-request probe cannot be. Hover alone
/// always answered in milliseconds; the server only ever wedged once
/// an editor asked for something else first, and then *every* later
/// request queued behind a lock that would never be released — which
/// is what "loading forever" looked like from the editor's side. One
/// handler (`document_link`) held a read guard while asking for a
/// write lock, and because tokio's RwLock is write-preferring, its
/// queued writer then blocked every later reader too.
///
/// All requests are put in flight before any is awaited. Awaiting
/// them one at a time would pass: the deadlock needs a second request
/// waiting behind the first.
///
/// `textDocument/diagnostic` is the one that has to be in this burst.
/// It is the only handler that took a read guard across the lint pass
/// *and then took a second read* for the severity floor. `didOpen`
/// spawns `warm_index`, which queues a writer once its blocking build
/// finishes; against that queued writer the second read blocks, and
/// since the first guard cannot drop until the second read returns,
/// the pair never resolves. An editor sends `didOpen` and
/// `textDocument/diagnostic` in the same instant, so this is not a
/// race that needs bad luck — it is every file open.
#[tokio::test]
async fn every_request_an_open_triggers_answers() {
    let dir = tempfile::tempdir().expect("tempdir");
    let text = spec_with_refs(2000);
    std::fs::write(dir.path().join("openapi.yaml"), &text).expect("write");
    let path = dir.path().join("openapi.yaml");
    let uri = Uri::parse(&format!("file://{}", path.display())).expect("uri");

    let (mut service, socket) = service().await;
    let root = format!("file://{}", dir.path().display());

    // Answer the server's own requests, as an editor does. Without
    // this the server waits forever on `workspace/configuration`
    // during `initialized`, and a test that ignores that wait spends
    // its whole budget in it.
    let answering = tokio::spawn(async move {
        use futures::{SinkExt, StreamExt};
        let mut socket = socket;
        while let Some(request) = socket.next().await {
            let result = match request.method() {
                "workspace/configuration" => {
                    let items = request
                        .params()
                        .and_then(|p| p.get("items"))
                        .and_then(|i| i.as_array())
                        .map_or(0, Vec::len);
                    serde_json::Value::Array(vec![serde_json::Value::Null; items])
                }
                "workspace/applyEdit" => serde_json::json!({"applied": true}),
                _ => serde_json::Value::Null,
            };
            if let Some(id) = request.id().cloned() {
                socket
                    .send(tower_lsp::jsonrpc::Response::from_ok(id, result))
                    .await
                    .ok();
            }
        }
    });

    assert!(
        in_flight(
            &mut service,
            1,
            "initialize",
            serde_json::json!({
                "processId": null,
                "rootUri": root,
                "capabilities": {"workspace": {"configuration": true}},
            }),
        )
        .await
        .is_some(),
        "initialize answered"
    );
    // `initialized` is a notification: sent, and its (empty) answer read.
    let _ = in_flight(&mut service, 0, "initialized", serde_json::json!({})).await;
    let _ = in_flight(
        &mut service,
        0,
        "textDocument/didOpen",
        serde_json::json!({"textDocument": {
            "uri": uri.as_str(), "languageId": "yaml", "version": 1, "text": text,
        }}),
    )
    .await;
    // Let the warmed index and the first diagnostics finish, so this
    // measures concurrency rather than cold start.
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    // Line 11 of the fixture is the `$ref` on the first operation's
    // 200 response; character 24 is inside the key, which must always
    // render the keyword entry. The fixture is small on purpose: what
    // this test reproduces is a lock-ordering bug, which has nothing to
    // do with how much document there is.
    let at = serde_json::json!({"line": 11, "character": 24});
    let td = serde_json::json!({"uri": uri.as_str()});
    let span = serde_json::json!({
        "start": {"line": 11, "character": 0},
        "end": {"line": 12, "character": 0},
    });
    let requests: Vec<(&str, serde_json::Value)> = vec![
        (
            "textDocument/hover",
            serde_json::json!({"textDocument": td, "position": at}),
        ),
        (
            "textDocument/documentSymbol",
            serde_json::json!({"textDocument": td}),
        ),
        (
            "textDocument/foldingRange",
            serde_json::json!({"textDocument": td}),
        ),
        (
            "textDocument/semanticTokens/full",
            serde_json::json!({"textDocument": td}),
        ),
        (
            "textDocument/documentLink",
            serde_json::json!({"textDocument": td}),
        ),
        (
            "textDocument/inlayHint",
            serde_json::json!({"textDocument": td, "range": span}),
        ),
        (
            "textDocument/documentHighlight",
            serde_json::json!({"textDocument": td, "position": at}),
        ),
        (
            "textDocument/selectionRange",
            serde_json::json!({"textDocument": td, "positions": [at]}),
        ),
        (
            "textDocument/codeLens",
            serde_json::json!({"textDocument": td}),
        ),
        (
            "textDocument/documentColor",
            serde_json::json!({"textDocument": td}),
        ),
        (
            "textDocument/codeAction",
            serde_json::json!({"textDocument": td, "range": span, "context": {"diagnostics": []}}),
        ),
        (
            "textDocument/completion",
            serde_json::json!({"textDocument": td, "position": at}),
        ),
        (
            "textDocument/prepareCallHierarchy",
            serde_json::json!({"textDocument": td, "position": at}),
        ),
        (
            "textDocument/prepareTypeHierarchy",
            serde_json::json!({"textDocument": td, "position": at}),
        ),
        (
            "textDocument/moniker",
            serde_json::json!({"textDocument": td, "position": at}),
        ),
        (
            "textDocument/linkedEditingRange",
            serde_json::json!({"textDocument": td, "position": at}),
        ),
        (
            "textDocument/prepareRename",
            serde_json::json!({"textDocument": td, "position": at}),
        ),
        (
            "textDocument/diagnostic",
            serde_json::json!({"textDocument": td}),
        ),
        ("workspace/diagnostic", serde_json::json!({})),
        (
            "textDocument/definition",
            serde_json::json!({"textDocument": td, "position": at}),
        ),
        (
            "textDocument/references",
            serde_json::json!({"textDocument": td, "position": at, "context": {"includeDeclaration": true}}),
        ),
    ];

    // Every request put in flight before any is awaited.
    let mut pending: Vec<(&str, Answer)> = Vec::new();
    for (n, (method, params)) in requests.iter().enumerate() {
        pending.push((
            *method,
            in_flight(&mut service, 100 + n as i64, method, params.clone()),
        ));
    }

    for (method, answer) in pending {
        let got = answer.await;
        assert!(
            got.is_some(),
            "{method} never answered: the server is wedged"
        );
    }

    // And the one an editor sends on every cursor move, afterwards.
    let after = in_flight(
        &mut service,
        900,
        "textDocument/hover",
        serde_json::json!({"textDocument": td, "position": at}),
    )
    .await
    .expect("hover answered after the burst");
    let value = after.result().expect("hover returned a result");
    assert!(
        !value.is_null(),
        "hover must still work once the burst is over"
    );

    // Every *notification* the client sends while you edit. A
    // notification carries no id, so it produces no reply to wait on —
    // which is exactly why a notification that wedges the server is
    // invisible until the next request also stops answering. `didSave`
    // is the one that matters: the server advertises `save`, so this
    // arrives on every save, and a hover afterwards must still work.
    for (method, params) in [
        (
            "textDocument/didSave",
            serde_json::json!({"textDocument": td}),
        ),
        (
            "workspace/didChangeWatchedFiles",
            serde_json::json!({"changes": [{"uri": td["uri"], "type": 2}]}),
        ),
        (
            "workspace/didChangeConfiguration",
            serde_json::json!({"settings": {}}),
        ),
    ] {
        let _ = in_flight(&mut service, 0, method, params.clone()).await;
        let after = hover_within(&mut service, &td, &at, 20.0)
            .await
            .unwrap_or_else(|| panic!("{method} wedged the server: hover never answered"));
        // An error response has no result, and the interesting question
        // is always which error — so print it rather than guessing.
        let Some(result) = after.result() else {
            panic!(
                "{method} made hover answer with an error: {:?}",
                after.error()
            );
        };
        assert!(
            !result.is_null(),
            "{method} wedged the server: hover came back empty"
        );
    }

    // And the command that takes the write lock while a document
    // borrow is live — the same shape, on the other handler.
    in_flight(
        &mut service,
        920,
        "workspace/executeCommand",
        serde_json::json!({
            "command": "suspect.changeImpact",
            "arguments": [td["uri"], 11, 24],
        }),
    )
    .await
    .expect("changeImpact answered");
    let after = hover_within(&mut service, &td, &at, 20.0)
        .await
        .expect("changeImpact must not wedge the server");
    assert!(
        !after.result().expect("hover returned a result").is_null(),
        "changeImpact must not wedge the server"
    );

    answering.abort();
}

/// A writer queued while `textDocument/diagnostic` is running must not
/// be able to wedge it.
///
/// This is the trigger for "hover spins forever" on a large document.
/// `didOpen` spawns `warm_index`, which builds the index off the
/// interaction path and then queues a `state.write()`. `diagnostic`
/// used to hold a read guard across the whole lint pass and then take
/// a *second* read for the severity floor. tokio's `RwLock` is
/// write-preferring, so with that writer queued the second read waits
/// for a writer that is itself waiting for the first guard to drop —
/// and because the queued writer also blocks every later reader, one
/// diagnostic wedges the entire server, hover included.
///
/// Scope of this test, stated honestly: it pins the request ordering
/// that triggers the bug (`didOpen` and `textDocument/diagnostic`
/// together, which is what an editor sends on every open) and it
/// requires both to answer with a live hover afterwards. It does *not*
/// reproduce the wedge on a synthetic fixture — `warm_index`'s build
/// has to outlast the lint, which only happens at the scale of a real
/// 63k-line specification. The deterministic reproduction of that is
/// the JSON-RPC bisect against the real spec, which reported
/// `textDocument/diagnostic WEDGED, hover afterwards: ALSO DEAD`
/// before this fix and `answered` after it.
#[tokio::test]
async fn a_queued_writer_does_not_wedge_diagnostics() {
    let dir = tempfile::tempdir().expect("tempdir");
    let text = spec_with_refs(2000);
    std::fs::write(dir.path().join("openapi.yaml"), &text).expect("write");
    let path = dir.path().join("openapi.yaml");
    let uri = Uri::parse(&format!("file://{}", path.display())).expect("uri");
    let (mut service, socket) = service().await;
    let answering = tokio::spawn(async move {
        use futures::{SinkExt, StreamExt};
        let mut socket = socket;
        while let Some(request) = socket.next().await {
            let result = match request.method() {
                "workspace/configuration" => {
                    let items = request
                        .params()
                        .and_then(|p| p.get("items"))
                        .and_then(|i| i.as_array())
                        .map_or(0, Vec::len);
                    serde_json::Value::Array(vec![serde_json::Value::Null; items])
                }
                _ => serde_json::Value::Null,
            };
            if let Some(id) = request.id().cloned() {
                socket
                    .send(tower_lsp::jsonrpc::Response::from_ok(id, result))
                    .await
                    .ok();
            }
        }
    });

    in_flight(
        &mut service,
        1,
        "initialize",
        serde_json::json!({
            "processId": null,
            "rootUri": format!("file://{}", dir.path().display()),
            "capabilities": {"workspace": {"configuration": true}},
        }),
    )
    .await
    .expect("initialize answered");
    let _ = in_flight(&mut service, 0, "initialized", serde_json::json!({})).await;

    let td = serde_json::json!({"uri": uri.as_str()});

    // The editor sends `didOpen` and `textDocument/diagnostic` in the
    // same instant, and the order matters: `didOpen` spawns the tasks
    // that queue a writer, so if the diagnostic is already running
    // when they arrive it is holding a read guard across the whole
    // lint. Awaiting `didOpen` first would let those writers finish
    // first and hide the bug, which is exactly what a request-at-a-
    // time probe does.
    let open = in_flight(
        &mut service,
        0,
        "textDocument/didOpen",
        serde_json::json!({"textDocument": {
            "uri": uri.as_str(), "languageId": "yaml", "version": 1, "text": text,
        }}),
    );
    let diagnostic = in_flight(
        &mut service,
        100,
        "textDocument/diagnostic",
        serde_json::json!({"textDocument": td}),
    );
    let joined = tokio::time::timeout(std::time::Duration::from_secs(30), async {
        tokio::join!(open, diagnostic)
    })
    .await
    .expect("textDocument/diagnostic never answered: a queued writer wedged it");
    let answer = joined.1.expect("the diagnostic returned no response");
    assert!(
        answer.result().is_some(),
        "textDocument/diagnostic returned no result"
    );
    let at = serde_json::json!({"line": 11, "character": 24});
    let hover = in_flight(
        &mut service,
        200,
        "textDocument/hover",
        serde_json::json!({"textDocument": td, "position": at}),
    )
    .await
    .expect("hover never answered: the server is still wedged");
    assert!(
        !hover.result().expect("hover result").is_null(),
        "hover came back empty after the diagnostic"
    );

    answering.abort();
}

/// Every hover answer must be markdown-kind on the wire: VS Code
/// renders `HoverContents::Markup(Markdown)` as markdown, and any
/// plaintext path in any hover surface would show the card's own
/// markdown as raw text — precisely the "not visually appealing"
/// failure the card redesign set out to fix. This drives the real
/// server across every hover surface: a keyword key, a description
/// value, a `$ref` value, a component key, a plain key, and a
/// configuration key.
#[tokio::test]
async fn every_hover_answer_is_markdown_kind() {
    let dir = tempfile::tempdir().expect("tempdir");
    let spec = "\
openapi: 3.1.0
info:
  title: T
  version: '1'
  description: Rates a **media item**.
paths:
  /pets:
    get:
      operationId: listPets
      responses:
        '200':
          description: ok
          content:
            application/json:
              schema:
                $ref: '#/components/schemas/Pet'
components:
  schemas:
    Pet:
      type: object
      properties:
        name: {type: string}
";
    std::fs::write(dir.path().join("openapi.yaml"), spec).expect("write");
    std::fs::write(
        dir.path().join(".suspect.yaml"),
        "lint:\n  min_severity: warning\n",
    )
    .expect("write");
    let spec_uri = format!("file://{}", dir.path().join("openapi.yaml").display());
    let conf_uri = format!("file://{}", dir.path().join(".suspect.yaml").display());

    let (mut service, socket) = service().await;
    let answering = tokio::spawn(async move {
        use futures::{SinkExt, StreamExt};
        let mut socket = socket;
        while let Some(request) = socket.next().await {
            let result = match request.method() {
                "workspace/configuration" => {
                    let items = request
                        .params()
                        .and_then(|p| p.get("items"))
                        .and_then(|i| i.as_array())
                        .map_or(0, Vec::len);
                    serde_json::Value::Array(vec![serde_json::Value::Null; items])
                }
                _ => serde_json::Value::Null,
            };
            if let Some(id) = request.id().cloned() {
                socket
                    .send(tower_lsp::jsonrpc::Response::from_ok(id, result))
                    .await
                    .ok();
            }
        }
    });

    in_flight(
        &mut service,
        1,
        "initialize",
        serde_json::json!({
            "processId": null,
            "rootUri": format!("file://{}", dir.path().display()),
            "capabilities": {"workspace": {"configuration": true}},
        }),
    )
    .await
    .expect("initialize answered");
    let _ = in_flight(&mut service, 0, "initialized", serde_json::json!({})).await;
    let _ = in_flight(
        &mut service,
        0,
        "textDocument/didOpen",
        serde_json::json!({"textDocument": {
            "uri": spec_uri, "languageId": "yaml", "version": 1, "text": spec,
        }}),
    )
    .await;
    let _ = in_flight(
        &mut service,
        0,
        "textDocument/didOpen",
        serde_json::json!({"textDocument": {
            "uri": conf_uri, "languageId": "yaml", "version": 1,
            "text": "lint:\n  min_severity: warning\n",
        }}),
    )
    .await;
    // Let the opened documents settle (workspace build, first lint)
    // so this asserts the steady-state answer, not a race with it.
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    // One position per hover surface, in the fixture above.
    let hovers: Vec<(&str, (u32, u32), &str)> = vec![
        ("keyword key", (0, 4), &spec_uri),
        ("description value", (4, 20), &spec_uri),
        ("$ref key", (15, 18), &spec_uri),
        ("component key", (18, 5), &spec_uri),
        ("plain key", (21, 10), &spec_uri),
        ("configuration key", (1, 6), &conf_uri),
    ];
    for (n, (index, (line, character), uri)) in hovers.iter().enumerate() {
        let answer = in_flight(
            &mut service,
            300 + n as i64,
            "textDocument/hover",
            serde_json::json!({
                "textDocument": {"uri": uri},
                "position": {"line": line, "character": character},
            }),
        )
        .await
        .unwrap_or_else(|| panic!("{index}: hover never answered"));
        let result = answer.result().unwrap_or_else(|| {
            panic!(
                "{index}: hover answered with an error: {:?}",
                answer.error()
            )
        });
        let contents = &result["contents"];
        assert_eq!(
            contents["kind"], "markdown",
            "{index}: hover must be markdown-kind, got {contents}"
        );
        assert!(
            contents["value"].as_str().is_some_and(|v| !v.is_empty()),
            "{index}: hover markdown must be non-empty"
        );
    }

    // The $ref card pins the restyled frame on the wire: the heading
    // the component renderers open with is what the editor receives.
    let answer = in_flight(
        &mut service,
        400,
        "textDocument/hover",
        serde_json::json!({
            "textDocument": {"uri": spec_uri},
            "position": {"line": 15, "character": 18},
        }),
    )
    .await
    .expect("$ref hover answered");
    let value = answer.result().expect("hover result")["contents"]["value"]
        .as_str()
        .expect("markdown string");
    assert!(value.contains("### `Pet`"), "card frame missing: {value}");

    answering.abort();
}
