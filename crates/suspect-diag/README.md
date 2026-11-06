# suspect-diag

The finding model shared by every suspect check: `Finding`, `Severity`,
`Level` (a severity or `off`), the one severity-name parser, static rule
metadata (`RuleInfo`), and the text, JSON and SARIF 2.1.0 renderers.

This is a leaf crate. It depends only on `suspect-source` (for `Uri`),
`serde` and `serde_json`, so configuration, check crates and front ends can
all use it without forming a cycle.

Part of [suspect](https://github.com/LukasParke/suspect), an OpenAPI,
Arazzo and Overlay toolchain. Most users want the `suspect` binary rather
than this crate; see the repository README for install instructions.

Licensed under either of MIT or Apache-2.0 at your option
([LICENSE-MIT](https://github.com/LukasParke/suspect/blob/main/LICENSE-MIT),
[LICENSE-APACHE](https://github.com/LukasParke/suspect/blob/main/LICENSE-APACHE)).
