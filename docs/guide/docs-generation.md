# Documentation generation

suspect renders API reference documentation from an OpenAPI document in two
ways. `suspect gen docs` writes a finished reference, and `suspect gen
template` renders your own templates. Native SDK documentation is separate:
each SDK profile emits its own (see the [SDK generation guide](sdk-generation.md)).
Flags are in the [command reference](../reference/cli.md#suspect-gen-docs).

## Reference docs: `gen docs`

```console
$ suspect gen docs api.yaml -o api.html                          # one self-contained HTML file
$ suspect gen docs api.yaml --style markdown -o docs/api         # Markdown pages
$ suspect gen docs api.yaml --style sveltekit -o site            # a runnable SvelteKit site
$ suspect gen docs api.yaml --title "Widgets API" -o api.html    # override info.title
```

| Style | Output |
|---|---|
| `html` (default) | one self-contained HTML file; printed to stdout without `-o` |
| `markdown` | a directory: an index, `toc.json`, one page per tag and one per schema, with frontmatter for MDX or MDsvex pipelines |
| `sveltekit` | a directory: a generated data and routes layer, plus a maintained layer scaffolded once and then left to you |

`markdown` and `sveltekit` need `-o`. The defaults can live in
`.suspect.yaml`:

```yaml
docs:
  style: markdown
  output: build/docs
```

A project manifest (`suspect.project.json`) with a `docs` section renders
the reference from the published, post-overlay spec in `suspect project
build`.

## Custom templates: `gen template`

A manifest lists output rules, each a template and a target path. It can be
`gen.yaml`, `gen.json` or `gen.toml`:

```yaml
# gen.yaml
output:
  - template: endpoints.md.j2
    target: ENDPOINTS.md
```

Templates are [minijinja](https://docs.rs/minijinja) (Jinja2 syntax), and
paths are relative to the manifest. A target path may contain `{{ }}`
expressions. The context is the spec's model: `title`, `version`, `servers`,
`operations` (`id`, `method`, `path`, `summary`, `description`, `tags`,
`deprecated`, `parameters`, `body_schema`, `responses`) and `schemas`
(`name`, `json`).

```jinja
# {{ title }} {{ version }}
{% for op in operations %}
- `{{ op.method }} {{ op.path }}`{% if op.id %} ({{ op.id | snake_case }}){% endif %}: {{ op.summary or "" }}
{% endfor %}
```

```console
$ suspect gen template api.yaml --manifest gen.yaml -o out
$ suspect gen template api.yaml --manifest gen.yaml -o out --check   # exit 1 when out/ is stale
$ suspect gen template api.yaml --manifest gen.yaml -o out --check --diff   # also print the diffs
```

Built-in filters: case conversion (`snake_case`, `camelCase`, `PascalCase`,
`kebab-case`/`kebab_case`, `CONSTANT_CASE`/`constant_case`), type and
identifier mapping (`ts_type`, `ts_ident`, `ts_line_comment`,
`ts_doc_comment`, `rust_type`, `rust_ident`, `rust_string`,
`rust_package_name`), example synthesis (`example_of`, `scalar_example`) and
`mermaid_refs` (a Mermaid graph of `$ref`s).

### Hand-written regions and ownership

A rendered file keeps hand-written code between a `suspect:begin:user-code`
and a `suspect:end:user-code` marker (inside comments in your template).
Only files whose bytes changed are rewritten. The output root records which
generator owns each file in `.suspect-artifacts.json`, so a second generator
writing to the same root refuses to overwrite files it does not own.
`--owner NAME` names a stream that shares a root, and `--adopt-identical`
takes over unowned files only when they are byte-identical. The model is in
[artifact ownership](../reference/artifact-ownership.md).

### The `docs-md` preset

`suspect gen template api.yaml --preset docs-md -o out` renders the bundled
starter templates (`docs/api/index.md`, `docs/api/schemas.md`). It is a base
for writing your own templates; for finished reference docs use `gen docs`.

## From the editor and CI

In VS Code, `Suspect: Generate SDK, Docs or Custom Templates` runs these
commands, and `Suspect: Preview Generated Docs` previews the reference. The
GitHub Action's `gen` command runs `suspect gen template` with `preset` or
`manifest`, `out-dir` and `diff` (on by default: print diffs, write nothing).
