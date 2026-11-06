# suspect-low

A generic ordered document model with source maps over `suspect-syntax`
CSTs: `LowDoc`, `NodeRef` and `Pointer`.

It provides typed scalar access, RFC 6901 JSON Pointer navigation,
transparent YAML alias resolution, merge-key (`<<`) expansion,
duplicate-key reporting, and OpenAPI/Arazzo/Overlay family sniffing. Every
higher layer of the toolchain builds on this API.

Part of [suspect](https://github.com/LukasParke/suspect), an OpenAPI,
Arazzo and Overlay toolchain. Most users want the `suspect` binary rather
than this crate; see the repository README for install instructions.

Licensed under either of MIT or Apache-2.0 at your option
([LICENSE-MIT](https://github.com/LukasParke/suspect/blob/main/LICENSE-MIT),
[LICENSE-APACHE](https://github.com/LukasParke/suspect/blob/main/LICENSE-APACHE)).
