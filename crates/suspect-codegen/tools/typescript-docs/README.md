# Native TypeScript documentation toolchain

This private development tool builds TypeDoc from generated `models.ts` and
checks the result against `docs-manifest.json`, which comes from the canonical
model plan. It does not infer a second set of SDK signatures. None of its
dependencies become generated SDK runtime dependencies.

The direct versions are pinned in `package.json`: Node 22.23.1, npm 10.9.8,
TypeDoc 0.28.15, TypeScript 5.9.3 and parse5 7.3.0. parse5 inspects actual HTML
without executing source prose or generated scripts. The build validates model
coverage, schema-origin text, missing-codec notices, literal source descriptions,
local links and anchors, and active-content restrictions. It writes
`docs/html/`, `docs/reflections.json` and `docs/coverage.json` inside the generated
TypeScript directory. Coverage includes the dependency-lock hash and missing
source descriptions. A successful docs check does not unblock SDK release.

## Current native gate

The dependency lock is resolved and the pinned toolchain is installed. Earlier
DNS failures cleared on 2026-09-08; both lock resolution and `npm ci` succeeded
against the official registry. All four documentation tests pass, including
native HTML for hostile source prose and tracked OpenRouter caller/image
closures. The gate uses TypeDoc's actual renderer router to locate pages rather
than guessing filenames or using removed reflection URL properties. Evidence:
`target/typescript-native-docs-tests.log`.

For an intentional dependency update, use the pinned Node/npm to resolve the
exact direct dependencies and review the resulting `package-lock.json`:

```sh
npm install --package-lock-only --ignore-scripts \
  --registry=https://registry.npmjs.org \
  --cache /private/tmp/suspect-typedoc-npm-cache --no-audit --no-fund
npm ci --ignore-scripts --registry=https://registry.npmjs.org \
  --cache /private/tmp/suspect-typedoc-npm-cache --no-audit --no-fund
node build.mjs /path/to/generated/typescript
```

Subsequent builds use `npm ci` with the recorded lock, then run the native tests:

```sh
SUSPECT_DOCS_NODE=/path/to/node-22.23.1 \
  cargo test --offline -p suspect-codegen --test typescript_docs \
  -- --include-ignored --nocapture
```

The ordinary artifact/manifest test and separate native TypeScript comment
consumer also run. Complete SDK documentation and client examples remain
separate release requirements; these native checks cover the selected model
documentation only.
