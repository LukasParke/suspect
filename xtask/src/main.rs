//! Workspace automation: corpus fetch, fixture generation, benches.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};

const USAGE: &str = "\
tasks:
  fetch-corpus [--force]          Download real-world OpenAPI specs into corpus/ (gitignored).
  gen-fixtures [options]          Generate deterministic synthetic OpenAPI 3.1 fixtures.
    --paths N                     Number of path items (default 100).
    --schemas N                   Number of component schemas (default 100).
    --format json|yaml            Output format (default json).
    --circular                    Last schema refs the first; every schema gets a
                                  self-recursive optional `child` property.
    --out FILE                    Output file (default fixtures/generated_<N>x<M>.<ext>).
    --seed S                      PRNG seed, splitmix64 (default 0x5EED).
    --all                         Generate the standard bench set into fixtures/.
  -h, --help                      Show this help.";

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let task = args.first().map(String::as_str).unwrap_or_default();
    let rest = &args[1.min(args.len())..];
    match task {
        "fetch-corpus" => fetch_corpus(rest),
        "gen-fixtures" => gen_fixtures(rest),
        "-h" | "--help" | "help" => {
            println!("{USAGE}");
            Ok(())
        }
        _ => {
            eprintln!("unknown task: {task:?}\n\n{USAGE}");
            std::process::exit(2);
        }
    }
}

// ---------------------------------------------------------------------------
// fetch-corpus
// ---------------------------------------------------------------------------

struct CorpusEntry {
    name: &'static str,
    /// Primary URL, then fallbacks tried in order until one succeeds.
    urls: &'static [&'static str],
}

const CORPUS_DIR: &str = "corpus";

const CORPUS: &[CorpusEntry] = &[
    CorpusEntry {
        name: "petstore-expanded.yaml",
        urls: &[
            "https://raw.githubusercontent.com/OAI/OpenAPI-Specification/main/_archive_/schemas/v3.0/pass/petstore-expanded.yaml",
        ],
    },
    CorpusEntry {
        name: "api.github.com.yaml",
        urls: &[
            "https://raw.githubusercontent.com/github/rest-api-description/main/descriptions/api.github.com/api.github.com.yaml",
        ],
    },
    CorpusEntry {
        name: "digitalocean.yaml",
        urls: &[
            "https://raw.githubusercontent.com/digitalocean/openapi/main/specification/api.openapi.yaml",
            "https://raw.githubusercontent.com/digitalocean/openapi/main/specification/DigitalOcean-public.v2.yaml",
        ],
    },
    CorpusEntry {
        name: "kubernetes-swagger.yaml",
        urls: &[
            // Historical location; upstream now ships JSON under the same dir.
            "https://raw.githubusercontent.com/kubernetes/kubernetes/master/api/openapi-spec/swagger.yaml",
            "https://raw.githubusercontent.com/kubernetes/kubernetes/master/api/openapi-spec/swagger.json",
            "https://raw.githubusercontent.com/kubernetes/kubernetes/master/api/discovery/k8s.yaml",
        ],
    },
    CorpusEntry {
        name: "stripe.yaml",
        urls: &["https://raw.githubusercontent.com/stripe/openapi/master/openapi/spec3.yaml"],
    },
    CorpusEntry {
        name: "stripe-sdk.yaml",
        urls: &["https://raw.githubusercontent.com/stripe/openapi/master/openapi/spec3.sdk.yaml"],
    },
    CorpusEntry {
        name: "gitlab.yaml",
        urls: &["https://gitlab.com/gitlab-org/gitlab/-/raw/master/openapi/openapi.v3.yaml"],
    },
];

enum FetchOutcome {
    Ok(u64),
    Fail(String),
}

fn fetch_corpus(args: &[String]) -> Result<()> {
    let mut force = false;
    for a in args {
        match a.as_str() {
            "--force" => force = true,
            "-h" | "--help" => {
                println!("{USAGE}");
                return Ok(());
            }
            other => bail!("fetch-corpus: unknown flag {other:?}"),
        }
    }

    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(60)))
        .build()
        .new_agent();

    std::fs::create_dir_all(CORPUS_DIR).with_context(|| format!("create {CORPUS_DIR}/"))?;

    let mut ok = 0usize;
    let mut skip = 0usize;
    let mut fail = 0usize;

    println!("{:<26} {:>12}  status", "name", "bytes");
    println!("{}", "-".repeat(60));

    for entry in CORPUS {
        let dest = Path::new(CORPUS_DIR).join(entry.name);
        if !force && dest.is_file() {
            let bytes = std::fs::metadata(&dest).map(|m| m.len()).unwrap_or(0);
            println!("{:<26} {:>12}  skip", entry.name, human_bytes(bytes));
            skip += 1;
            continue;
        }

        let mut outcome: Option<FetchOutcome> = None;
        for url in entry.urls {
            match download(&agent, url) {
                Ok(bytes) if bytes.is_empty() => {
                    outcome = Some(FetchOutcome::Fail("empty body".into()));
                }
                Ok(bytes) => {
                    if let Err(e) = std::fs::write(&dest, &bytes) {
                        outcome =
                            Some(FetchOutcome::Fail(format!("write {}: {e}", dest.display())));
                    } else {
                        outcome = Some(FetchOutcome::Ok(bytes.len() as u64));
                    }
                    break;
                }
                Err(e) => {
                    outcome = Some(FetchOutcome::Fail(e.to_string()));
                }
            }
        }

        match outcome.unwrap_or_else(|| FetchOutcome::Fail("no urls".into())) {
            FetchOutcome::Ok(bytes) => {
                println!("{:<26} {:>12}  ok", entry.name, human_bytes(bytes));
                ok += 1;
            }
            FetchOutcome::Fail(reason) => {
                eprintln!("warn: {} failed ({reason:#})", entry.name);
                println!("{:<26} {:>12}  FAIL", entry.name, "-");
                fail += 1;
            }
        }
    }

    println!("\n{} ok, {skip} skipped, {fail} failed", ok);

    if ok + skip == 0 && fail > 0 {
        bail!("every corpus download failed");
    }
    Ok(())
}

fn download(agent: &ureq::Agent, url: &str) -> Result<Vec<u8>> {
    let mut resp = agent
        .get(url)
        .header("User-Agent", "suspect-xtask/0.1")
        .call()
        .context(format!("GET {url}"))?;
    let status = resp.status().as_u16();
    if !(200..300).contains(&status) {
        bail!("GET {url}: HTTP {status}");
    }
    let bytes = resp
        .body_mut()
        .read_to_vec()
        .with_context(|| format!("read body of {url}"))?;
    Ok(bytes)
}

/// Compact byte count for table display (`15.3M` style).
fn human_bytes(n: u64) -> String {
    const UNITS: [(u64, &str); 3] = [(1 << 30, "G"), (1 << 20, "M"), (1 << 10, "K")];
    for (div, suffix) in UNITS {
        if n >= div {
            return format!("{:.1}{suffix}", n as f64 / div as f64);
        }
    }
    format!("{n}")
}
// ---------------------------------------------------------------------------
// gen-fixtures — deterministic synthetic OpenAPI 3.1 generator
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Format {
    Json,
    Yaml,
}

impl Format {
    fn ext(self) -> &'static str {
        match self {
            Format::Json => "json",
            Format::Yaml => "yaml",
        }
    }
}

#[derive(Clone)]
struct GenOptions {
    paths: usize,
    schemas: usize,
    format: Format,
    circular: bool,
    seed: u64,
}

impl Default for GenOptions {
    fn default() -> Self {
        GenOptions {
            paths: 100,
            schemas: 100,
            format: Format::Json,
            circular: false,
            seed: 0x5EED,
        }
    }
}

/// splitmix64 — small, deterministic, dependency-free PRNG.
struct SplitMix64(u64);

impl SplitMix64 {
    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform integer in `0..bound` (`bound > 0`) via modulo on fresh bits.
    fn below(&mut self, bound: u64) -> u64 {
        self.next_u64() % bound
    }

    /// Picks from a fixed word list — used to vary description text.
    fn word(&mut self) -> &'static str {
        const WORDS: &[&str] = &[
            "alpha", "bravo", "delta", "echo", "kilo", "nova", "orbit", "pulse", "quartz", "raven",
            "sigma", "tango", "umbra", "vortex", "willow", "zephyr",
        ];
        WORDS[self.below(WORDS.len() as u64) as usize]
    }
}

/// Minimal ordered JSON/YAML value tree for emission.
enum Node {
    Str(String),
    Int(i64),
    Bool(bool),
    Map(Vec<(String, Node)>),
    Seq(Vec<Node>),
}

impl Node {
    fn map(entries: Vec<(&str, Node)>) -> Node {
        Node::Map(
            entries
                .into_iter()
                .map(|(k, v)| (k.to_string(), v))
                .collect(),
        )
    }
}

fn schema_ref(name: &str) -> Node {
    Node::Map(vec![(
        "$ref".to_string(),
        Node::Str(format!("#/components/schemas/{name}")),
    )])
}

fn schema_name(i: usize) -> String {
    format!("Item{i}")
}

/// Builds the full spec document as an ordered tree.
fn build_spec(opts: &GenOptions, rng: &mut SplitMix64) -> Node {
    let n_paths = opts.paths.max(1);
    let n_schemas = opts.schemas.max(1);
    let n_tags = n_paths.clamp(1, 8);

    // Pre-draw rng values in a fixed order so output depends only on the seed.
    let tag_words: Vec<&str> = (0..n_tags).map(|_| rng.word()).collect();
    let prop_counts: Vec<u64> = (0..n_schemas).map(|_| rng.below(4) + 2).collect();
    let prop_words: Vec<Vec<&str>> = (0..n_schemas)
        .map(|i| (0..prop_counts[i]).map(|_| rng.word()).collect())
        .collect();
    let example_ints: Vec<i64> = (0..n_schemas * 8)
        .map(|_| rng.below(10_000) as i64)
        .collect();
    let title_word = rng.word();

    // --- component schemas -------------------------------------------------
    let mut schemas: Vec<(String, Node)> = Vec::with_capacity(n_schemas);
    for i in 0..n_schemas {
        let name = schema_name(i);
        let words = &prop_words[i];

        let mut props: Vec<(String, Node)> = vec![
            ("id".to_string(), prop_schema("string")),
            ("count".to_string(), prop_schema("integer")),
            ("enabled".to_string(), prop_schema("boolean")),
        ];
        for (j, w) in words.iter().enumerate() {
            props.push((
                format!("{w}_{j}"),
                Node::Map(vec![
                    ("type".to_string(), Node::Str("string".to_string())),
                    (
                        "maxLength".to_string(),
                        Node::Int(example_ints[(i * 8 + j % 8) % example_ints.len()]),
                    ),
                ]),
            ));
        }
        // Chain link: every schema refs the next one...
        let next = schema_name(i + 1);
        if i + 1 < n_schemas || opts.circular {
            // ...except the tail, which stops the chain unless --circular wraps
            // it back to the first schema.
            let target = if i + 1 < n_schemas {
                next
            } else {
                schema_name(0)
            };
            props.push(("next".to_string(), schema_ref(&target)));
        }
        // Self-recursive optional child property (only under --circular):
        // never listed in `required`, so it stays optional.
        if opts.circular {
            props.push(("child".to_string(), schema_ref(&name)));
        }

        let required: Vec<Node> = ["id", "count"]
            .iter()
            .map(|s| Node::Str(s.to_string()))
            .collect();

        schemas.push((
            name.clone(),
            Node::map(vec![
                ("type", Node::Str("object".to_string())),
                (
                    "description",
                    Node::Str(format!(
                        "Synthetic schema {name} seeded from {}",
                        words.join("-")
                    )),
                ),
                ("properties", Node::Map(props)),
                ("required", Node::Seq(required)),
            ]),
        ));
    }

    // --- paths --------------------------------------------------------------
    let mut path_items: Vec<(String, Node)> = Vec::with_capacity(n_paths);
    for p in 0..n_paths {
        let get_resp = schema_name(p % n_schemas);
        let post_body = schema_name((p + 1) % n_schemas);

        let parameters = Node::Seq(vec![
            Node::map(vec![
                ("name", Node::Str("id".to_string())),
                ("in", Node::Str("path".to_string())),
                ("required", Node::Bool(true)),
                ("schema", prop_schema("string")),
            ]),
            Node::map(vec![
                ("name", Node::Str("limit".to_string())),
                ("in", Node::Str("query".to_string())),
                (
                    "description",
                    Node::Str(format!("max items, tag {}", tag_words[p % n_tags])),
                ),
                ("schema", prop_schema("integer")),
            ]),
            Node::map(vec![
                ("name", Node::Str("verbose".to_string())),
                ("in", Node::Str("query".to_string())),
                ("schema", prop_schema("boolean")),
            ]),
        ]);

        let json_media = |schema: Node| {
            Node::map(vec![(
                "application/json",
                Node::map(vec![("schema", schema)]),
            )])
        };

        let get_op = Node::map(vec![
            ("operationId", Node::Str(format!("get_item_{p}"))),
            (
                "tags",
                Node::Seq(vec![Node::Str(format!("tag{}", p % n_tags))]),
            ),
            ("summary", Node::Str(format!("Fetch item {p}"))),
            ("parameters", parameters),
            (
                "responses",
                Node::map(vec![(
                    "200",
                    Node::map(vec![
                        ("description", Node::Str("the item".to_string())),
                        ("content", json_media(schema_ref(&get_resp))),
                    ]),
                )]),
            ),
        ]);

        let post_op = Node::map(vec![
            ("operationId", Node::Str(format!("create_item_{p}"))),
            (
                "tags",
                Node::Seq(vec![Node::Str(format!("tag{}", p % n_tags))]),
            ),
            ("summary", Node::Str(format!("Create item {p}"))),
            (
                "requestBody",
                Node::map(vec![
                    ("required", Node::Bool(true)),
                    ("content", json_media(schema_ref(&post_body))),
                ]),
            ),
            (
                "responses",
                Node::map(vec![(
                    "201",
                    Node::map(vec![
                        ("description", Node::Str("created".to_string())),
                        ("content", json_media(schema_ref(&get_resp))),
                    ]),
                )]),
            ),
        ]);

        path_items.push((
            format!("/items/item-{p}"),
            Node::map(vec![
                ("get", get_op),
                ("post", post_op),
                (
                    "parameters",
                    Node::Seq(vec![Node::map(vec![
                        ("name", Node::Str("id".to_string())),
                        ("in", Node::Str("path".to_string())),
                        ("required", Node::Bool(true)),
                        ("schema", prop_schema("string")),
                    ])]),
                ),
            ]),
        ));
    }

    let tags = Node::Seq(
        (0..n_tags)
            .map(|t| {
                Node::map(vec![
                    ("name", Node::Str(format!("tag{t}"))),
                    (
                        "description",
                        Node::Str(format!("group for {} resources", tag_words[t])),
                    ),
                ])
            })
            .collect(),
    );

    Node::map(vec![
        ("openapi", Node::Str("3.1.0".to_string())),
        (
            "info",
            Node::map(vec![
                (
                    "title",
                    Node::Str(format!("suspect generated fixture ({title_word})")),
                ),
                ("version", Node::Str("1.0.0".to_string())),
                (
                    "description",
                    Node::Str(format!(
                        "Deterministic synthetic spec: {} paths x {} schemas, seed 0x{:X}",
                        n_paths, n_schemas, opts.seed
                    )),
                ),
            ]),
        ),
        (
            "servers",
            Node::Seq(vec![Node::map(vec![
                ("url", Node::Str("https://api.example.com/v1".to_string())),
                ("description", Node::Str("primary".to_string())),
            ])]),
        ),
        ("tags", tags),
        ("paths", Node::Map(path_items)),
        (
            "components",
            Node::map(vec![("schemas", Node::Map(schemas))]),
        ),
    ])
}

fn prop_schema(ty: &str) -> Node {
    Node::map(vec![("type", Node::Str(ty.to_string()))])
}

/// Renders the tree as pretty JSON (2-space indent), deterministic ordering.
fn emit_json(node: &Node) -> String {
    let mut out = String::new();
    write_json(node, 0, &mut out);
    out.push('\n');
    out
}

fn write_json(node: &Node, indent: usize, out: &mut String) {
    let pad = "  ".repeat(indent);
    let pad_in = "  ".repeat(indent + 1);
    match node {
        Node::Str(s) => {
            out.push_str(&json_str(s));
        }
        Node::Int(i) => out.push_str(&i.to_string()),
        Node::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Node::Map(entries) if entries.is_empty() => out.push_str("{}"),
        Node::Map(entries) => {
            out.push_str("{\n");
            for (i, (k, v)) in entries.iter().enumerate() {
                out.push_str(&pad_in);
                out.push_str(&json_str(k));
                out.push_str(": ");
                write_json(v, indent + 1, out);
                if i + 1 < entries.len() {
                    out.push(',');
                }
                out.push('\n');
            }
            out.push_str(&pad);
            out.push('}');
        }
        Node::Seq(items) if items.is_empty() => out.push_str("[]"),
        Node::Seq(items) => {
            out.push_str("[\n");
            for (i, v) in items.iter().enumerate() {
                out.push_str(&pad_in);
                write_json(v, indent + 1, out);
                if i + 1 < items.len() {
                    out.push(',');
                }
                out.push('\n');
            }
            out.push_str(&pad);
            out.push(']');
        }
    }
}

fn json_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                out.push_str(&format!("\\u{:04x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Renders the tree as YAML. Structure stays block-style, but any subtree
/// whose single-line JSON form fits in `FLOW_MAX_BYTES` is emitted as a flow
/// collection (JSON is valid YAML flow syntax). This keeps generated documents
/// far below the ~32k-line ceiling where the vendored tree-sitter YAML grammar
/// degrades, while remaining fully deterministic and spec-valid.
fn emit_yaml(node: &Node) -> String {
    yaml_node(node, 0)
}

/// Subtrees up to this size render on one line in YAML mode.
const FLOW_MAX_BYTES: usize = 4096;

/// Single-line JSON rendering of a subtree (valid YAML flow collection).
fn json_compact(node: &Node) -> String {
    let mut out = String::new();
    write_json_compact(node, &mut out);
    out
}

fn write_json_compact(node: &Node, out: &mut String) {
    match node {
        Node::Str(s) => out.push_str(&json_str(s)),
        Node::Int(i) => out.push_str(&i.to_string()),
        Node::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Node::Map(entries) => {
            out.push('{');
            for (i, (k, v)) in entries.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push_str(&json_str(k));
                out.push(':');
                write_json_compact(v, out);
            }
            out.push('}');
        }
        Node::Seq(items) => {
            out.push('[');
            for (i, v) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_json_compact(v, out);
            }
            out.push(']');
        }
    }
}

fn yaml_indent(indent: usize) -> String {
    "  ".repeat(indent)
}

fn yaml_scalar(node: &Node) -> Option<String> {
    match node {
        Node::Str(s) => Some(json_str(s)), // JSON string escapes are valid YAML d-quotes
        Node::Int(i) => Some(i.to_string()),
        Node::Bool(b) => Some(if *b { "true" } else { "false" }.to_string()),
        _ => None,
    }
}

fn yaml_node(node: &Node, indent: usize) -> String {
    let pad = yaml_indent(indent);
    match node {
        Node::Str(s) => format!("{}\n", json_str(s)),
        Node::Int(i) => format!("{i}\n"),
        Node::Bool(b) => format!("{b}\n"),
        Node::Map(entries) if entries.is_empty() => "{}\n".to_string(),
        Node::Map(entries) => {
            let mut out = String::new();
            for (k, v) in entries {
                let key = json_str(k); // quoted key is valid YAML
                if let Some(s) = yaml_scalar(v) {
                    out.push_str(&format!("{pad}{key}: {s}\n"));
                } else if let Node::Map(m) = v {
                    if m.is_empty() {
                        out.push_str(&format!("{pad}{key}: {{}}\n"));
                        continue;
                    }
                } else if let Node::Seq(s) = v
                    && s.is_empty()
                {
                    out.push_str(&format!("{pad}{key}: []\n"));
                    continue;
                }
                let compact = json_compact(v);
                if compact.len() <= FLOW_MAX_BYTES {
                    out.push_str(&format!("{pad}{key}: {compact}\n"));
                } else {
                    out.push_str(&format!("{pad}{key}:\n"));
                    out.push_str(&yaml_node(v, indent + 1));
                }
            }
            out
        }
        Node::Seq(items) if items.is_empty() => "[]\n".to_string(),
        Node::Seq(items) => {
            let mut out = String::new();
            for item in items {
                match item {
                    item if let Some(s) = yaml_scalar(item) => {
                        out.push_str(&format!("{pad}- {s}\n"));
                    }
                    _ => {
                        let compact = json_compact(item);
                        if compact.len() <= FLOW_MAX_BYTES {
                            out.push_str(&format!("{pad}- {compact}\n"));
                        } else {
                            // Render the child at indent+1 and splice `- ` onto
                            // its first line; later lines keep indent+1 spaces.
                            let inner = yaml_node(item, indent + 1);
                            out.push_str(&format!("{pad}- {}", &inner[indent * 2 + 2..]));
                        }
                    }
                }
            }
            out
        }
    }
}
fn generate_spec(opts: &GenOptions) -> Vec<u8> {
    let mut rng = SplitMix64(opts.seed);
    let doc = build_spec(opts, &mut rng);
    let text = match opts.format {
        Format::Json => emit_json(&doc),
        Format::Yaml => emit_yaml(&doc),
    };
    text.into_bytes()
}

fn gen_fixtures(args: &[String]) -> Result<()> {
    let mut opts = GenOptions::default();
    let mut out: Option<PathBuf> = None;
    let mut all = false;

    let mut it = args.iter().peekable();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--paths" => {
                opts.paths = next_value(&mut it, "--paths")?
                    .parse()
                    .context("--paths expects a number")?;
            }
            "--schemas" => {
                opts.schemas = next_value(&mut it, "--schemas")?
                    .parse()
                    .context("--schemas expects a number")?;
            }
            "--seed" => {
                let raw = next_value(&mut it, "--seed")?;
                let parsed = if let Some(hex) = raw.strip_prefix("0x") {
                    u64::from_str_radix(hex, 16)
                } else {
                    raw.parse::<u64>()
                };
                opts.seed = parsed.context("--seed expects an integer")?;
            }
            "--out" => {
                out = Some(PathBuf::from(next_value(&mut it, "--out")?));
            }
            "--format" => {
                let raw = next_value(&mut it, "--format")?;
                opts.format = match raw.as_str() {
                    "json" => Format::Json,
                    "yaml" => Format::Yaml,
                    other => bail!("--format must be json or yaml, got {other:?}"),
                };
            }
            "--circular" => opts.circular = true,
            "--all" => all = true,
            "-h" | "--help" => {
                println!("{USAGE}");
                return Ok(());
            }
            other => bail!("gen-fixtures: unknown flag {other:?}\n\n{USAGE}"),
        }
    }

    if all {
        // Standard bench set: (paths, schemas, format, circular)
        const BENCH_SET: [(usize, usize, Format, bool); 5] = [
            (100, 100, Format::Json, false),
            (1000, 1000, Format::Json, false),
            (100, 100, Format::Yaml, false),
            (1000, 1000, Format::Yaml, false),
            (2000, 2000, Format::Yaml, true),
        ];
        for (paths, schemas, format, circular) in BENCH_SET {
            let o = GenOptions {
                paths,
                schemas,
                format,
                circular,
                seed: opts.seed,
            };
            let dest = default_out_path(&o);
            write_and_validate(&o, &dest)?;
        }
        return Ok(());
    }

    anyhow::ensure!(opts.paths > 0, "--paths must be >= 1");
    anyhow::ensure!(opts.schemas > 0, "--schemas must be >= 1");
    let dest = out.unwrap_or_else(|| default_out_path(&opts));
    write_and_validate(&opts, &dest)
}
/// Returns the value following `flag`, erroring when missing.
fn next_value<'a>(
    it: &mut std::iter::Peekable<std::slice::Iter<'a, String>>,
    flag: &str,
) -> Result<&'a String> {
    it.next()
        .with_context(|| format!("{flag} requires a value"))
}

fn default_out_path(opts: &GenOptions) -> PathBuf {
    PathBuf::from(format!(
        "fixtures/generated_{}x{}.{}",
        opts.paths,
        opts.schemas,
        opts.format.ext()
    ))
}

/// Writes the generated spec to `dest`, then proves it parses with
/// suspect-low as an OpenAPI 3.1 document, printing stats.
fn write_and_validate(opts: &GenOptions, dest: &Path) -> Result<()> {
    if let Some(parent) = dest.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }

    let start = Instant::now();
    let bytes = generate_spec(opts);
    let gen_ms = start.elapsed().as_millis();

    std::fs::write(dest, &bytes).with_context(|| format!("write {}", dest.display()))?;

    let parse_start = Instant::now();
    let uri = suspect_source::Uri::from_path(dest)
        .with_context(|| format!("uri for {}", dest.display()))?;
    let doc = suspect_low::LowDoc::parse(uri, suspect_source::Source::from_vec(bytes.clone()));
    let parse_ms = parse_start.elapsed().as_millis();

    let family = doc.sniff_family();
    if family != suspect_low::SpecFamily::Oas31 {
        let errs = doc.inner().errors();
        for e in errs.iter().take(10) {
            eprintln!("  err @ {:?}: {}", e.range, e.message);
        }
        eprintln!("diagnostics: {} syntax errors total", errs.len(),);
        bail!("{}: expected Oas31, sniffed {family:?}", dest.display());
    }
    let root = doc.root();
    let paths = root.get("paths").map(|p| p.entries().len()).unwrap_or(0);
    let schemas = root
        .get("components")
        .and_then(|c| c.get("schemas"))
        .map(|s| s.entries().len())
        .unwrap_or(0);

    println!(
        "{}: {} bytes, gen {} ms, parse {} ms, family Oas31, paths={paths}, schemas={schemas}",
        dest.display(),
        bytes.len(),
        gen_ms,
        parse_ms
    );
    anyhow::ensure!(
        paths == opts.paths,
        "{}: wrote {} paths but parsed {paths}",
        dest.display(),
        opts.paths
    );
    anyhow::ensure!(
        schemas == opts.schemas,
        "{}: wrote {} schemas but parsed {schemas}",
        dest.display(),
        opts.schemas
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// tests — pure generator/PRNG logic only; no network access here.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splitmix64_sequence_matches_reference() {
        // Reference values computed with the canonical splitmix64 algorithm.
        let mut g = SplitMix64(42);
        assert_eq!(g.next_u64(), 0xbdd7_3226_2feb_6e95);
        assert_eq!(g.next_u64(), 0x28ef_e333_b266_f103);
        assert_eq!(g.next_u64(), 0x4752_6757_130f_9f52);
        assert_eq!(g.next_u64(), 0x581c_e1ff_0e4a_e394);

        let mut g = SplitMix64(0);
        assert_eq!(g.next_u64(), 0xe220_a839_7b1d_cdaf);
        assert_eq!(g.next_u64(), 0x6e78_9e6a_a1b9_65f4);
        assert_eq!(g.next_u64(), 0x06c4_5d18_8009_454f);
    }

    #[test]
    fn same_seed_identical_bytes_both_formats() {
        for format in [Format::Json, Format::Yaml] {
            let opts = GenOptions {
                paths: 25,
                schemas: 12,
                format,
                circular: true,
                seed: 7,
            };
            let a = generate_spec(&opts);
            let b = generate_spec(&opts);
            assert_eq!(a, b, "{format:?}: same seed must give identical bytes");

            let mut different_seed = opts.clone();
            different_seed.seed = 8;
            let c = generate_spec(&different_seed);
            assert_ne!(a, c, "{format:?}: different seed should change output");
        }
    }

    #[test]
    fn generated_specs_parse_as_oas31_with_expected_counts() {
        for format in [Format::Json, Format::Yaml] {
            let opts = GenOptions {
                paths: 7,
                schemas: 5,
                format,
                circular: false,
                seed: 99,
            };
            let bytes = generate_spec(&opts);
            let uri = suspect_source::Uri::from_path(Path::new(&format!(
                "fixtures/generated_test.{0}",
                format.ext()
            )))
            .unwrap();
            let doc = suspect_low::LowDoc::parse(uri, suspect_source::Source::from_vec(bytes));
            assert_eq!(doc.sniff_family(), suspect_low::SpecFamily::Oas31);
            let root = doc.root();
            assert_eq!(root.get("paths").unwrap().entries().len(), 7);
            assert_eq!(
                root.get("components")
                    .and_then(|c| c.get("schemas"))
                    .unwrap()
                    .entries()
                    .len(),
                5
            );
        }
    }

    #[test]
    fn circular_chain_wraps_and_self_recurses() {
        let opts = GenOptions {
            paths: 2,
            schemas: 3,
            format: Format::Json,
            circular: true,
            seed: 1,
        };
        let bytes = generate_spec(&opts);
        let uri =
            suspect_source::Uri::from_path(Path::new("fixtures/generated_circ.json")).unwrap();
        let doc = suspect_low::LowDoc::parse(uri, suspect_source::Source::from_vec(bytes));
        assert_eq!(doc.sniff_family(), suspect_low::SpecFamily::Oas31);
        let schemas = doc
            .root()
            .get("components")
            .and_then(|c| c.get("schemas"))
            .expect("components.schemas");

        // Last schema wraps the chain back to Item0...
        let last_next = schemas
            .get("Item2")
            .and_then(|s| s.get("properties"))
            .and_then(|p| p.get("next"))
            .and_then(|n| n.get("$ref"))
            .and_then(|r| r.as_str())
            .expect("chain link on last schema");
        assert_eq!(last_next, "#/components/schemas/Item0");

        // ...and every schema carries a self-recursive `child` property.
        for i in 0..3u8 {
            let child_ref = schemas
                .get(&format!("Item{i}"))
                .and_then(|s| s.get("properties"))
                .and_then(|p| p.get("child"))
                .and_then(|c| c.get("$ref"))
                .and_then(|r| r.as_str())
                .unwrap_or_else(|| panic!("self-recursive child missing on Item{i}"));
            assert_eq!(child_ref, format!("#/components/schemas/Item{i}"));
        }
    }

    #[test]
    fn non_circular_chain_ends_at_last_schema() {
        let opts = GenOptions {
            paths: 1,
            schemas: 3,
            format: Format::Json,
            circular: false,
            seed: 1,
        };
        let text = String::from_utf8(generate_spec(&opts)).unwrap();
        // Item2 is the tail: no `next` ref leaving it.
        assert!(text.contains("#/components/schemas/Item2"));
        assert!(
            !text.contains("\"child\""),
            "child property requires --circular"
        );
    }
}
