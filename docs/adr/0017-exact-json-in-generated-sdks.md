# Generated SDKs keep exact JSON values through a generator-owned JSON runtime

Generated models keep number tokens exactly as written. A general `number` becomes a string-backed `JsonNumber`, and an unbounded integer becomes a big integer (`bigint`, `JsonInteger`). Only a range the schema proves to be safe uses the host's native number type. Absent, explicit null and value are distinct states in every language (`UNSET`, `Presence[T]`, `Absent | …`). To make this hold identically everywhere, each package emits its own small JSON runtime and checked codecs instead of relying on the platform's JSON library. Platform libraries round through binary floats, differ on duplicate keys and lone surrogates, and have no input limits. The runtime rejects duplicate keys, malformed UTF-8 and lone surrogates, and enforces byte, depth and work budgets. A backend may wrap a platform tokenizer (Go wraps `encoding/json` with `UseNumber`) only when its own guards keep this contract.

## Consequences

Callers convert explicitly for arithmetic and native JSON interop. TypeScript `JsonNumber.toJSON()` throws, so `JSON.stringify` fails rather than lose precision. Go models without codecs return an error from `MarshalJSON` rather than emit unvalidated JSON. Object key order is not part of the contract, so cross-language comparisons must normalize it.
