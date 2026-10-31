# Pinned remote acquisition and offline closure compilation

Status: implemented in `suspect-ref` and integrated into the public CLI and SDK
sessions on 2026-09-10, following the reviewed M1 design. The real CLI pipeline
has passed acquisition → offline generation/watch/comparison checks.

## Public CLI and generation

```sh
suspect acquire api.pins.json --cache-dir .suspect-cache --format json

suspect codegen --pins api.pins.json --cache-dir .suspect-cache \
  --profile typescript-http --package-name @example/sdk \
  --package-version 0.1.0 --out generated

suspect acquire api.pins.json --cache-dir .suspect-cache --offline --format json
```

`acquire` fills missing pins, or explicitly refreshes them with `--refresh` or
`--stale-before YYYY-MM-DDTHH:MM:SSZ`. Changed bytes or redirect provenance fail;
the manifest is never rewritten. `codegen --pins` always uses offline cache
verification and is mutually exclusive with a positional source file.

Session configurations choose `spec` **or** `pins`:

```json
{
  "pins": {
    "manifest": "./api.pins.json",
    "cache_dir": "./.suspect-cache"
  },
  "targets": [
    { "backend": "typescript-http", "package_name": "@example/sdk", "package_version": "0.1.0" }
  ]
}
```

Both paths are relative to the session configuration. This same configuration
works with `codegen-session` generation/check/preview/watch and `codegen-compare`.
The library entry point is `Session::with_input(Input::Pinned { ... }, config)`.

Before a pinned session reuses a cached Contract or artifact set, it verifies
every declared cached document. Missing/tampered cache files therefore fail even
on a warm snapshot. Revision identity includes the exact pin manifest and logical
provider fingerprint. Cache paths are watch/provenance inputs; generated sources
and `newDocuments` retain logical document URIs. The original source files and
server can be unavailable throughout offline generation.

Numeric-loopback HTTP fixtures require an explicit `--insecure-test-origin` during
acquisition and direct generation, or `pins.insecure_test_origins` in a session.
This test policy does not disable HTTPS verification or enable compilation-time
network requests.

Evidence: `target/sdk-pinned-generation-integration-02.log` (3 new cache/source
tests plus 8 session regressions) and `target/sdk-pinned-cli-integration-02.log`
(actual CLI remote acquisition, server shutdown, original-file deletion, offline
generation/watch/comparison, then missing-cache rejection).

## Boundary and source identity

Acquisition is an **explicit operation over declared retrieval requests**. A
versioned manifest pins the exact bytes of every document, including the local
entry and all local dependencies. Acquisition does not interpret `$ref`, infer a
download from `$id`, or scan directories to discover inputs.

Ordinary `Workspace` and `Contract::from_workspace` compilation remain offline.
A workspace can receive a concrete immutable `DocumentProvider` containing
already-owned bytes. That provider has no callback, file handle, network client,
or mutable buffer. A provider miss is terminal; it cannot fall back to a local
source file or a network request. Without a provider, local files remain usable
and remote loads return `RefError::RemoteDenied`.

The **effective logical retrieval URI** is each parsed document's identity and
initial reference base. Its requested URI is a lookup alias. Both identify the
same document slot; cache filenames are provenance only. Relative references
therefore resolve against the final retrieval address, followed by the existing
vocabulary-scoped JSON Schema `$id` rules. Source text is never rewritten.

Both aliases must be admitted by an explicit `allowed_documents` policy. The
policy is checked before byte-provider lookup or filesystem I/O, including entry
loads and already-loaded alias lookups. `logical_uris()` supplies the complete
requested/effective allowlist. Intermediate redirect addresses are ledger entries,
not additional document aliases unless separately declared as requests.

## Immutable version-1 manifest

The caller supplies a JSON file with this shape:

```json
{
  "manifest_version": 1,
  "entry": "https://spec.example.com/latest.json",
  "resources": [
    {
      "requested_uri": "https://spec.example.com/latest.json",
      "effective_uri": "https://spec.example.com/v1/schema.json",
      "digest": "sha256-44136fa355b3678a1146ad16f7e8649e94fb4fc21fe77e8310c060f61caaff8a",
      "media_type": "application/schema+json",
      "via": "redirect",
      "redirects": [
        {
          "from_uri": "https://spec.example.com/latest.json",
          "to_uri": "https://spec.example.com/v1/schema.json",
          "status": 302
        }
      ],
      "retrieved_at": "2026-09-09T00:00:00Z",
      "attempts": 2
    }
  ]
}
```

The example digest is the two-byte document `{}` and illustrates retrieval
metadata; this example is not an OpenAPI entry. For local documents, use an
absolute `file:///...` URI for both addresses, `via: "local"`, `redirects: []`,
and `attempts: 0`. Direct HTTP retrieval has equal requested/effective URIs,
`via: "direct"`, no hops, and `attempts: 1`.

Validation establishes these invariants before contacting any source:

- `manifest_version` is exactly 1. Unknown or duplicate JSON fields are errors.
- `entry` has its own `requested_uri` resource declaration.
- Each digest is `sha256-` plus 64 lowercase hexadecimal digits.
- URIs are absolute, fragment-free file/HTTP(S) retrieval addresses, with no
  embedded username/password, malformed escapes, controls, or whitespace. URI
  equivalences are those of the existing `suspect_source::Uri` module. Local
  file URIs cannot carry query strings or a remote filesystem authority.
- Redirects form a continuous declared chain of 301/302/303/307/308 responses,
  ending at `effective_uri`. `attempts` is the hop count plus one; retries are
  absent. Local entries cannot redirect.
- `retrieved_at` is a valid UTC `YYYY-MM-DDTHH:MM:SSZ` timestamp, year >= 1970.
- Media types are supported JSON/YAML types, including `+json`/`+yaml` suffixes.
  An explicit charset must be UTF-8. Responses must match the declared media
  type, ignoring supported parameters. Missing or unsupported types fail.
- Shared requested/effective identities must have identical digests, effective
  bases, and media types. Provider construction additionally compares actual
  bytes. Inconsistent duplicates fail rather than selecting the first value.

The validated `PinManifest`, resource declarations, and provided bytes expose
read-only accessors. Acquisition never generates, updates, or relaxes pins.

## Cache and verification

```text
<cache_dir>/pins/<manifest-sha256-hex>/manifest.json
<cache_dir>/pins/<manifest-sha256-hex>/<document-sha256-hex>/<requested-uri-sha256-hex>.blob
```

The manifest key hashes the **exact JSON input bytes**, including formatting.
Every cache load verifies the archived manifest and every declared document,
including documents that later prove unnecessary to the semantic closure.
Missing cache files fail offline loading. Tampered bytes fail digest verification;
an explicit refresh does not silently repair them. Cache paths must be regular
files; symlink cache entries are rejected.

All source bytes are validated before cache publication starts. Individual cache
files are installed atomically with create-new/hard-link operations, never by
overwriting an existing entry, and made read-only. The archived manifest is
published after the documents. Failed/cancelled publication may leave verified
content-addressed files, but never returns a partial provider. Every subsequent
load independently verifies completeness and digests.

The returned provider owns its bytes in memory. Editing or deleting an original
source, cache file, or manifest after acquisition cannot mutate that snapshot.
Future generation requests explicitly reverify cache inputs before reusing a
generation result. There is no ambient cache search or global "latest" pointer.

## Acquisition and refresh API

```rust
use suspect_ref::acquire::{acquire, AcquireOptions, RefreshPolicy};

let pins = acquire(&manifest_path, AcquireOptions {
    cache_dir: cache_dir.clone(),
    ..AcquireOptions::default()
})?;
```

`RefreshPolicy::Never` is the default: verify existing cached pins and, in an
explicit online acquisition call, fill missing entries. This distinguishes
initial acquisition from reacquisition. `All` rereads/refetches every source.
`StaleOnly { before: SystemTime }` refetches declarations whose `retrieved_at`
predates the caller-owned cutoff. `parse_utc_timestamp` parses that same UTC
syntax for callers. None of these policies rewrites the manifest timestamp.

Refresh must reproduce both exact bytes and declared retrieval provenance.
Changed bytes produce `DigestDrift { expected, actual }`; changed redirects or
effective addresses have separate drift errors. Existing cache files are all
verified before any refresh request starts. HTTP execution records retain actual
request counts and observed hops separately from declared metadata.

`offline: true` requires `RefreshPolicy::Never` and verifies only the declared
manifest and cache files. Original local sources are not opened. No transport or
credential hook is invoked. Cleartext test-origin permissions apply to transfers;
an already-verified offline HTTP-identified snapshot needs no network permission.

## Offline compiler and session integration

```rust
use std::sync::Arc;
use suspect_ir::contract::Contract;
use suspect_ref::acquire::{acquire, AcquireOptions};

let pins = acquire(&manifest_path, AcquireOptions {
    cache_dir: cache_dir.clone(),
    offline: true,
    ..AcquireOptions::default()
})?;
let workspace = Arc::new(pins.workspace_builder().build()?);
let contract = Contract::from_workspace(&workspace, pins.entry())?;
```

`pins.entry()` is the **effective** entry URI; `pins.requested_entry()` retains
the original lookup alias. Direct callers with an alias can first obtain
`workspace.open(alias)?.uri()` and pass that canonical effective identity to the
Contract compiler. This also makes redirected OpenAPI roots agree with their
owned source IDs and HTTP operation indices.

The equivalent explicit builder composition is:

```rust
let builder = suspect_ref::WorkspaceBuilder::new()
    .allowed_documents(pins.logical_uris())
    .document_provider(pins.provider());
```

The public input/invalidation interfaces are:

| API | Result |
| --- | --- |
| `AcquiredClosure::provider()` | `Arc<DocumentProvider>` |
| `AcquiredClosure::documents()` | `&[DocumentMetadata]` |
| `AcquiredClosure::fingerprint()` | Exact manifest SHA-256, `&str` |
| `AcquiredClosure::cache_manifest_path()` | Archived manifest `&Path` |
| `AcquiredClosure::records()` | `&[AcquisitionRecord]` |
| `DocumentProvider::document(&Uri)` | `Option<&ProvidedDocument>` |
| `DocumentProvider::fingerprint()` | Bytes/aliases/bases/media fingerprint |
| `ProvidedDocument::{metadata,bytes}()` | Immutable metadata / exact `&[u8]` |
| `DocumentMetadata::{requested_uri,effective_uri}()` | `&Uri` |
| `DocumentMetadata::{digest,byte_len,fingerprint}()` | `&str`, `u64`, `String` |
| `DocumentMetadata::{cache_path,manifest_path}()` | `Option<&Path>` |
| `Workspace::document_metadata(&Uri)` | Metadata even before parsing |
| `Workspace::document_provider()` | `Option<&Arc<DocumentProvider>>` |

Session invalidation can watch manifest/cache paths and re-run offline acquisition
before accepting a cache hit. Manifest fingerprints cover declared pins and
provenance; provider/document fingerprints cover semantic byte inputs, aliases,
bases and media types while excluding cache placement. `Workspace::uris()` lists
unique effective loaded identities; `failed_document_uris()` retains requested
addresses that semantic resolution attempted and could not load. Cache paths
belong in watch/provenance metadata, never in `SourceId` or reference-base logic.

## Transport and resource limits

The adapter runs one trusted curl child per HTTP hop, tested with curl 8.7.1.
The default executable is `/usr/bin/curl` on Unix (`curl.exe` on Windows); an
explicit `curl_program` can select the installed trusted executable.

The first argument is `--disable`, excluding `.curlrc`. The child environment is
cleared. Proxy use, netrc, retries, automatic redirects, and content decompression
are explicitly disabled. HTTP/1.1 and GET are fixed; no cookie jar, credential
file, alternate service file, HSTS file, or connection pool is used. HTTPS uses
certificate/hostname verification. The adapter never passes `--insecure` or
accepts ambient certificate/proxy settings.

`Credentials::headers(requested_uri, current_origin)` selects fresh credentials
for each hop, including same-origin redirects. Values go over child stdin, not
argv, cache files, or reports. Header names/values, reserved transport headers,
duplicates, and outgoing byte counts are checked before spawning curl. The
synchronous hook should return promptly and perform no I/O.

Cross-origin redirects are denied by default. Explicit
`allowed_redirect_origins` grants exact destination origins; the actual hop must
still match the declared URL/status ledger. Scheme, host, and effective port are
part of the origin. A denied or drifting hop is recorded before target I/O.
HTTP exceptions are confined to explicitly supplied numeric-loopback origins in
`insecure_test_origins`, including the pinned port. They do not relax TLS.

| Bound | Default |
| --- | --- |
| Document bytes | 64 MiB |
| Aggregate document bytes | 256 MiB |
| Declared retrieval requests | 10,000 |
| Manifest bytes | 4 MiB |
| Response header bytes / fields | 64 KiB / 128 |
| Outgoing header bytes | 16 KiB |
| Redirect hops | 3 |
| Operation-wide timeout | 30 seconds |

Both declared lengths and unknown-length/chunked bodies are bounded while
streaming, also by the remaining aggregate byte budget. curl runs with `--raw`;
the adapter decodes chunk framing itself and charges chunk extensions, delimiters
and trailers to the header budget. Wire metadata around a tiny body is therefore
bounded too. Header blocks and long unterminated header lines are bounded before
body processing. Compressed content
and compressed transfer encodings are rejected; the adapter never inflates an
unbounded representation. UTF-8 is validated before snapshot creation. UTF-8
BOMs remain in the pin digest; existing parser BOM handling governs source spans.

`CancellationToken` and the deadline are checked before I/O and during chunked
local reads. A supervising loop kills and reaps curl on cancellation, deadline,
or early size/header/media rejection, including blocked DNS/header/body phases.
Workspace byte/document caps also apply to lazy reference and direct entry loads;
lookup aliases do not consume additional document slots.

## Diagnostics, CLI routine, and checks

`AcquireError::kind()` yields a structured `AcquireErrorKind`; `code()` provides
stable `pin-*` codes. Manifest path, resource index, URI, cache/source path, JSON
parse-error line where available, expected/actual digests, and bounded redirect
evidence are accessible. Display/Debug redact URI queries and userinfo, and do
not copy credentials, server response bodies, or curl stderr into diagnostics.

The self-contained CLI routine is:

```rust
commands::acquire::run(&manifest_path, options, OutputFormat::Json)?;
```

Its `suspect.acquire.v1` report includes manifest fingerprint, requested/effective
entry, document/cache metadata, actual requests/redirects, and structured errors.
CLI URI labels are diagnostic-redacted; identity-sensitive consumers use the
complete URIs exposed by the manifest/provider APIs.
It returns 0 on a complete verified closure, 1 for acquisition findings, and 130
after Ctrl-C cancellation and child cleanup. Shared argument registration supplies
`AcquireOptions`; ordinary compilation selects `offline: true` explicitly.

Tests live in `crates/suspect-ref/tests/pinned_{provider,acquisition,transport}.rs`
at the public Workspace/Contract/provider seam. Independent TCP peers cover
acquisition followed by stopped-server offline remote-relative resolution,
redirected entries, credential reselection, default denial, immutable snapshots,
missing/tampered caches, refresh drift, unsupported media/UTF-8, framing/byte/header
limits, timeout/cancellation, and ambient proxy/config isolation. A self-signed
loopback TLS peer verifies rejection even with hostile `.curlrc`/CA environment
settings. Native transport tests use curl; that TLS fixture also uses OpenSSL and
Python 3. Generated fixtures stay under `target/sdk-acquire-fixtures`.

Acquisition preserves existing static-anchor behavior and source-linked
unsupported dynamic/embedded-resource SDK diagnostics. It does not broaden the
compiler's admitted schema dialects or guess missing anchor targets.

## Primary references

- [RFC3986 §5: reference resolution and base establishment](https://www.rfc-editor.org/rfc/rfc3986#section-5)
- [RFC3986 §6: equivalence and normalization](https://www.rfc-editor.org/rfc/rfc3986#section-6)
- [JSON Schema 2020-12 Core §8.2: resource identification](https://json-schema.org/draft/2020-12/json-schema-core#section-8.2)
- [OpenAPI 3.1.2 Reference Object](https://spec.openapis.org/oas/v3.1.2.html#reference-object)
