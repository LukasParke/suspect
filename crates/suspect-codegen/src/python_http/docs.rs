//! Source-bound documentation and executable examples for the admitted Python
//! HTTP slice. Native signatures are obtained by importing the built package at
//! documentation time; neither source text nor a second signature table is used.

use std::collections::BTreeMap;

use serde_json::{Value, json};
use suspect_ir::contract::SourceId;

use super::{HttpPlan, NativeType, PackageConfig, PlannedOperation, native_examples};
use crate::{
    OutFile,
    examples::ExampleEntry,
    http_examples,
    python_models::{PyDecl, PyType},
};

struct Symbol {
    module: String,
    name: String,
    kind: &'static str,
    file: String,
    source: Option<SourceId>,
    description: String,
    related: Vec<String>,
    members: bool,
}

impl Symbol {
    fn key(&self) -> String {
        format!("{}.{}", self.module, self.name)
    }
}

fn location(source: &SourceId) -> Value {
    json!({"document":source.document().as_str(), "pointer":source.pointer()})
}

fn q(text: &str) -> String {
    native_examples::quote(text)
}

/// Always indent source prose as literal text. In particular, an OpenAPI
/// description cannot introduce a Sphinx directive, role, substitution or HTML.
fn literal(text: &str, language: &str) -> String {
    let mut out = format!(".. code-block:: {language}\n\n");
    for line in text
        .replace(
            [
                '\r', '\u{b}', '\u{c}', '\u{1c}', '\u{1d}', '\u{1e}', '\u{85}', '\u{2028}',
                '\u{2029}',
            ],
            "\n",
        )
        .lines()
    {
        out.push_str("   ");
        out.push_str(line);
        out.push('\n');
    }
    out.push('\n');
    out
}

fn heading(text: &str, level: char) -> String {
    format!("{text}\n{}\n\n", level.to_string().repeat(text.len()))
}

fn source_text(source: &SourceId) -> String {
    format!("Source: {}#{}", source.document(), source.pointer())
}

fn comment(text: &str) -> String {
    text.replace(['\r', '\n', '\u{2028}', '\u{2029}'], " ")
}

fn type_links(ty: &PyType, plan: &HttpPlan, import: &str, links: &mut Vec<String>) {
    match ty {
        PyType::Named(source) => links.push(format!("{import}.models.{}", plan.symbols()[source])),
        PyType::Nullable(inner)
        | PyType::Optional(inner)
        | PyType::List(inner)
        | PyType::Map(inner) => type_links(inner, plan, import, links),
        PyType::Union(alternatives) => {
            for (_, ty) in alternatives {
                type_links(ty, plan, import, links);
            }
        }
        PyType::Primitive("_json.JsonNumber") => links.push(format!("{import}.JsonNumber")),
        PyType::JsonValue => links.push(format!("{import}.JsonValue")),
        _ => {}
    }
}

fn symbols(plan: &HttpPlan, package: &PackageConfig) -> Vec<Symbol> {
    let import = &package.import_name;
    let mut result = Vec::new();
    for &(module, name, description) in super::emit::ROOT_EXPORTS {
        result.push(Symbol {
            module: import.clone(),
            name: name.into(),
            kind: "runtime",
            file: format!("src/{import}/{module}.py"),
            source: None,
            description: description.into(),
            related: Vec::new(),
            members: name != "UNSET",
        });
    }
    for (module, name, description) in [
        (
            "_runtime",
            "Source",
            "Canonical source document and JSON pointer carried by HTTP diagnostics.",
        ),
        (
            "codec_runtime",
            "ModelCodec",
            "Typed codec boundary. decode/decode_value validate exact wire values and construct a native model; encode/encode_value revalidate the current mutable model. Each exported model codec is an instance specialized by its module annotation.",
        ),
        (
            "codec_runtime",
            "CodecError",
            "Source-located codec conversion, schema or finite-budget failure.",
        ),
        (
            "json_runtime",
            "JsonNumber",
            "Exact JSON number token. Decimal tokens never pass through float; integer models use Python's exact int domain.",
        ),
        (
            "json_runtime",
            "JsonLimits",
            "Finite JSON parsing and writing budgets; these are resource policy, not schema assertions.",
        ),
        (
            "json_runtime",
            "JsonError",
            "Classified exact-JSON syntax, duplicate-name, representation or resource error.",
        ),
        (
            "json_runtime",
            "JsonValue",
            "The exact JSON value domain used by codec value methods.",
        ),
        (
            "json_runtime",
            "parse_json",
            "Parse the exact JSON domain with a fresh finite budget.",
        ),
        (
            "json_runtime",
            "stringify_json",
            "Write the exact JSON domain with a fresh finite budget.",
        ),
        (
            "validation",
            "ValidationError",
            "Located schema invalidity or incomplete evaluation. Incomplete evaluation is never validation success.",
        ),
        (
            "validation",
            "ValidationSession",
            "Low-level validation session over the embedded checked schema program.",
        ),
        (
            "validation",
            "validate",
            "Low-level schema-program entry point. Prefer a model's typed codec for ordinary use.",
        ),
        (
            "validation_number",
            "Exact",
            "Low-level exact decimal arithmetic used by the checked schema program. Application model numbers use json_runtime.JsonNumber.",
        ),
        (
            "validation_number",
            "integer",
            "Low-level exact integer-token conversion used by the validation engine.",
        ),
        (
            "validation_number",
            "divisible",
            "Low-level exact divisibility check charged to the validation engine's work budget.",
        ),
    ] {
        result.push(Symbol {
            module: format!("{import}.{module}"),
            name: name.into(),
            kind: "runtime",
            file: format!("src/{import}/{module}.py"),
            source: None,
            description: description.into(),
            related: Vec::new(),
            members: true,
        });
    }
    for name in ["models", "codecs", "operations"] {
        result.push(Symbol {
            module: import.clone(),
            name: name.into(),
            kind: "module",
            file: format!("src/{import}/__init__.py"),
            source: None,
            description: "Public package namespace. Models, codecs and operation result types below have individual source bindings.".into(),
            related: Vec::new(),
            members: false,
        });
    }
    if plan.codecs().validation_program().version != suspect_schema::OwnedProgram::V1_VERSION {
        result.push(Symbol {
            module: format!("{import}.validation"), name: "ProgramError".into(), kind: "runtime",
            file: format!("src/{import}/validation_guard.py"), source: None,
            description: "A malformed or unsupported low-level compiled program. Version/profile, targets, operand shapes, source identities and pattern graphs are checked before execution; this is distinct from an invalid instance or exhausted evaluation budget.".into(),
            related: vec![format!("{import}.validation.ValidationSession")], members: true,
        });
    }
    for name in ["Unset", "UNSET"] {
        result.push(Symbol {
            module: format!("{import}.models"),
            name: name.into(),
            kind: "runtime-alias",
            file: format!("src/{import}/models.py"),
            source: None,
            description: "The same public absence type/sentinel also re-exported by the package root.".into(),
            related: vec![format!("{import}.{name}")],
            members: false,
        });
    }
    for name in [
        "SYNTAX",
        "INVALID_UTF8",
        "DUPLICATE_KEY",
        "RESOURCE_LIMIT",
        "NOT_INTEGER",
        "UNSUPPORTED_VALUE",
        "CYCLE",
    ] {
        result.push(Symbol {
            module: format!("{import}.json_runtime"),
            name: name.into(),
            kind: "runtime",
            file: format!("src/{import}/json_runtime.py"),
            source: None,
            description: "Exact JSON error kind.".into(),
            related: Vec::new(),
            members: false,
        });
    }
    for model in plan.codecs().models().symbols() {
        let name = model.name();
        let declaration = &plan.codecs().models().declarations()[model.source()];
        let mut related = vec![format!("{import}.model_codecs.{name}Codec")];
        match declaration {
            PyDecl::Alias(ty) => type_links(ty, plan, import, &mut related),
            PyDecl::Dataclass { fields, extras } => {
                for field in fields {
                    type_links(&field.ty, plan, import, &mut related);
                }
                if let Some(extra) = extras {
                    type_links(extra, plan, import, &mut related);
                }
            }
        }
        related.sort();
        related.dedup();
        related.retain(|target| target != &format!("{import}.models.{name}"));
        result.push(Symbol {
            module: format!("{import}.models"),
            name: name.into(),
            kind: if matches!(declaration, PyDecl::Dataclass { .. }) {
                "model"
            } else {
                "alias"
            },
            file: format!("src/{import}/models.py"),
            source: Some(model.source().clone()),
            description: model.description().into(),
            related,
            members: true,
        });
        result.push(Symbol {
            module: format!("{import}.model_codecs"), name: format!("{name}Codec"), kind: "codec",
            file: format!("src/{import}/model_codecs.py"), source: Some(model.source().clone()),
            description: "Public import: from the package import codecs. This registry entry validates the linked model's own source schema on decode and on mutable encode.".into(),
            related: vec![format!("{import}.models.{name}"), format!("{import}.codec_runtime.ModelCodec")], members: false,
        });
        if let PyDecl::Dataclass { fields, extras } = declaration {
            for field in fields {
                let mut related = vec![format!("{import}.models.{name}")];
                type_links(&field.ty, plan, import, &mut related);
                related.sort();
                related.dedup();
                result.push(Symbol {
                    module: format!("{import}.models"),
                    name: format!("{name}.{}", field.name),
                    kind: "field",
                    file: format!("src/{import}/models.py"),
                    source: Some(field.source.clone()),
                    description: format!(
                        "Wire property: {}. Required: {}. Fixed constructor field: {}.\n{}",
                        field.wire,
                        field.required,
                        field.fixed.is_some(),
                        plan.contract()
                            .source(&field.source)
                            .and_then(|raw| raw.get("description"))
                            .and_then(Value::as_str)
                            .unwrap_or("")
                    ),
                    related,
                    members: false,
                });
            }
            if let Some(extra) = extras {
                let mut related = vec![format!("{import}.models.{name}")];
                type_links(extra, plan, import, &mut related);
                related.sort();
                related.dedup();
                for member in ["set_extra", "extra_fields"] {
                    result.push(Symbol {
                        module: format!("{import}.models"), name: format!("{name}.{member}"), kind: "extra-properties",
                        file: format!("src/{import}/models.py"), source: Some(model.source().clone()),
                        description: "Access undeclared wire properties without shadowing a declared property. The source additionalProperties policy is validated on encoding.".into(),
                        related: related.clone(), members: false,
                    });
                }
            }
        }
    }
    for group in plan.groups() {
        let mut related = Vec::new();
        for field in &group.fields {
            native_links(&field.ty, plan, import, &mut related);
        }
        if let Some(ty) = &group.part_value {
            native_links(ty, plan, import, &mut related);
        }
        if let Some(name) = &group.header_group {
            related.push(format!("{import}.operations.{name}"));
        }
        related.sort();
        related.dedup();
        result.push(Symbol {
            module: format!("{import}.operations"),
            name: group.name.clone(),
            kind: "http-group",
            file: format!("src/{import}/operations.py"),
            source: Some(group.source.clone()),
            description: format!(
                "Native {} values with source-bound fields and structural validation.",
                group.kind
            ),
            related,
            members: true,
        });
        for field in &group.fields {
            let mut related = vec![format!("{import}.operations.{}", group.name)];
            native_links(&field.ty, plan, import, &mut related);
            result.push(Symbol {
                module: format!("{import}.operations"),
                name: format!("{}.{}", group.name, field.name),
                kind: "http-field",
                file: format!("src/{import}/operations.py"),
                source: Some(field.source.clone()),
                description: format!(
                    "Wire name: {}. Required: {}.",
                    field.wire_name, field.required
                ),
                related,
                members: false,
            });
        }
    }
    for operation in plan.operations() {
        let mut related = vec![
            format!("{import}.operations.{}", operation.success_type),
            format!("{import}.operations.{}", operation.error_type),
        ];
        related.extend(
            operation
                .parameters()
                .iter()
                .map(|p| format!("{import}.models.{}", plan.symbols()[p.schema()])),
        );
        if let Some(body) = operation.body() {
            native_links(&body.ty, plan, import, &mut related);
        }
        for client in ["Client", "AsyncClient"] {
            result.push(Symbol {
                module: import.clone(),
                name: format!("{client}.{}", operation.snake_name),
                kind: "operation",
                file: format!("src/{import}/_client.py"),
                source: Some(operation.source.clone()),
                description: operation.description().into(),
                related: related.clone(),
                members: false,
            });
        }
        let mut aliases = std::collections::BTreeSet::new();
        for (alias, success, asynchronous) in [
            (&operation.success_type, true, false),
            (&operation.async_success_type, true, true),
            (&operation.error_type, false, false),
            (&operation.async_error_type, false, true),
        ] {
            if !aliases.insert(alias) {
                continue;
            }
            result.push(Symbol {
                module: format!("{import}.operations"),
                name: alias.clone(),
                kind: "response-union",
                file: format!("src/{import}/operations.py"),
                source: Some(operation.source.clone()),
                description: if success {
                    "Exact declared successful status alternatives."
                } else {
                    "Exact declared API-error alternatives. An empty alias is typing.Never."
                }
                .into(),
                related: operation
                    .responses()
                    .iter()
                    .filter(|r| if success { r.succeeds() } else { r.fails() })
                    .map(|r| {
                        format!(
                            "{import}.operations.{}",
                            match (success, asynchronous) {
                                (true, false) => &r.class_name,
                                (true, true) => &r.async_class_name,
                                (false, false) => &r.error_class_name,
                                (false, true) => &r.async_error_class_name,
                            }
                        )
                    })
                    .collect(),
                members: false,
            });
        }
        for response in operation.responses() {
            let wire = response.wire();
            let mut related = Vec::new();
            native_links(&response.ty, plan, import, &mut related);
            if let Some(group) = &response.header_group {
                related.push(format!("{import}.operations.{group}"));
            }
            related.sort();
            related.dedup();
            let mut names = std::collections::BTreeSet::new();
            if response.succeeds() {
                names.extend([&response.class_name, &response.async_class_name]);
            }
            if response.fails() {
                names.extend([&response.error_class_name, &response.async_error_class_name]);
            }
            for name in names {
                result.push(Symbol{module:format!("{import}.operations"),name:name.clone(),kind:"response",file:format!("src/{import}/operations.py"),source:Some(wire.source().use_site().source().clone()),description:format!("HTTP {}. Actual status, selected media, typed data and headers are retained. Stream items validate on each pull.\n{}",wire.status_key(),wire.description().value()),related:related.clone(),members:true});
            }
        }
        // Former private imports still resolve to these same public objects.
        // Keep them bound for existing coverage/compatibility consumers, but
        // direct new application code and all recipes to operations.
        for name in operation.exports() {
            let source = operation
                .responses()
                .iter()
                .find(|r| {
                    [
                        r.class_name.as_str(),
                        r.async_class_name.as_str(),
                        r.error_class_name.as_str(),
                        r.async_error_class_name.as_str(),
                    ]
                    .contains(&name)
                })
                .map_or(&operation.source, |r| r.wire.source().use_site().source());
            result.push(Symbol {
                module: format!("{import}._client"), name: name.into(), kind: "compatibility-alias",
                file: format!("src/{import}/_client.py"), source: Some(source.clone()),
                description: "Compatibility alias of the public operations symbol. Use the linked public import in application code.".into(),
                related: vec![format!("{import}.operations.{name}")], members: false,
            });
        }
    }
    result.sort_by_key(|symbol| (symbol.kind == "compatibility-alias", symbol.key()));
    result
}

fn native_links(ty: &NativeType, plan: &HttpPlan, import: &str, links: &mut Vec<String>) {
    match ty {
        NativeType::Model(source) => {
            links.push(format!("{import}.models.{}", plan.symbols()[source]));
            links.push(format!(
                "{import}.model_codecs.{}Codec",
                plan.symbols()[source]
            ));
        }
        NativeType::Group(name) => links.push(format!("{import}.operations.{name}")),
        NativeType::Part(inner) | NativeType::List(inner) => {
            native_links(inner, plan, import, links)
        }
        NativeType::Union(types) => {
            for ty in types {
                native_links(ty, plan, import, links);
            }
        }
        NativeType::Items(source) | NativeType::Stream(source) => {
            native_links(&NativeType::Model(source.clone()), plan, import, links)
        }
        NativeType::Builtin("JsonNumber" | "JsonValue") => {
            if let NativeType::Builtin(name) = ty {
                links.push(format!("{import}.{name}"));
            }
        }
        _ => {}
    }
}

/// All returned paths are generation-root-relative, including the configured
/// import package. The parent package assembler may insert these unchanged.
pub(super) fn artifacts(plan: &HttpPlan, package: &PackageConfig) -> Vec<OutFile> {
    let symbols = symbols(plan, package);
    let labels: BTreeMap<_, _> = symbols
        .iter()
        .enumerate()
        .map(|(i, symbol)| (symbol.key(), format!("python-symbol-{i}")))
        .collect();
    let mut api = heading("Public Python symbols", '=');
    api.push_str("Signatures, annotations and docstrings below are read from the importable package. Source prose is displayed literally. Model aliases retain their own source binding even when Python represents two aliases with the same runtime object.\n\n");
    for symbol in &symbols {
        api.push_str(&format!(".. _{}:\n\n", labels[&symbol.key()]));
        api.push_str(&heading(&format!("``{}``", symbol.key()), '-'));
        if let Some(source) = &symbol.source {
            api.push_str(&literal(&source_text(source), "text"));
        }
        if !symbol.description.is_empty() {
            api.push_str(&literal(&symbol.description, "text"));
        }
        if matches!(symbol.kind, "model" | "alias" | "field")
            && let Some(raw) = symbol
                .source
                .as_ref()
                .and_then(|source| plan.contract().source(source))
        {
            api.push_str("Original schema (annotations are not inferred behavior):\n\n");
            api.push_str(&literal(
                &serde_json::to_string_pretty(raw).unwrap(),
                "json",
            ));
        }
        api.push_str(&format!(
            ".. native-python:: {} {}\n{}\n",
            symbol.module,
            symbol.name,
            if symbol.members { "   :members:\n" } else { "" }
        ));
        if !symbol.related.is_empty() {
            api.push_str("Related: ");
            api.push_str(
                &symbol
                    .related
                    .iter()
                    .map(|key| format!(":ref:`{}`", labels[key]))
                    .collect::<Vec<_>>()
                    .join(", "),
            );
            api.push_str(".\n\n");
        }
    }
    let import = &package.import_name;
    let mut operations = heading("Operation reference", '=');
    operations.push_str("Each method uses keyword-only inputs and returns its declared success type. Concrete result and exception classes are public in the operations module. Detailed source and wire-slot metadata is available in the downloadable source bindings.\n\n");
    for operation in plan.operations() {
        operations.push_str(&heading(&format!("``{}``", operation.snake_name), '-'));
        operations.push_str(&format!(
            "Native calls: :ref:`{}` and :ref:`{}`.\n\n",
            labels[&format!("{}.Client.{}", package.import_name, operation.snake_name)],
            labels[&format!(
                "{}.AsyncClient.{}",
                package.import_name, operation.snake_name
            )]
        ));
        operations.push_str(&literal(
            &format!(
                "{} {}\n{}",
                operation.wire().method().as_str(),
                operation.wire().path(),
                source_text(&operation.source)
            ),
            "text",
        ));
        if !operation.description().is_empty() {
            operations.push_str(&literal(operation.description(), "text"));
        }
        operations.push_str("Inputs\n~~~~~~\n\n");
        for parameter in operation.parameters() {
            operations.push_str(&format!(
                "* ``{}``: :ref:`{}`; {}.\n",
                parameter.name,
                labels[&format!("{import}.models.{}", plan.symbols()[parameter.schema()])],
                if parameter.wire().required() {
                    "required"
                } else {
                    "optional, defaults to UNSET"
                }
            ));
        }
        if let Some(body) = operation.body() {
            operations.push_str(&format!(
                "* ``body``: ``{}``; {}.\n",
                body.ty.render(plan.symbols(), true, false),
                if body.required {
                    "required JSON body"
                } else {
                    "optional JSON body, defaults to UNSET"
                }
            ));
        } else if operation.parameters().is_empty() {
            operations.push_str("No input arguments.\n");
        }
        operations.push_str("\nResponses\n~~~~~~~~~\n\n");
        for response in operation.responses() {
            operations.push_str(&format!(
                "* HTTP {}: :ref:`{}` — {}.\n",
                response.wire().status_key(),
                labels[&format!("{import}.operations.{}", response.class_name)],
                if response.succeeds() {
                    "returned on an actual successful status; default can also raise an API error"
                } else {
                    "raised with validated API-error data"
                }
            ));
        }
        operations.push('\n');
    }
    let mut renderer = native_examples::Renderer::new(plan);
    let examples = plan_examples(plan, &mut renderer);
    let guides = guides(plan, package, &examples, &mut renderer);
    let validated = validated_examples(plan, package, &examples, guides.presence.as_ref());
    let mut bindings = json!({
        "format":"suspect-python-docs-v1", "package":package.name, "version":package.version, "import":package.import_name,
        "signatureAuthority":"imported Python objects (inspect.signature and annotations)",
        "exampleAvailability":"available means required source-valid slot values exist; nativeAvailable additionally requires bounded native constructions for bound inputs",
        "nativeExampleAuthority":"accepted ExamplePlan values and PyDecl/PyType descriptors; first valid union branch under the codec schema configuration",
        "nativeExampleVerification":"examples/validated.py compares UTF-8 encode bytes of each native construction with its codec-decoded source example",
        "operationsModule":format!("{import}.operations"),
        "symbols":symbols.iter().map(|symbol| json!({
            "name":symbol.key(), "module":symbol.module, "qualname":symbol.name, "kind":symbol.kind,
            "file":symbol.file, "source":symbol.source.as_ref().map(location),
            "description":symbol.description, "related":symbol.related,
            "documentation":{"file":"docs/api.rst","anchor":labels[&symbol.key()]},
        })).collect::<Vec<_>>(),
        "operations":plan.operations().iter().map(|op| operation_binding(plan, package, op)).collect::<Vec<_>>(),
        "examples":examples.records,
        "recipes":guides.records,
    });
    if let Some(policy) = plan.credential_env() {
        bindings["credentialEnv"] = json!(policy);
    }
    let index = format!(
        "{}Native keyword-only models, reusable sync/async clients and source-bound JSON codecs. Start with a complete request, then explore the task guides and native reference.\n\n.. toctree::\n   :maxdepth: 1\n\n{}   operations\n   api\n   examples\n\nBuild documentation\n-------------------\n\nInstall the generated package and the pinned docs/requirements.txt, then run::\n\n   python -m sphinx -W --keep-going -b html docs docs/_build/html\n\n:download:`Machine-readable source bindings <source-bindings.json>`\n",
        heading(&package.name, '='),
        guides
            .recipes
            .iter()
            .map(|recipe| format!("   {}\n", recipe.slug))
            .collect::<String>()
    );
    let example_docs = format!(
        "{}The shared ExamplePlan validates declared examples first and labels synthesized values explicitly. Bindings use canonical parameter/media containers, so equal wire names in different slots do not collide. Missing required examples and unsupported or over-budget native constructions are recorded separately in source-bindings.json.\n\nThe script keeps the source codec decode/encode/decode round trip and also compares UTF-8 encode bytes from every available native construction with its codec-decoded example. Native callsites use those same constructors and literals. Presence recipes check their derived variants separately. By default the script performs no HTTP calls::\n\n   python examples/validated.py\n   python -m mypy --strict examples\n\nTo execute all available native sync or async calls against an explicit fixture, supply its URL and bearer token::\n\n   python examples/validated.py --server-url http://127.0.0.1:8080/api/v1 --token fixture-token\n   python examples/validated.py --server-url http://127.0.0.1:8080/api/v1 --token fixture-token --async-client\n\nHTTP calls expect declared successful responses; API errors propagate normally. Schema and native example availability are recorded facts, not a claim about arbitrary service responses.\n\n.. literalinclude:: ../examples/validated.py\n   :language: python\n\n{}:download:`ExamplePlan manifest <../src/{import}/examples.json>`\n\n.. literalinclude:: ../src/{import}/examples.json\n   :language: json\n",
        heading("Executable examples", '='),
        heading("Values, origins and located findings", '-')
    );
    let mut files = vec![
        OutFile {
            path: "python/docs/conf.py".into(),
            content: format!(
                "PROJECT = {}\nIMPORT = {}\nVERSION = {}\n{SPHINX}\n",
                q(&package.name),
                q(import),
                q(&package.version)
            ),
        },
        OutFile {
            path: "python/docs/requirements.txt".into(),
            content: "Sphinx==8.2.3\n".into(),
        },
        OutFile {
            path: "python/docs/index.rst".into(),
            content: index,
        },
        OutFile {
            path: "python/docs/api.rst".into(),
            content: api,
        },
        OutFile {
            path: "python/docs/operations.rst".into(),
            content: operations,
        },
        OutFile {
            path: "python/docs/examples.rst".into(),
            content: example_docs,
        },
        OutFile {
            path: "python/docs/source-bindings.json".into(),
            content: format!("{}\n", serde_json::to_string_pretty(&bindings).unwrap()),
        },
        OutFile {
            path: "python/examples/validated.py".into(),
            content: validated,
        },
    ];
    let mut readme = format!(
        "# {}\n\nA Python 3.11+ SDK with native dataclasses, synchronous and asynchronous clients, and exact source-bound validation. The selected operations are documented in the [operation reference](docs/operations.rst).\n\n",
        package.name
    );
    for recipe in &guides.recipes {
        let mut page = format!("{}{}\n\n", heading(recipe.title, '='), recipe.prose);
        readme.push_str(&format!("## {}\n\n{}\n\n", recipe.title, recipe.prose));
        if let Some(example) = recipe.example {
            page.push_str(&format!(
                ".. literalinclude:: ../examples/{example}\n   :language: python\n\n"
            ));
            readme.push_str(&format!("```python\n{}```\n\n", recipe.code));
            files.push(OutFile {
                path: format!("python/examples/{example}"),
                content: recipe.code.clone(),
            });
        }
        if recipe.slug == "authentication" {
            let security = serde_json::to_string_pretty(&guides.records["authentication"]).unwrap();
            page.push_str(&literal(&security, "json"));
            readme.push_str(&format!("```json\n{security}\n```\n\n"));
        }
        if recipe.slug == "environment-credentials" {
            let policy =
                serde_json::to_string_pretty(plan.credential_env().expect("configured recipe"))
                    .unwrap();
            page.push_str(&literal(&policy, "json"));
            readme.push_str(&format!("```json\n{policy}\n```\n\n"));
        }
        if recipe.slug == "getting-started"
            && let Some(description) = guides.records["quickstart"]["description"].as_str()
        {
            page.push_str("Operation guidance from the source:\n\n");
            page.push_str(&literal(description, "text"));
            readme.push_str(&format!("<details><summary>Operation guidance from the source</summary>\n<pre>{}</pre>\n</details>\n\n", description.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")));
        }
        files.push(OutFile {
            path: format!("python/docs/{}.rst", recipe.slug),
            content: page,
        });
    }
    let exports = super::emit::ROOT_EXPORTS
        .iter()
        .map(|(_, name, _)| *name)
        .chain(["models", "codecs", "operations"])
        .map(|name| format!("`{name}`"))
        .collect::<Vec<_>>()
        .join(", ");
    readme.push_str(&format!("## Public imports and verification\n\nImport {exports} from `{import}`. Concrete response and error classes also support direct imports from `{import}.operations`. Source descriptions, constraints and every public signature are in the [native API reference](docs/api.rst).\n\nThe README code blocks are the same files included by the Sphinx task guides. Validate the exact native constructions and build the reference with the installed package:\n\n```sh\npython examples/validated.py\npython -m mypy --strict examples\npython -m pip install -r docs/requirements.txt\npython -m sphinx -W --keep-going -b html docs docs/_build/html\n```\n\n[Source bindings and recipe availability](docs/source-bindings.json) retain declared/synthesized origins and located unsupported constructions. [Executable examples](docs/examples.rst) describe the optional fixture HTTP run.\n"));
    files.push(OutFile {
        path: "python/README.md".into(),
        content: readme,
    });
    files
}

fn operation_binding(plan: &HttpPlan, package: &PackageConfig, op: &PlannedOperation) -> Value {
    super::emit::operation_binding(plan, package, op)
}

struct NativeEntry<'a> {
    entry: &'a ExampleEntry,
    construction: Result<native_examples::Construction, native_examples::Unavailable>,
}

struct NativeCall<'a> {
    index: usize,
    members: Vec<http_examples::BoundInput<'a>>,
}

struct Examples<'a> {
    entries: BTreeMap<(usize, usize), NativeEntry<'a>>,
    calls: Vec<NativeCall<'a>>,
    records: Vec<Value>,
}

fn example_record(entry: &ExampleEntry) -> Value {
    json!({
        "schema":location(&entry.schema), "container":location(&entry.container),
        "role":http_examples::role(&entry.role), "origin":http_examples::origin(&entry.origin),
        "declaredSource":entry.declared_source.as_ref().map(location),
        "name":entry.name, "summary":entry.summary,
    })
}

fn native_record(
    construction: &Result<native_examples::Construction, native_examples::Unavailable>,
) -> Value {
    match construction {
        Ok(value) => {
            json!({"available":true,"branches":value.branches.iter().map(location).collect::<Vec<_>>()})
        }
        Err(finding) => {
            json!({"available":false,"code":finding.code,"source":location(&finding.source),"reason":finding.message})
        }
    }
}

fn plan_examples<'a>(
    plan: &'a HttpPlan,
    renderer: &mut native_examples::Renderer<'_>,
) -> Examples<'a> {
    let mut examples = Examples {
        entries: BTreeMap::new(),
        calls: Vec::new(),
        records: Vec::new(),
    };
    let bindings = http_examples::bindings(plan.examples());
    for (op_index, op) in plan.operations().iter().enumerate() {
        let Some(binding) = bindings.get(&op.source) else {
            examples.records.push(json!({"source":location(&op.source),"method":op.snake_name,"available":false,"nativeAvailable":false,"reason":"example planning unavailable"}));
            continue;
        };
        let mut entries = Vec::new();
        for (index, entry) in binding.operation.entries.iter().enumerate() {
            let construction = renderer.render(
                &entry.schema,
                &entry.value,
                &format!("sample_{op_index}_{index}"),
            );
            let mut record = example_record(entry);
            record["entry"] = json!(index);
            record["native"] = native_record(&construction);
            if construction.is_ok() {
                record["native"]["function"] = json!(format!("native_{op_index}_{index}"));
            }
            entries.push(record);
            examples.entries.insert(
                (op_index, index),
                NativeEntry {
                    entry,
                    construction,
                },
            );
        }
        let parameters = op.parameters().iter().map(|p| http_examples::InputSlot {
            name: &p.name,
            required: p.wire().required(),
            container: p.wire().source().use_site().source().clone(),
        });
        let body = op.body().into_iter().map(|body| http_examples::InputSlot {
            name: "body",
            required: body.required,
            container: body
                .media
                .iter()
                .find(|media| {
                    binding.operation.entries.iter().any(|entry| {
                        matches!(entry.role, crate::examples::ExampleRole::RequestBody)
                            && &entry.container == media.wire.source().use_site().source()
                    })
                })
                .unwrap_or(&body.media[0])
                .wire
                .source()
                .use_site()
                .source()
                .clone(),
        });
        let Some(bound) = binding.bind(parameters.chain(body)) else {
            examples.records.push(json!({"source":location(&op.source),"method":op.snake_name,"available":false,"nativeAvailable":false,"reason":"required input has no validated example","entries":entries}));
            continue;
        };
        let native = bound.iter().all(|member| {
            examples.entries[&(op_index, member.example_index)]
                .construction
                .is_ok()
        });
        let mut record = json!({
            "source":location(&op.source),"method":op.snake_name,"available":true,"nativeAvailable":native,"entries":entries,
            "bindings":bound.iter().map(|member| json!({
                "member":member.name,"entry":member.example_index,"required":member.required,
                "container":location(&binding.operation.entries[member.example_index].container),
            })).collect::<Vec<_>>(),
        });
        if native {
            examples.calls.push(NativeCall {
                index: op_index,
                members: bound,
            });
        } else {
            record["reason"] =
                json!("a bound input has no supported native construction; see entries.native");
        }
        examples.records.push(record);
    }
    examples
}

/// A single rendering is reused in the validation function, ordinary callsites
/// and published guides. Parameters use their allocated keyword; only the body
/// needs a local variable, so source parameters cannot shadow client or token.
fn call_source(
    plan: &HttpPlan,
    examples: &Examples<'_>,
    call: &NativeCall<'_>,
    asynchronous: bool,
    body_override: Option<&str>,
) -> (String, String) {
    let mut setup = String::new();
    let mut arguments = Vec::new();
    for member in &call.members {
        let value = &examples.entries[&(call.index, member.example_index)];
        let native = value.construction.as_ref().expect("native call admission");
        if member.name == "body" {
            if plan.operations()[call.index]
                .body()
                .is_some_and(|body| body.content_type_parameter)
            {
                arguments.push(format!("content_type={}", q(&value.entry.media_type)));
            }
            if let Some(name) = body_override {
                arguments.push(format!("body={name}"));
            } else {
                setup.push_str(&native.assign(
                    "body",
                    &format!("models.{}", plan.symbols()[&value.entry.schema]),
                ));
                arguments.push("body=body".into());
            }
        } else {
            setup.push_str(&native.prelude());
            arguments.push(format!("{}={}", member.name, native.expression));
        }
    }
    let args = if arguments.is_empty() {
        String::new()
    } else {
        format!(
            "\n{}",
            native_examples::indent(
                &arguments
                    .iter()
                    .map(|argument| format!("{argument},\n"))
                    .collect::<String>(),
                4
            )
        )
    };
    (
        setup,
        format!(
            "{}client.{}({args})",
            if asynchronous { "await " } else { "" },
            plan.operations()[call.index].snake_name
        ),
    )
}

fn provenance(examples: &Examples<'_>, call: &NativeCall<'_>) -> String {
    let mut text = String::new();
    for member in &call.members {
        let entry = examples.entries[&(call.index, member.example_index)].entry;
        text.push_str(&format!(
            "# {} {} example; {}\n",
            http_examples::origin(&entry.origin),
            comment(member.name),
            comment(&source_text(
                entry.declared_source.as_ref().unwrap_or(&entry.schema)
            ))
        ));
    }
    text
}

struct PresenceVariant {
    name: &'static str,
    value: Value,
    construction: native_examples::Construction,
}

struct Presence {
    schema: SourceId,
    call_index: usize,
    entry_index: usize,
    field: String,
    wire: String,
    variants: Vec<PresenceVariant>,
}

fn plan_presence(
    plan: &HttpPlan,
    examples: &Examples<'_>,
    renderer: &mut native_examples::Renderer<'_>,
) -> Result<Presence, String> {
    let mut calls: Vec<_> = examples.calls.iter().collect();
    calls.sort_by_key(|call| plan.operations()[call.index].wire().method().as_str() != "PATCH");
    for call in calls {
        let Some(body) = call.members.iter().find(|member| member.name == "body") else {
            continue;
        };
        let entry = examples.entries[&(call.index, body.example_index)].entry;
        let Some(object) = entry.value.as_object() else {
            continue;
        };
        let Some((_, fields)) = renderer.object(&entry.schema) else {
            continue;
        };
        let mut candidates = Vec::new();
        for field in fields
            .iter()
            .filter(|field| !field.required && field.fixed.is_none())
        {
            let mut absent = entry.value.clone();
            absent.as_object_mut().unwrap().remove(&field.wire);
            if !renderer
                .valid(&entry.schema, &absent)
                .map_err(|failure| failure.message)?
            {
                continue;
            }
            let mut null = entry.value.clone();
            null.as_object_mut()
                .unwrap()
                .insert(field.wire.clone(), Value::Null);
            let nullable = renderer
                .valid(&entry.schema, &null)
                .map_err(|failure| failure.message)?;
            candidates.push((field, absent, nullable.then_some(null)));
        }
        // Prefer a source-present nullable field so the guide can show all
        // three states. Schema validation, not a spelling heuristic, proves it.
        candidates
            .sort_by_key(|(field, _, null)| (null.is_none(), !object.contains_key(&field.wire)));
        if let Some((field, absent, null)) = candidates.into_iter().next() {
            let mut values = vec![("absent", absent)];
            if let Some(null) = null {
                values.push(("explicit_null", null));
            }
            if object
                .get(&field.wire)
                .is_some_and(|value| !value.is_null())
            {
                values.push(("present", entry.value.clone()));
            }
            let mut variants = Vec::new();
            for (index, (name, value)) in values.into_iter().enumerate() {
                let construction = renderer
                    .render(&entry.schema, &value, &format!("presence_{index}"))
                    .map_err(|failure| failure.message)?;
                variants.push(PresenceVariant {
                    name,
                    value,
                    construction,
                });
            }
            return Ok(Presence {
                schema: entry.schema.clone(),
                call_index: call.index,
                entry_index: body.example_index,
                field: field.name.clone(),
                wire: field.wire.clone(),
                variants,
            });
        }
    }
    Err("no source-valid optional request field has a bounded native presence recipe".into())
}

struct Recipe {
    slug: &'static str,
    title: &'static str,
    prose: String,
    example: Option<&'static str>,
    code: String,
}

struct Guides {
    recipes: Vec<Recipe>,
    presence: Option<Presence>,
    records: Value,
}

fn auth_example(op: &PlannedOperation) -> String {
    use crate::http_protocol::CredentialHook;
    let Some(alternative) = op.wire().security().alternatives().first() else {
        return "auth={}".into();
    };
    let values = alternative
        .requirements()
        .iter()
        .map(|requirement| {
            let value = match requirement.credential() {
                CredentialHook::Basic => "BasicAuth(username=\"example-user\", password=token)",
                CredentialHook::OAuth2 { .. } | CredentialHook::OpenIdConnect { .. } => {
                    "Authorization(token)"
                }
                _ => "token",
            };
            format!("{}: {value}", q(requirement.name()))
        })
        .collect::<Vec<_>>();
    format!("auth={{{}}}", values.join(", "))
}

fn response_fixture(
    plan: &HttpPlan,
    op: &PlannedOperation,
    examples: &Examples<'_>,
    index: usize,
    entry: &ExampleEntry,
) -> Option<(String, String)> {
    let crate::examples::ExampleRole::Response { status } = entry.role else {
        return None;
    };
    let response = op
        .responses()
        .iter()
        .find(|response| response.exact_status() == Some(status))?;
    let selected = response
        .media
        .iter()
        .find(|media| media.wire.media_type().declared() == entry.media_type)?;
    let body = match selected.wire.representation() {
        crate::http_protocol::Representation::Json { .. } => {
            serde_json::to_string(&entry.value).ok()?
        }
        crate::http_protocol::Representation::Text { .. } => entry
            .value
            .as_str()
            .map(str::to_owned)
            .unwrap_or_else(|| entry.value.to_string()),
        _ => return None,
    };
    let mut headers = vec![("Content-Type".to_owned(), entry.media_type.clone())];
    for header in response.wire.headers() {
        let value=examples.entries.iter().find(|((op_index,_),value)|*op_index==index && matches!(&value.entry.role,crate::examples::ExampleRole::ResponseHeader{status:s,wire_name} if s==&status.to_string()&&wire_name==header.name())).map(|(_,value)|value.entry);
        if let Some(value) = value {
            headers.push((
                header.name().into(),
                header.serialize(&value.value).ok()?.value().into(),
            ));
        } else if header.required() {
            return None;
        }
    }
    let _ = plan;
    Some((
        format!(
            "{{{}}}",
            headers
                .iter()
                .map(|(name, value)| format!("{}: {}", q(name), q(value)))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        q(&body),
    ))
}

fn streaming_recipe(
    plan: &HttpPlan,
    package: &PackageConfig,
    examples: &Examples<'_>,
) -> Option<Recipe> {
    let call = examples.calls.iter().find(|call| {
        plan.operations()[call.index]
            .responses()
            .iter()
            .any(|response| response.succeeds() && response.ty.streaming())
    })?;
    let op = &plan.operations()[call.index];
    let import = &package.import_name;
    let mut code = format!(
        "from __future__ import annotations\n\nimport asyncio\nfrom {import} import Client, AsyncClient, SyncStream, AsyncStream, JsonNumber, models\n\n\n"
    );
    for asynchronous in [false, true] {
        let (setup, invoke) = call_source(plan, examples, call, asynchronous, None);
        code.push_str(&format!(
            "{}def {}(client: {}) -> int:\n    count = 0\n",
            if asynchronous { "async " } else { "" },
            if asynchronous {
                "count_items_async"
            } else {
                "count_items"
            },
            if asynchronous {
                "AsyncClient"
            } else {
                "Client"
            }
        ));
        let block = format!(
            "{setup}response = {invoke}\nif isinstance(response.data, {}Stream):\n    {}with response.data as items:\n        {}for item in items:\n            # Each item has passed its own source codec.\n            count += 1\n",
            if asynchronous { "Async" } else { "Sync" },
            if asynchronous { "async " } else { "" },
            if asynchronous { "async " } else { "" }
        );
        if asynchronous {
            code.push_str("    async with asyncio.timeout(30):\n");
            code.push_str(&native_examples::indent(&block, 8));
        } else {
            code.push_str(&native_examples::indent(&block, 4));
        }
        code.push_str("    return count\n\n\n");
    }
    Some(Recipe {
        slug: "streaming",
        title: "Read source-typed streams",
        prose: format!(
            "``{}`` returns a pull-driven item stream for its declared sequential media. Keep the client open while consuming it. Use ``with response.data`` or ``async with response.data`` so breaking early closes the body; explicit close/aclose and client exit also close outstanding responses. Exhaustion, item failures and cancellation close the body. Plain loop break without a surrounding context does not itself notify a Python iterator.\n\nSSE follows HTML event-stream framing: comments and unknown fields are ignored, data lines join with newline, valid id/retry updates are retained, and incomplete final events are discarded. Data is a string; JSON within data and sentinel values are not inferred. JSON lines parse each LF-delimited JSON value (optional CR before LF); blank records are rejected and the final LF is optional. Each stream has per-item and total byte bounds. The async deadline below includes item consumption and runs on the caller task.",
            op.snake_name
        ),
        example: Some("streaming.py"),
        code,
    })
}

fn part_wrapper<'a>(plan: &'a HttpPlan, ty: &NativeType) -> Option<&'a super::PlannedGroup> {
    match ty {
        NativeType::Group(name) => plan
            .groups()
            .iter()
            .find(|group| group.name == *name && group.kind == "part"),
        NativeType::List(inner) => part_wrapper(plan, inner),
        NativeType::Union(types) => types.iter().find_map(|ty| part_wrapper(plan, ty)),
        _ => None,
    }
}

fn parts_recipe(
    plan: &HttpPlan,
    package: &PackageConfig,
    examples: &Examples<'_>,
    multipart: bool,
) -> Option<Recipe> {
    use crate::examples::ExampleRole;
    use crate::http_protocol::{MediaRange, PartMultiplicity, PartRepresentation, Representation};
    'operation: for (index, op) in plan.operations().iter().enumerate() {
        let Some(body) = op.body() else {
            continue;
        };
        if !op.responses().iter().any(|response| response.succeeds()) {
            continue;
        }
        for media in &body.media {
            let NativeType::Group(group_name) = &media.ty else {
                continue;
            };
            let group = plan
                .groups()
                .iter()
                .find(|group| &group.name == group_name)?;
            if (group.kind == "multipart") != multipart {
                continue;
            }
            let rules = match media.wire.representation() {
                Representation::Form { form } => form.rules(),
                Representation::Multipart {
                    multipart: crate::http_protocol::MultipartPlan::Named { rules, .. },
                } => rules,
                _ => continue,
            };
            let mut setup = String::new();
            let mut fields = Vec::new();
            let mut inputs = Vec::new();
            for (field_index, field) in group.fields.iter().enumerate() {
                let part = field.part.as_ref()?;
                let repeated = part.multiplicity() == PartMultiplicity::RepeatedArrayItems;
                let (mut value, byte_input) =
                    if matches!(part.representation(), PartRepresentation::Binary { .. }) {
                        let name = format!("input_{field_index}_bytes");
                        inputs.push(format!(
                            "{name}: {}",
                            if repeated { "list[bytes]" } else { "bytes" }
                        ));
                        (
                            if repeated {
                                "part_bytes".into()
                            } else {
                                name.clone()
                            },
                            Some(name),
                        )
                    } else {
                        let Some(entry) = examples
                            .entries
                            .iter()
                            .find(|((op_index, _), entry)| {
                                *op_index == index
                                    && matches!(entry.entry.role, ExampleRole::RequestPart { .. })
                                    && &entry.entry.container == part.source().use_site().source()
                            })
                            .map(|(_, entry)| entry)
                        else {
                            if field.required {
                                continue 'operation;
                            } else {
                                continue;
                            }
                        };
                        let Ok(native) = &entry.construction else {
                            continue 'operation;
                        };
                        setup.push_str(&native.prelude());
                        (native.expression.clone(), None)
                    };
                let wrapper = part_wrapper(plan, &field.ty);
                let mut options = Vec::new();
                if let Some(wrapper) = wrapper {
                    let header_group = plan
                        .groups()
                        .iter()
                        .find(|group| Some(&group.name) == wrapper.header_group.as_ref())?;
                    let mut headers = Vec::new();
                    for header in &header_group.headers {
                        let entry = examples
                            .entries
                            .iter()
                            .find(|((op_index, _), entry)| {
                                *op_index == index
                                    && matches!(
                                        entry.entry.role,
                                        ExampleRole::RequestPartHeader { .. }
                                    )
                                    && &entry.entry.container
                                        == header.wire.source().use_site().source()
                            })
                            .map(|(_, entry)| entry);
                        if let Some(entry) = entry {
                            let Ok(native) = &entry.construction else {
                                continue 'operation;
                            };
                            setup.push_str(&native.prelude());
                            headers.push(format!("{}={}", header.name, native.expression));
                        } else if header.wire.required() {
                            continue 'operation;
                        }
                    }
                    options.push(format!(
                        "headers=operations.{}({})",
                        header_group.name,
                        headers.join(", ")
                    ));
                }
                if part.content_types().len() > 1
                    || part
                        .content_types()
                        .iter()
                        .any(|ty| !matches!(ty.range(), MediaRange::Concrete { .. }))
                {
                    if let Some(concrete) = part
                        .content_types()
                        .iter()
                        .find(|ty| matches!(ty.range(), MediaRange::Concrete { .. }))
                    {
                        options.push(format!("content_type={}", q(concrete.declared())));
                    } else {
                        let name = format!("input_{field_index}_content_type");
                        inputs.push(format!("{name}: str"));
                        options.push(format!("content_type={name}"));
                    }
                }
                if !options.is_empty() {
                    value = format!(
                        "{}(value={value}, {})",
                        wrapper.map_or("Part".into(), |wrapper| format!(
                            "operations.{}",
                            wrapper.name
                        )),
                        options.join(", ")
                    );
                }
                if repeated {
                    if let Some(input) = byte_input {
                        value = format!("[{value} for part_bytes in {input}]");
                    } else {
                        let count = part.min_items().map_or(1, |n| *n.value().max(&1));
                        if count > 8 {
                            continue 'operation;
                        }
                        value = format!("[{}]", vec![value; count as usize].join(", "));
                    }
                }
                fields.push(format!("{}={value}", field.name));
            }
            if rules.required().iter().any(|required| {
                !group
                    .fields
                    .iter()
                    .any(|field| &field.wire_name == required.value())
            }) || rules
                .min_properties()
                .is_some_and(|value| *value.value() > fields.len() as u64)
                || rules
                    .max_properties()
                    .is_some_and(|value| *value.value() < fields.len() as u64)
            {
                continue;
            }
            let mut arguments = Vec::new();
            for parameter in op.parameters() {
                let entry = examples
                    .entries
                    .iter()
                    .find(|((op_index, _), entry)| {
                        *op_index == index
                            && matches!(entry.entry.role, ExampleRole::Parameter { .. })
                            && &entry.entry.container == parameter.wire.source().use_site().source()
                    })
                    .map(|(_, entry)| entry);
                if let Some(entry) = entry {
                    let Ok(native) = &entry.construction else {
                        continue 'operation;
                    };
                    setup.push_str(&native.prelude());
                    arguments.push(format!("{}={}", parameter.name, native.expression));
                } else if parameter.wire.required() {
                    continue 'operation;
                }
            }
            arguments.push("body=body".into());
            if body.content_type_parameter {
                arguments.push(format!(
                    "content_type={}",
                    q(media.wire.media_type().declared())
                ));
            }
            let code = format!(
                "from __future__ import annotations\n\nfrom {} import Client, JsonNumber, Part, models, operations\n\n\ndef send_parts(client: Client{}) -> operations.{}:\n{}{}    return client.{}(\n{}    )\n",
                package.import_name,
                if inputs.is_empty() {
                    String::new()
                } else {
                    format!(", *, {}", inputs.join(", "))
                },
                op.success_type,
                native_examples::indent(&setup, 4),
                native_examples::indent(
                    &format!(
                        "body = operations.{group_name}(\n{})\n",
                        native_examples::indent(
                            &fields
                                .iter()
                                .map(|field| format!("{field},\n"))
                                .collect::<String>(),
                            4
                        )
                    ),
                    4
                ),
                op.snake_name,
                native_examples::indent(
                    &arguments
                        .iter()
                        .map(|argument| format!("{argument},\n"))
                        .collect::<String>(),
                    8
                )
            );
            return Some(Recipe {
                slug: if multipart { "request-parts" } else { "forms" },
                title: if multipart {
                    "Send typed byte parts"
                } else {
                    "Send typed forms"
                },
                prose: format!(
                    "This recipe constructs the source's ``{group_name}`` carrier for ``{}``. JSON/text parts and declared headers use accepted source examples; binary content is a caller-supplied bytes argument. Repeated parts use lists and per-item codecs. Required fields, extras and cardinalities are checked structurally, without substituting JSON null for bytes.\n\nPart values may carry a concrete content type, a filename, typed declared headers and explicit extra headers. Filename/name strings are quoted as UTF-8; controls are rejected. The SDK supplies MIME framing, a collision-checked boundary, part limits and a separate whole-body limit. A caller-provided transport remains caller-owned.",
                    op.snake_name
                ),
                example: Some(if multipart {
                    "request_parts.py"
                } else {
                    "forms.py"
                }),
                code,
            });
        }
    }
    None
}

fn guides(
    plan: &HttpPlan,
    package: &PackageConfig,
    examples: &Examples<'_>,
    renderer: &mut native_examples::Renderer<'_>,
) -> Guides {
    let import = &package.import_name;
    let mut recipes = Vec::new();
    // Prefer a complete native body construction, then a parameter/no-input
    // operation. Selection depends on accepted examples, never prose or a name
    // guessed from generated Python source.
    let mut candidates: Vec<_> = examples
        .calls
        .iter()
        .filter(|call| {
            plan.operations()[call.index]
                .responses()
                .iter()
                .any(|response| response.succeeds())
                && !plan.operations()[call.index]
                    .responses()
                    .iter()
                    .any(|response| response.ty.streaming())
        })
        .collect();
    candidates.sort_by_key(|call| plan.operations()[call.index].body().is_none());
    let first = candidates.first().copied();
    let mut quickstart_record = json!({"available":false,"reason":"no operation has both a native input example and a declared successful response"});
    if let Some(call) = first {
        let op = &plan.operations()[call.index];
        let auth = auth_example(op);
        let (setup, invoke) = call_source(plan, examples, call, false, None);
        let source = format!("{setup}return {invoke}\n");
        let imports = format!(
            "from __future__ import annotations\n\nfrom {import} import Client, JsonNumber, BasicAuth, Authorization, models, operations\n"
        );
        let quickstart = format!(
            "{imports}\n\ndef first_request(token: str) -> operations.{}:\n    with Client({auth}) as client:\n{}\n\nif __name__ == \"__main__\":\n    response = first_request(\"replace-with-your-bearer-token\")\n    print(response.status)\n",
            op.success_type,
            native_examples::indent(&source, 8)
        );
        let origins = call
            .members
            .iter()
            .map(|member| {
                http_examples::origin(
                    &examples.entries[&(call.index, member.example_index)]
                        .entry
                        .origin,
                )
            })
            .collect::<std::collections::BTreeSet<_>>();
        let origins = if origins.is_empty() {
            "This operation has no example-valued inputs.".into()
        } else {
            format!(
                "Input values are {} examples accepted by the source-schema validator. Their original locations are in the example inventory.",
                origins.into_iter().collect::<Vec<_>>().join(" and ")
            )
        };
        recipes.push(Recipe { slug: "getting-started", title: "Make your first request", prose: format!("Python 3.11 or newer is required. From the generated package directory, install with ``python -m pip install .``. To build a distributable wheel, run ``python -m build --wheel`` and install the resulting file from ``dist/``.\n\nThe first request calls ``Client.{}``. Supply a bearer token from your application. The source supplies the server URL. Construct request models with keyword arguments and read the typed response through ``response.data``; ``response.status`` and ``response.headers`` retain HTTP metadata.\n\n{origins}\n\nReuse a Client inside one ``with`` block for several calls. Its context manager closes the SDK-owned connection pool. Required inputs are keywords; model constructors and literals are the ordinary request interface.", op.snake_name), example: Some("quickstart.py"), code: quickstart });
        let (setup, invoke) = call_source(plan, examples, call, true, None);
        let asynchronous = format!(
            "from __future__ import annotations\n\nimport asyncio\nfrom {import} import AsyncClient, JsonNumber, BasicAuth, Authorization, models, operations\n\n\nasync def first_request_async(token: str) -> operations.{}:\n    async with AsyncClient({auth}) as client:\n        async with asyncio.timeout(30):\n{}\n\nif __name__ == \"__main__\":\n    response = asyncio.run(first_request_async(\"replace-with-your-bearer-token\"))\n    print(response.status)\n",
            op.async_success_type,
            native_examples::indent(&format!("{setup}return {invoke}\n"), 12)
        );
        recipes.push(Recipe { slug: "async", title: "Async calls and cancellation", prose: "Use AsyncClient with ``async with`` and await each operation. In an existing event loop, await ``first_request_async(token)`` directly. ``asyncio.run`` is the standalone script entry point.\n\nThe 30-second ``asyncio.timeout`` is application policy for the awaited call. Cancellation remains on the caller task, including body reads and cleanup. It propagates as ``asyncio.CancelledError``; timeout expiry becomes ``TimeoutError`` outside the timeout context. Synchronous codec work has separate finite budgets and runs between await points.".into(), example: Some("async_client.py"), code: asynchronous });
        let (setup, invoke) = call_source(plan, examples, call, false, None);
        let mut errors = format!(
            "from __future__ import annotations\n\nfrom {import} import (\n    ApiError, Client, CodecError, JsonError, JsonNumber, SdkError, BasicAuth, Authorization, models, operations,\n)\n\n\ndef request_with_errors(token: str) -> operations.{} | None:\n    with Client({auth}) as client:\n        try:\n{}",
            op.success_type,
            native_examples::indent(&format!("{setup}return {invoke}\n"), 12)
        );
        if let Some(response) = op.responses().iter().find(|response| response.fails()) {
            errors.push_str(&format!("        except operations.{} as error:\n            # error.data uses the source's selected native representation.\n            print(\"Declared API failure:\", error.status)\n", response.error_class_name));
        }
        errors.push_str("        except ApiError as error:\n            print(\"Other declared API failure:\", error.status)\n        except CodecError as error:\n            print(\"Invalid input:\", error.kind, error.path)\n        except JsonError as error:\n            print(\"JSON representation failure:\", error.kind)\n        except SdkError as error:\n            print(\"SDK failure:\", error.kind)\n    return None\n");
        recipes.push(Recipe { slug: "errors", title: "Handle API and SDK failures", prose: format!("Import concrete response classes from ``{import}.operations`` or use the root ``operations`` namespace. Successful statuses return frozen classes with typed data. Declared non-success statuses raise concrete subclasses of ApiError, also with validated data. Catch one concrete class when its particular status matters; catch ApiError for other declared responses.\n\nOperation ``Success`` and ``ApiError`` aliases are type annotations, including ``typing.Never`` for an empty set. A union alias is not an exception handler.\n\nInput CodecError or JsonError can occur before transport. Unexpected status/media, invalid response data, transport failures and response limits raise SdkError. Normal HTTP error formatting omits response bodies, headers and causes. Inspect ``data`` or the bounded ``capture`` and ``cause`` attributes explicitly when needed; this recipe logs only classifications."), example: Some("errors.py"), code: errors });
        let response = examples.entries.iter().find(|((op_index, _), entry)| *op_index == call.index && matches!(entry.entry.role, crate::examples::ExampleRole::Response { status } if (200..300).contains(&status)));
        if let Some(((_, response_index), response)) = response
            && let Some((response_headers, response_body)) =
                response_fixture(plan, op, examples, call.index, response.entry)
        {
            let crate::examples::ExampleRole::Response { status } = response.entry.role else {
                unreachable!()
            };
            let (setup, invoke) = call_source(plan, examples, call, false, None);
            let token = if op
                .wire()
                .security()
                .alternatives()
                .first()
                .is_some_and(|a| {
                    a.requirements().iter().any(|r| {
                        matches!(
                            r.credential(),
                            crate::http_protocol::CredentialHook::OAuth2 { .. }
                                | crate::http_protocol::CredentialHook::OpenIdConnect { .. }
                        )
                    })
                }) {
                "Example fixture-token"
            } else {
                "example-token"
            };
            let transport = format!(
                "from __future__ import annotations\n\nimport httpx\nfrom {import} import Client, JsonNumber, BasicAuth, Authorization, models, operations\n\n\ndef with_transport(token: str, transport: httpx.BaseTransport, *, server_url: str | None = None) -> operations.{}:\n    with Client(\n        {auth},\n        transport=transport,\n        server_url=server_url,\n        timeout=10.0,\n        max_response_bytes={},\n        max_capture_bytes=1024,\n    ) as client:\n{}\n\ndef mock_request() -> operations.{}:\n    def handle(request: httpx.Request) -> httpx.Response:\n        assert request.method == {}\n        return httpx.Response(\n            {status},\n            headers={response_headers},\n            content={response_body}.encode(\"utf-8\"),\n        )\n\n    # Injected transports belong to the application; close them here.\n    with httpx.MockTransport(handle) as transport:\n        return with_transport({}, transport, server_url=\"https://sdk.example.test\")\n\n\nif __name__ == \"__main__\":\n    print(mock_request().status)\n",
                op.success_type,
                plan.config.max_response_bytes.min(1024 * 1024),
                native_examples::indent(&format!("{setup}return {invoke}\n"), 8),
                op.success_type,
                q(op.wire().method().as_str()),
                q(token)
            );
            recipes.push(Recipe { slug: "transport", title: "Transports, timeouts and response limits", prose: format!("Inject an httpx BaseTransport into Client, or an AsyncBaseTransport into AsyncClient, to control TLS, proxies or instrumentation. ``httpx.MockTransport`` supplies a local fixture; the response below is a {} source example.\n\nThe client ``timeout`` sets httpx connect/read/write/pool phase timeouts. It is not a whole-operation deadline, and a custom transport must honor request timeout extensions. Use the async cancellation recipe for an awaited-call deadline.\n\nGenerated request and response ceilings are {} and {} bytes. A caller may lower the response ceiling; a larger override fails before transport. Captures have their own byte limit. Limits apply while reading even when Content-Length is absent or incorrect.\n\nSDK-created transports ignore environment proxy discovery and use zero retries. The SDK performs no redirect, cookie-persistence, decompression or pagination loop. Custom transports choose their own policy. Responses close on success, failure and cancellation. Client exit closes only SDK-owned transports; the outer transport context below closes the injected transport.", http_examples::origin(&response.entry.origin), plan.config.max_request_bytes, plan.config.max_response_bytes), example: Some("transport.py"), code: transport });
            quickstart_record["mockResponseEntry"] = json!(response_index);
        }
        quickstart_record["available"] = json!(true);
        quickstart_record.as_object_mut().unwrap().remove("reason");
        quickstart_record["source"] = location(&op.source);
        quickstart_record["method"] = json!(op.snake_name);
        quickstart_record["description"] = json!(op.description());
        quickstart_record["security"] = json!(op.wire().security());
        quickstart_record["file"] = json!("examples/quickstart.py");
        quickstart_record["inputs"] = json!(
            call.members
                .iter()
                .map(|member| json!({"member":member.name,"entry":member.example_index}))
                .collect::<Vec<_>>()
        );
    } else {
        recipes.push(Recipe { slug: "getting-started", title: "Make your first request", prose: "Install the generated package with ``python -m pip install .`` (Python 3.11+). A complete native first-request example is unavailable for this selection. See the example inventory for located missing values or unsupported native constructions and the operation reference for the actual required inputs.".into(), example: None, code: String::new() });
    }
    let auth = String::from(
        "Credentials are explicit constructor arguments keyed by the source security-scheme names. A bearer scheme accepts a token string; an API-key scheme attaches its string at the declared header, query or cookie location. BasicAuth supplies username/password and an explicit UTF-8 (default) or Latin-1 charset. OAuth/OIDC uses Authorization with a complete header, including its scheme, or a caller callback receiving CredentialRequest. The callback sees operation/scheme sources, scopes versus roles, flow URLs and discovery metadata. Async callbacks stay on the caller task. Token acquisition, refresh and discovery are application responsibilities.\n\nSecurity alternatives are OR; requirements within one alternative are AND. The first fully available alternative is selected, including an explicitly anonymous alternative. ``auth_alternative`` chooses an index explicitly. Disabled or undeclared security attaches no credentials. Conflicting attachments are rejected.\n\n``server`` selects a source server by index or declared name; ``server_variables`` overrides declared variables with enum/unknown-name checks. ``server_url`` is an explicit absolute override. Relative servers use the URL serving their source document; local files require an explicit ``document_url`` or ``server_url``. Source descriptions preserve business credential guidance; the SDK does not validate management-key permissions locally.\n\nSelected scheme and server bindings:",
    );
    let authentication: Vec<_> = plan.operations().iter().map(|op|json!({"method":op.snake_name,"security":op.wire().security(),"servers":op.wire().servers()})).collect();
    recipes.insert(
        1,
        Recipe {
            slug: "authentication",
            title: "Supply credentials and choose a server",
            prose: auth,
            example: None,
            code: String::new(),
        },
    );
    let (presence, presence_record) = match plan_presence(plan, examples, renderer) {
        Ok(presence) => {
            let model = &plan.symbols()[&presence.schema];
            let mut code = format!(
                "from __future__ import annotations\n\nfrom {import} import Client, JsonNumber, UNSET, models, codecs\n\n\ndef variants() -> tuple[{}]:\n",
                presence
                    .variants
                    .iter()
                    .map(|_| format!("models.{model}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            );
            for variant in &presence.variants {
                code.push_str(&native_examples::indent(
                    &variant
                        .construction
                        .assign(variant.name, &format!("models.{model}")),
                    4,
                ));
            }
            code.push_str(&native_examples::indent(
                &presence.variants[0]
                    .construction
                    .assign("explicit_unset", &format!("models.{model}")),
                4,
            ));
            code.push_str(&format!("    explicit_unset.{} = UNSET\n    assert absent.{} is UNSET\n    assert codecs.{model}Codec.encode(absent) == codecs.{model}Codec.encode(explicit_unset)\n", presence.field, presence.field));
            if presence
                .variants
                .iter()
                .any(|variant| variant.name == "explicit_null")
            {
                code.push_str(&format!(
                    "    assert explicit_null.{} is None\n",
                    presence.field
                ));
            }
            code.push_str(&format!("    return ({},)\n\n\ndef send_variants(client: Client) -> None:\n    for body in variants():\n", presence.variants.iter().map(|variant| variant.name).collect::<Vec<_>>().join(", ")));
            let call = examples
                .calls
                .iter()
                .find(|call| call.index == presence.call_index)
                .unwrap();
            let (setup, invoke) = call_source(plan, examples, call, false, Some("body"));
            code.push_str(&native_examples::indent(&format!("{setup}{invoke}\n"), 8));
            code.push_str("\n\nif __name__ == \"__main__\":\n    variants()\n");
            let nullable = presence
                .variants
                .iter()
                .any(|variant| variant.name == "explicit_null");
            recipes.push(Recipe { slug: "presence", title: "Distinguish omission, null and values", prose: format!("Optional model fields and optional operation arguments default to UNSET. Omit the keyword or assign UNSET to leave a wire field absent. None encodes JSON null only where the source allows it. Required nullable fields still require a value, even when that value is None. Defaults from the schema are not inserted.\n\nThis recipe uses ``models.{model}`` and its actual optional ``{}`` field. {} Each variant is checked against the source schema; the absent/null variants are derived from the accepted request example and labeled separately in source-bindings.json. The original example is retained unchanged.\n\nCall ``send_variants(client)`` to send these distinct requests. The schema establishes allowed wire states; the server's business effect is described by the API. Mutable models are checked again on every encode and before HTTP transport.", presence.field, if nullable { "The source permits explicit null here." } else { "This request field does not admit null; the recipe demonstrates its available states." }), example: Some("presence.py"), code });
            let record = json!({"available":true,"method":plan.operations()[presence.call_index].snake_name,"schema":location(&presence.schema),"baseExample":presence.entry_index,"member":presence.field,"wire":presence.wire,"variants":presence.variants.iter().map(|variant| json!({"state":variant.name,"origin":if variant.name == "present" {"source-example"} else {"derived-presence"},"value":variant.value})).collect::<Vec<_>>()});
            (Some(presence), record)
        }
        Err(reason) => {
            recipes.push(Recipe { slug: "presence", title: "Distinguish omission, null and values", prose: "Optional inputs default to UNSET; None is a distinct JSON null value and is accepted only where the source schema permits it. Source defaults are not applied.\n\nA native request presence recipe is unavailable for this selection. The reason is retained in source-bindings.json. See model field annotations in the native reference.".into(), example: None, code: String::new() });
            (None, json!({"available":false,"reason":reason}))
        }
    };
    let numbers = format!(
        "from decimal import Decimal\nfrom {import} import JsonNumber\n\n\ndef exact_numbers() -> None:\n    # Runtime value examples; these are not inferred API field defaults.\n    count: int = 42\n    amount = JsonNumber(\"9007199254740993.000000000000000001\")\n    decimal_amount = Decimal(amount.token)\n    assert decimal_amount == Decimal(\"9007199254740993.000000000000000001\")\n    assert count == 42\n    assert JsonNumber(\"1e3\").to_int() == 1000\n\n\nif __name__ == \"__main__\":\n    exact_numbers()\n"
    );
    recipes.push(Recipe { slug: "numbers", title: "Use exact numbers and native models", prose: "Integer schemas use Python int. General number schemas use JsonNumber, including integer-looking tokens; decimal and exponent spelling stays exact. Construct JsonNumber from a decimal string. ``.token`` retains the original token; ``.to_int()`` is an exact, bounded conversion that raises JsonError for a fractional value or an exceeded digit budget. The runtime examples below do not supply schema defaults.\n\n``Decimal(number.token)`` is an explicit exact interoperability step. Decimal arithmetic uses the application's decimal context. Passing a binary float would already have lost the original decimal digits. Use the model's ``codecs.<Model>Codec.encode`` for JSON serialization; it revalidates the current model and preserves optional absence, exact numbers and extras.\n\nNested objects are dataclasses, arrays are lists and literals are ordinary Python values. Required constant tag fields are initialized by their dataclass and are not constructor arguments. Open objects expose ``set_extra`` and ``extra_fields`` for undeclared wire names.".into(), example: Some("numbers_example.py"), code: numbers });
    if let Some(recipe) = streaming_recipe(plan, package, examples) {
        recipes.push(recipe);
    }
    if plan.codecs().validation_program().version == suspect_schema::OwnedProgram::V3_VERSION {
        recipes.push(Recipe {
            slug: "resource-validation", title: "Construct resource-bound exact values", example: None, code: String::new(),
            prose: "This package executes the indexed resource/dynamic v3 schema profile, including scoped applicators. Models in this closure use exact JSON carriers with source-bound codec types. Construct dictionaries, lists, None, booleans, strings, integers and JsonNumber values according to the native annotations and executable source examples. General numeric schemas use JsonNumber; binary float is outside the exact JSON carrier. Decode and encode validate the complete root under its resource context, including mutable values.\n\nEvery evaluated schema enters its indexed resource, even when the selected source is nested below the resource root. The outermost actually entered matching dynamic anchor wins; unentered candidates stay inert. Static references and pointer/empty/static-anchor fallbacks keep their indexed targets. Selected dynamic targets start fresh evaluated-location scopes. Invalid branches export no annotations, and all returns restore resource scope. Shared numeric, equality, work and depth failures remain noninvertible.\n\nPhysical SourceIds identify documents, assertions and diagnostics. Canonical resource URIs and aliases are separate metadata. The installed resource registry supplies all dynamic targets; validation performs no acquisition or URI lookup. Relative HTTP servers resolve against the effective physical retrieval document or an explicit caller document_url. A logical $self/$id never relocates the API.\n\nThe http-manifest.json validation record identifies the exact v3 version/profile. validation_program.json retains the checked resource registry, aligned node scopes and dynamic bindings. CodecError.kind and its source/path distinguish invalid values from resource exhaustion. Model-only planning retains explicit source-codec obligations.".into(),
        });
    }
    if plan.codecs().validation_program().version == suspect_schema::OwnedProgram::V2_VERSION {
        recipes.push(Recipe {
            slug: "scoped-validation", title: "Construct source-validated scoped values", example: None, code: String::new(),
            prose: "This package executes the static applicator v2 schema profile. Conditional if/then/else, dependentRequired/dependentSchemas, contains counts, patternProperties, propertyNames and unevaluated members/items run on every decode and encode, including each HTTP request and response. Native field annotations express the representable domains; the source codec enforces the remaining assertions.\n\nPattern-matched undeclared members remain in set_extra/extra_fields even when additionalProperties is false. Every matching pattern applies, and the additional-properties schema applies only to residual names. Required dependencies use actual key presence: a present None value is distinct from an omitted UNSET field. Values are never defaulted, stripped or coerced to satisfy a schema.\n\nConditional/intersection objects may use an exact JSON-value carrier or a dataclass with checked extras. Prefix/contains arrays use exact JSON-value lists where a homogeneous item annotation would lose valid values. Nullable object carriers retain None as a value. Use the native constructions in the executable examples and the per-model annotations in the API reference. Mutable carriers are checked again before sending.\n\nEach subschema starts a fresh evaluated-location scope. Successful same-instance applicators contribute the documented property/index sets; failed alternatives export none. A finite shared budget charges visits and every annotation merge candidate, including duplicates. Trials cannot hide numeric, equality, work or depth failure. CodecError.kind distinguishes invalid values from resource failures; low-level ValidationError retains the original assertion source and member/index pointer. The source bindings retain the exact v2 program version/profile.".into(),
        });
    }
    for multipart in [false, true] {
        if let Some(recipe) = parts_recipe(plan, package, examples, multipart) {
            recipes.push(recipe);
        }
    }
    if plan.credential_env().is_some() {
        recipes.push(Recipe {
            slug: "environment-credentials", title: "Use runtime environment credentials", example: None, code: String::new(),
            prose: "This package has an explicitly configured v1 environment policy. Client() and AsyncClient() snapshot the mapped process variables at construction when auth is omitted. The same constructors accept transport, server_url, document_url, server selection, timeouts and the existing resource-limit arguments. Omitted server overrides use the actual source-declared server.\n\nOnly variable names and source scheme bindings are generated. Importing the SDK does not read credential variables. Later environment changes affect newly created clients. Missing, empty or inaccessible variables remain unavailable; protected operations raise SdkError before HTTP if their security requirements cannot be satisfied. Anonymous operations remain usable. Existing OR/AND requirements and explicit auth_alternative selection determine attachment.\n\nAn explicitly supplied auth argument is authoritative for the whole mapping. Explicit None, an empty mapping, missing members and empty/null/UNSET member values are never supplemented from the environment; the existing explicit-credential validation applies. Pass auth={\"apiKey\": token} only when apiKey is the actual source scheme name. Bearer versus API-key attachment is determined by that source declaration. Environment acquisition supports strings for bearer and API-key schemes only; Basic/OAuth/OIDC structures retain their explicit interfaces.\n\nThe bound policy below records variable names and source provenance. Environment values are never present in generated code, documentation or compatibility metadata.".into(),
        });
    }
    recipes.sort_by_key(|recipe| {
        [
            "getting-started",
            "authentication",
            "environment-credentials",
            "async",
            "errors",
            "presence",
            "numbers",
            "transport",
            "forms",
            "request-parts",
            "streaming",
            "scoped-validation",
            "resource-validation",
        ]
        .iter()
        .position(|slug| *slug == recipe.slug)
    });
    let files:Vec<_>=recipes.iter().filter_map(|recipe|recipe.example.map(|file|json!({"file":format!("examples/{file}"),"page":format!("docs/{}.rst",recipe.slug),"title":recipe.title}))).collect();
    Guides {
        recipes,
        presence,
        records: json!({"quickstart":quickstart_record,"presence":presence_record,"authentication":authentication,"files":files}),
    }
}

fn validated_examples(
    plan: &HttpPlan,
    package: &PackageConfig,
    examples: &Examples<'_>,
    presence: Option<&Presence>,
) -> String {
    let import = &package.import_name;
    let mut code = format!(
        "\"\"\"Check source examples and the exact native constructions shown in the guides.\nValues, origins and unavailable constructions: docs/source-bindings.json.\n\"\"\"\nfrom __future__ import annotations\n\nimport argparse\nimport asyncio\nfrom {import} import Client, AsyncClient, JsonNumber, models, codecs, operations\nfrom numbers_example import exact_numbers\n"
    );
    if presence.is_some() {
        code.push_str("from presence import variants as presence_variants\n");
    }
    for (&(op_index, index), value) in &examples.entries {
        if let Ok(native) = &value.construction {
            code.push_str(&format!(
                "\n\ndef native_{op_index}_{index}() -> models.{}:\n{}",
                plan.symbols()[&value.entry.schema],
                native_examples::indent(&native.returning(), 4)
            ));
        }
    }
    code.push_str("\n\ndef validate() -> None:\n    exact_numbers()\n");
    for (&(op_index, index), entry) in &examples.entries {
        let model = &plan.symbols()[&entry.entry.schema];
        let value = q(&serde_json::to_string(&entry.entry.value).unwrap());
        code.push_str(&format!("    # {} example: {}\n    value_{op_index}_{index} = codecs.{model}Codec.decode({value})\n    codecs.{model}Codec.decode(codecs.{model}Codec.encode(value_{op_index}_{index}))\n", http_examples::origin(&entry.entry.origin), comment(&source_text(&entry.entry.schema))));
        if entry.construction.is_ok() {
            code.push_str(&format!("    assert codecs.{model}Codec.encode(native_{op_index}_{index}()).encode('utf-8') == codecs.{model}Codec.encode(value_{op_index}_{index}).encode('utf-8'), {}\n", q(&format!("native_{op_index}_{index} differs from the codec-decoded source example"))));
        }
    }
    if let Some(presence) = presence {
        code.push_str("    presence_values = presence_variants()\n");
        for (index, variant) in presence.variants.iter().enumerate() {
            let model = &plan.symbols()[&presence.schema];
            code.push_str(&format!("    assert codecs.{model}Codec.encode(presence_values[{index}]).encode('utf-8') == codecs.{model}Codec.encode(codecs.{model}Codec.decode({})).encode('utf-8')\n", q(&serde_json::to_string(&variant.value).unwrap())));
        }
    }
    code.push_str(&format!(
        "    print(\"validated-examples {}\")\n",
        examples.entries.len()
    ));
    for call in &examples.calls {
        let op_index = call.index;
        let op = &plan.operations()[op_index];
        for asynchronous in [false, true] {
            let prefix = if asynchronous { "async_" } else { "" };
            code.push_str(&format!(
                "\n\n{}def {prefix}call_{op_index}(client: {}) -> operations.{}:\n    # {}\n",
                if asynchronous { "async " } else { "" },
                if asynchronous {
                    "AsyncClient"
                } else {
                    "Client"
                },
                if asynchronous {
                    &op.async_success_type
                } else {
                    &op.success_type
                },
                comment(&source_text(&op.source))
            ));
            let (setup, invoke) = call_source(plan, examples, call, asynchronous, None);
            code.push_str(&native_examples::indent(
                &format!(
                    "{}{setup}{}{invoke}\n",
                    provenance(examples, call),
                    if op.responses().iter().any(|response| response.succeeds()) {
                        "return "
                    } else {
                        ""
                    }
                ),
                4,
            ));
        }
    }
    code.push_str("\n\ndef run_sync(client: Client) -> None:\n");
    for call in &examples.calls {
        code.push_str(&format!("    call_{}(client)\n", call.index));
    }
    code.push_str(&format!(
        "    print(\"http-examples {}\")\n",
        examples.calls.len()
    ));
    code.push_str("\n\nasync def run_async(client: AsyncClient) -> None:\n");
    for call in &examples.calls {
        code.push_str(&format!("    await async_call_{}(client)\n", call.index));
    }
    code.push_str(&format!(
        "    print(\"http-examples {}\")\n",
        examples.calls.len()
    ));
    let schemes = plan
        .operations()
        .iter()
        .flat_map(|op| {
            op.wire()
                .security()
                .alternatives()
                .iter()
                .flat_map(|alternative| {
                    alternative
                        .requirements()
                        .iter()
                        .filter(|requirement| {
                            matches!(
                                requirement.credential(),
                                crate::http_protocol::CredentialHook::Bearer { .. }
                                    | crate::http_protocol::CredentialHook::ApiKey { .. }
                            )
                        })
                        .map(|requirement| requirement.name())
                })
        })
        .collect::<std::collections::BTreeSet<_>>();
    let auth = if schemes.is_empty() {
        "auth: dict[str, str] = {}".to_owned()
    } else {
        format!(
            "auth = {{scheme: args.token for scheme in [{}]}}",
            schemes.into_iter().map(q).collect::<Vec<_>>().join(", ")
        )
    };
    code.push_str(&format!("\n\nasync def _async_fixture(server_url: str, auth: dict[str, str]) -> None:\n    async with AsyncClient(auth=auth, server_url=server_url) as client:\n        await run_async(client)\n\ndef main() -> None:\n    parser = argparse.ArgumentParser(description=__doc__)\n    parser.add_argument(\"--server-url\")\n    parser.add_argument(\"--token\")\n    parser.add_argument(\"--async-client\", action=\"store_true\")\n    args = parser.parse_args()\n    validate()\n    if args.server_url is not None:\n        if args.token is None:\n            parser.error(\"--token is required for fixture HTTP calls\")\n        {auth}\n        if args.async_client:\n            asyncio.run(_async_fixture(args.server_url, auth))\n        else:\n            with Client(auth=auth, server_url=args.server_url) as client:\n                run_sync(client)\n\n\nif __name__ == \"__main__\":\n    main()\n"));
    code
}

// This is a documentation-time tool, never a generator-time import of emitted
// code. Literal nodes make both native docstrings and native annotations inert.
const SPHINX: &str = r#"import importlib
import inspect
import json
from pathlib import Path
import sys

from docutils import nodes
from docutils.parsers.rst import directives
from sphinx.errors import SphinxError
from sphinx.util.docutils import SphinxDirective

project = PROJECT
release = VERSION
extensions = []
master_doc = "index"
html_theme = "alabaster"
exclude_patterns = ["_build"]
nitpicky = True
_seen = {}


def _annotations(owner):
    try:
        if inspect.isclass(owner):
            annotations = {}
            for base in reversed(owner.__mro__):
                annotations.update(inspect.get_annotations(base, eval_str=False))
            return annotations
        return inspect.get_annotations(owner, eval_str=False)
    except (TypeError, ValueError):
        return {}


def _describe(module_name, qualname):
    owner = importlib.import_module(module_name)
    parts = qualname.split(".")
    for part in parts[:-1]:
        owner = getattr(owner, part)
    name = parts[-1]
    annotations = _annotations(owner)
    absent = object()
    value = inspect.getattr_static(owner, name, absent)
    if value is absent and name not in annotations:
        raise SphinxError("Missing native Python symbol: " + module_name + "." + qualname)
    if isinstance(value, (classmethod, staticmethod)):
        value = getattr(owner, name)
    if isinstance(value, property):
        value = value.fget
    full = module_name + "." + qualname
    if name in annotations:
        annotation = annotations[name]
        text = full + ": " + (annotation if isinstance(annotation, str) else inspect.formatannotation(annotation))
    elif inspect.ismodule(value):
        text = full + " = module " + value.__name__
    elif callable(value):
        try:
            text = full + str(inspect.signature(value, eval_str=False))
        except (TypeError, ValueError):
            text = full + " = " + inspect.formatannotation(value)
    else:
        text = full + " = " + repr(value)
    # Future-annotation TypeAliases retain an annotation plus a real value.
    if name in annotations and str(annotations[name]).endswith("TypeAlias"):
        text += " = " + inspect.formatannotation(value)
    _seen[full] = text
    doc = inspect.getdoc(value) if str(getattr(value, "__module__", "")).startswith(IMPORT + ".") else None
    return text, doc, value


class NativePython(SphinxDirective):
    required_arguments = 2
    option_spec = {"members": directives.flag}

    def run(self):
        module, name = self.arguments
        _seen.clear()
        text, doc, value = _describe(module, name)
        result = [nodes.literal_block(text, text, language="python")]
        if doc:
            result.append(nodes.literal_block(doc, doc, language="text"))
        if "members" in self.options and inspect.isclass(value) and value.__module__.startswith(IMPORT + "."):
            members = set()
            for base in value.__mro__:
                if base.__module__.startswith(IMPORT + "."):
                    members.update(vars(base))
                    members.update(_annotations(base))
            for member in sorted(members):
                if member.startswith("_") and member not in ("__init__", "__enter__", "__exit__", "__aenter__", "__aexit__"):
                    continue
                signature, _, _ = _describe(module, name + "." + member)
                result.append(nodes.literal_block(signature, signature, language="python"))
        inventory = getattr(self.env, "suspect_python_symbols", {})
        inventory.setdefault(self.env.docname, {}).update(_seen)
        self.env.suspect_python_symbols = inventory
        self.env.note_dependency(str(Path(__file__).parent / "source-bindings.json"))
        # Native imports are the signature authority. Tracking their files also
        # makes incremental Sphinx builds invalidate on native API changes.
        for module_name, imported in list(sys.modules.items()):
            if module_name == IMPORT or module_name.startswith(IMPORT + "."):
                filename = getattr(imported, "__file__", None)
                if filename:
                    self.env.note_dependency(filename)
        return result


def _finished(app, exception):
    if exception is not None:
        return
    bindings = json.loads((Path(__file__).parent / "source-bindings.json").read_text())
    seen = {}
    for entries in getattr(app.env, "suspect_python_symbols", {}).values():
        seen.update(entries)
    missing = [s["name"] for s in bindings["symbols"] if s["name"] not in seen]
    if missing:
        raise SphinxError("Undocumented planned Python symbols: " + ", ".join(missing))
    coverage = {"format": "suspect-python-native-docs-v1", "import": IMPORT,
                "documented": sorted(seen), "signatures": seen,
                "plannedSymbols": len(bindings["symbols"])}
    (Path(app.outdir) / "coverage.json").write_text(json.dumps(coverage, indent=2) + "\n")


def _purge(app, env, docname):
    getattr(env, "suspect_python_symbols", {}).pop(docname, None)


def setup(app):
    app.add_directive("native-python", NativePython)
    app.connect("env-purge-doc", _purge)
    app.connect("build-finished", _finished)
    return {"version": "1", "parallel_read_safe": False}
"#;
