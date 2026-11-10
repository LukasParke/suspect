# References into other files become components of the IR snapshot

When the IR snapshot (`IrSpec`) is built from a description that references other documents, every schema, response, parameter, request body, header and path item reached through such a reference is loaded through the workspace, copied into the snapshot's `components` under a generated name, and the reference is rewritten to that name. References inside a copied target are rewritten against its own document, so recursion stays a reference. Everything that reads the snapshot (the mock, the contract checks, stateful and fuzz generation) then treats a multi-file description like a bundled one. A reference that cannot be loaded stays as written and is reported by whatever needs it; it is never treated as "no constraint".

## Considered Options

- Leave external references opaque and tell users to bundle first: rejected. `suspect bundle --strategy inline` cannot inline a recursive reference, so a multi-file description with any recursion had no path to a contract test, and until then its bodies passed unchecked.

## Consequences

Generated names come from the target's last pointer token or file stem, made unique against the entry document's own components; they are stable for a given description but are not names its author chose. A self-contained document never loads a workspace.
