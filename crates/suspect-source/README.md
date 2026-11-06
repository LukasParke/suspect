# suspect-source

Document loading for the suspect toolchain: raw bytes (memory-mapped or
pooled), encoding detection and transcoding, line indexes and canonical URIs.

Every `Source` is UTF-8. BOMs are stripped and UTF-16 input is transcoded
once at load, so all byte offsets in higher layers refer to the decoded
buffer. Everything above this crate is zero-copy over the buffers it loads.

Part of [suspect](https://github.com/LukasParke/suspect), an OpenAPI,
Arazzo and Overlay toolchain. Most users want the `suspect` binary rather
than this crate; see the repository README for install instructions.

Licensed under either of MIT or Apache-2.0 at your option
([LICENSE-MIT](https://github.com/LukasParke/suspect/blob/main/LICENSE-MIT),
[LICENSE-APACHE](https://github.com/LukasParke/suspect/blob/main/LICENSE-APACHE)).
