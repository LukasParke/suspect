# Behavioral fragments

One focused, self-describing OpenAPI document per cross-backend behavior,
executed through the **generator** of every backend that claims the
behavior. Where the sibling `../fragments/` suite pins admission verdicts,
this suite pins emission: a backend that stops carrying a fragment's
markers fails `tests/behavioral_fragments.rs` hermetically, before any
native toolchain runs.

## Convention

- File name: kebab-case, describing the behavior
  (`pagination-limit-offset-and-cursor.yaml`).
- Leading `# INVARIANT:` comment stating what must hold — there may be
  more than one line, each starting with `# INVARIANT:`.
- `x-suspect-feature:` the `features.rs` feature id the fragment proves
  (e.g. `pagination`). The coverage test uses this to tie manifest claims
  to fragment coverage.
- `x-suspect-gated:` optional, default `true`. When `false`, the
  no-defaults control generation is skipped (use for behaviors that are
  not SDK-defaults-gated).
- `x-suspect-behavior:` map of backend name (`features.rs::BACKENDS`) to:
  - `file:` the emitted path the markers live in (pins the file contract)
  - `markers:` strings that must appear in that file's generated source.
    Use distinctive symbols — full signatures beat bare names.

## Adding a fragment

1. Write the smallest document that exercises the behavior.
2. State the invariant header.
3. Add `x-suspect-behavior` entries per backend, taking the markers from
   that backend's acceptance suite (the `*_pagination.rs`-style files) —
   they are the same assertions, re-pinned per fragment.
4. Run `cargo test -p suspect-codegen --test behavioral_fragments`. If a
   backend's markers fail, fix the markers against the emitted output —
   never loosen the invariant to make it pass.

## Coverage debt

`COVERAGE_DEBT` in `tests/behavioral_fragments.rs` records feature claims
whose only evidence is the per-backend native suite. Adding a fragment
that covers a debt entry requires removing the entry in the same commit —
the suite fails on stale debt.
