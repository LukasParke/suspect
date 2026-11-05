# suspect-ref

The `$ref` resolution engine: workspace graph, JSON pointers, cycle
analysis and memoized resolution.

A `Workspace` (built with `WorkspaceBuilder`) owns loaded `LowDoc`s keyed by
canonical URI. Each `DocHandle` exposes the document's `$ref` edges,
resolves pointers and reference chains to a node, a whole document or a
detected cycle, and classifies loops as legal schema recursion or illegal
(unresolvable) cycles.

Part of [suspect](https://github.com/LukasParke/suspect), an OpenAPI,
Arazzo and Overlay toolchain. Most users want the `suspect` binary rather
than this crate; see the repository README for install instructions.

Licensed under either of MIT or Apache-2.0 at your option
([LICENSE-MIT](https://github.com/LukasParke/suspect/blob/main/LICENSE-MIT),
[LICENSE-APACHE](https://github.com/LukasParke/suspect/blob/main/LICENSE-APACHE)).
