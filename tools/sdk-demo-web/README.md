# Local SDK playground

From the repository root:

```sh
./demo-web.sh
```

Open **http://127.0.0.1:8765**. The page shows twelve native SDKs, their verified
four-line examples, and a separate JavaScript bonus. Add an OpenRouter API key,
then click **Run live request** on a card or **Run all 12**. If
`OPENROUTER_API_KEY` is already exported, the server loads it at startup.

The request is **`GET https://openrouter.ai/api/v1/key`** (`getCurrentKey`), using
a normal API key. Each button starts that language's accepted, prebuilt native
consumer in the background. Success requires its actual HTTP 200 and decoded
confirmation. **Ready** means local preflight; it is not a live API result.

```sh
./demo-web.sh --port 8767  # choose another loopback port
./demo-web.sh --preflight # check all pins, without running a native request
```

This launcher uses Python's standard library and local CSS/JavaScript/system fonts.
It needs the prepared workspace described in
[`LIVE-ENV-DEMO-README.md`](../../LIVE-ENV-DEMO-README.md). Its packages are local
version `0.1.0`. The complete native programs, package READMEs, and verified pins
are linked from the page.

## Session behavior

- Three concurrent jobs; 15-second native deadlines and a 22-second process deadline.
- Per-card cancellation and **Cancel all** stop owned native process groups.
- Replacing or clearing the key cancels active and queued jobs. Keys stay in RAM;
  the browser receives only a readiness boolean. Saving a key does not run a job.
- The loopback server uses exact Host/Origin checks and a page-bootstrap nonce.
  Dispatch accepts fixed language IDs and the current-key read operation.
- A fresh `target/sdk-demo-web-20260911-NN/` holds preflight and append-only,
  redacted scalar job receipts. Raw child streams are never stored or returned.
  A stream over 65,536 bytes is omitted in its entirety and cannot qualify as success.
- A session retains at most 64 jobs in memory and admits at most 256 jobs. Ctrl-C
  stops the server and cancels its owned jobs.

## Verification

Backend checks use controlled local children, including process-tree shutdown:

```sh
python3 -B -m unittest discover -s tools/sdk-demo-web/tests -p 'test_*.py' -v
```

With the actual default server running, the browser check uses the existing
`playwright-core` installation from the native-host tool root and installed Chrome:

```sh
node tools/sdk-demo-web/tests/browser_check.mjs \
  --url http://127.0.0.1:8765 \
  --output target/sdk-demo-web-20260911-01/checks/browser-NEW
```

`--output` must be fresh. `--tools` and `--chrome` can point at existing local
installations. The browser script permits only GETs to the actual default server.
Interactive execution checks use `tests/controlled_server.py`, with a persistent
**CONTROLLED UI TEST** banner and local test children. The public launcher always
uses the accepted native runtimes.

Delivery evidence and screenshots are indexed in
[`target/sdk-demo-web-20260911-01/HANDOFF.md`](../../target/sdk-demo-web-20260911-01/HANDOFF.md).
