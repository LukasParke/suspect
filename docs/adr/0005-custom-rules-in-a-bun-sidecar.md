# Custom TS/JS rules run in a long-lived Bun sidecar

User-authored TypeScript rules run in a persistent `bun` worker that suspect spawns once and talks to over a versioned NDJSON protocol on stdio (`crates/suspect-rules`). The host evaluates each rule's `given` selector natively against the CST and ships only the matched nodes. It enforces a per-run deadline (250 ms default): on breach it kills and restarts the worker and disables the rule for the session. Pure leaf functions (`casing`, `pattern`, …) exist in both Rust and TS, kept identical by a shared conformance corpus, because a per-node cross-process call costs about 10x the function body.

## Considered Options

- Embedding Bun: it has no embedding API, and raw JavaScriptCore loses TS execution.
- A subprocess per evaluation: a 30–100 ms cold start on every keystroke breaks the editor latency budget.
- Embedding V8/deno_core: a different runtime from the user's toolchain, and a much heavier embed.

## Consequences

Custom rules run arbitrary code, so they are opt-in through config. With `bun: auto`, TS rules are skipped when `bun` is not on PATH; `bun: require` makes that an error.
