# suspect-jsonpath

RFC 9535 JSONPath queries over `suspect-low` nodes.

Compile once with `Path::parse`, evaluate many times with `Path::query`
against any `NodeRef` subtree. Results are deduplicated by source position
and ordered by document position. Evaluation is iterative, so deeply nested
documents cannot overflow the stack.

```rust
use suspect_jsonpath::Path;

let path = Path::parse("$.store.book[?(@.price < 10)].title").unwrap();
// let results = path.query(doc.root());
```

Part of [suspect](https://github.com/LukasParke/suspect), an OpenAPI,
Arazzo and Overlay toolchain. Most users want the `suspect` binary rather
than this crate; see the repository README for install instructions.

Licensed under either of MIT or Apache-2.0 at your option
([LICENSE-MIT](https://github.com/LukasParke/suspect/blob/main/LICENSE-MIT),
[LICENSE-APACHE](https://github.com/LukasParke/suspect/blob/main/LICENSE-APACHE)).
