# Independent YAML value oracle

The pinned `yaml` 2.8.1 parser supplies a YAML 1.2 core-schema AST, independently
of Suspect's CST/Fast readers. `normalize.mjs` walks that AST and writes JSON.
Numeric nodes use their original scalar source and `JSON.rawJSON`, avoiding
JavaScript floating-point conversion. Strings/booleans/null retain their parsed
kind. Duplicate keys, aliases, unsupported scalar kinds and parser findings fail.

Run with pinned Node 22.23.1 after `npm ci --ignore-scripts`:

```sh
node normalize.mjs INPUT.yaml NEW_OUTPUT.json
```

The output's adjacent `.meta.json` records input/output, script and dependency
lock digests, tool versions and visit counts. The milestone runner copies the
tool into its fresh evidence directory and installs dependencies offline there.
Hand-authored normative vectors remain separate from this corpus-wide oracle.
