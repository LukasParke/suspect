//! PHP source, Composer package and native documentation from one admitted plan.

use super::{
    ModelCore as Plan, PhpConfig, PlannedOperation,
    models::{Extras, Field, Initializer, Node, Shape, union},
};
use crate::OutFile;
use std::{collections::BTreeMap, fmt::Write};
use suspect_ir::contract::{ParameterLocation, SchemaId};
use suspect_schema::{OwnedProgram, ProgramInstruction, ProgramSource};

#[path = "docs.rs"]
pub(super) mod docs;

pub(super) fn validation(program: &OwnedProgram, config: &PhpConfig) -> Vec<OutFile> {
    let runtime = RuntimeContext { program, config };
    let mut files: Vec<_> = [
        ("Number.php", include_str!("Number.php")),
        ("Json.php", include_str!("Json.php")),
        ("Validation.php", include_str!("Validation.php")),
        ("ValidationV2.php", include_str!("ValidationV2.php")),
        (
            "ValidationResources.php",
            include_str!("ValidationResources.php"),
        ),
    ]
    .into_iter()
    .map(|(name, text)| OutFile {
        path: format!("php/src/{name}"),
        content: text.replace("__NAMESPACE__", &config.namespace),
    })
    .collect();
    files.push(OutFile {
        path: "php/src/RuntimeConfig.php".into(),
        content: runtime.config(),
    });
    files.push(OutFile {
        path: "php/src/ValidationProgram.php".into(),
        content: runtime.validation_program(),
    });
    files
}

pub(super) fn package(plan: &Plan) -> Vec<OutFile> {
    let mut files = Vec::new();
    let mut add = |path: &str, content: String| {
        files.push(OutFile {
            path: format!("php/{path}"),
            content,
        })
    };
    for (path, text) in [
        ("src/Number.php", include_str!("Number.php")),
        ("src/Json.php", include_str!("Json.php")),
        ("src/Validation.php", include_str!("Validation.php")),
        ("src/ValidationV2.php", include_str!("ValidationV2.php")),
        (
            "src/ValidationResources.php",
            include_str!("ValidationResources.php"),
        ),
        ("src/Http.php", include_str!("Http.php")),
        ("src/Protocol.php", include_str!("Protocol.php")),
        ("src/Stream.php", include_str!("Stream.php")),
        ("src/Parts.php", include_str!("Parts.php")),
        ("src/Transport.php", include_str!("Transport.php")),
    ] {
        add(path, text.replace("__NAMESPACE__", &plan.config.namespace));
    }
    let indices: BTreeMap<_, _> = plan
        .program
        .nodes
        .iter()
        .enumerate()
        .map(|(i, n)| ((n.source.document.clone(), n.source.pointer.clone()), i))
        .collect();
    let context = Context { plan, indices };
    let runtime = RuntimeContext {
        program: &plan.program,
        config: &plan.config,
    };
    add("src/RuntimeConfig.php", runtime.config());
    add("src/ValidationProgram.php", runtime.validation_program());
    add("src/Models.php", context.models());
    add("src/Codecs.php", context.codecs());
    add("src/Operations.php", context.operations());
    add("src/Client.php", context.client());
    let manifest = serde_json::json!({
        "name": plan.config.package_name,
        "version": plan.config.package_version,
        "description": "Native source-generated PHP SDK with exact JSON codecs and bounded validation",
        "type": "library",
        "license": "proprietary",
        "require": {"php": "^8.3", "ext-json": "*"},
        "suggest": {"ext-curl": "Required by the default CurlTransport; custom transports may supply their own HTTP implementation."},
        "autoload": {"classmap": ["src/"]},
        "require-dev": {"phpstan/phpstan": "2.2.13"},
        "scripts": {"typecheck": "phpstan analyse --no-progress", "examples": ["@php examples/codecs.php", "@php examples/client.php", "@php examples/quickstart.php"]},
        "config": {"allow-plugins": false, "sort-packages": true},
        "archive": {"exclude": ["/vendor", "/composer.lock", "/build", "/.git"]},
    });
    add(
        "composer.json",
        format!("{}\n", serde_json::to_string_pretty(&manifest).unwrap()),
    );
    add("phpstan.neon", "parameters:\n    level: max\n    phpVersion: 80300\n    treatPhpDocTypesAsCertain: false\n    paths:\n        - src\n        - examples\n    tmpDir: build/phpstan\n".into());
    add("README.md", context.readme());
    add("docs/index.html", context.reference());
    add("docs/coverage.json", context.coverage());
    add(
        "docs/examples.json",
        crate::http_examples::manifest(&plan.examples),
    );
    add(
        "docs/examples.md",
        crate::http_examples::markdown(&plan.examples, "composer examples"),
    );
    add("examples/codecs.php", context.codec_examples());
    add("examples/client.php", context.client_examples());
    add("examples/quickstart.php", context.quickstart_example());
    add("examples/autoload.php", "<?php\ndeclare(strict_types=1);\n// An installed-package consumer may explicitly supply its own Composer loader.\nrequire getenv('SUSPECT_SDK_AUTOLOAD') ?: dirname(__DIR__) . '/vendor/autoload.php';\n".into());
    files.sort_by(|a, b| a.path.cmp(&b.path));
    files
}

struct Context<'a> {
    plan: &'a Plan,
    indices: BTreeMap<(String, String), usize>,
}

pub(super) fn model_assets(plan: &Plan) -> Vec<OutFile> {
    let mut files = validation(&plan.program, &plan.config);
    for (name, text) in [
        ("Http.php", include_str!("Http.php")),
        ("Protocol.php", include_str!("Protocol.php")),
        ("Stream.php", include_str!("Stream.php")),
        ("Parts.php", include_str!("Parts.php")),
        ("Transport.php", include_str!("Transport.php")),
    ] {
        files.push(OutFile {
            path: format!("php/src/{name}"),
            content: text.replace("__NAMESPACE__", &plan.config.namespace),
        });
    }
    let indices = plan
        .program
        .nodes
        .iter()
        .enumerate()
        .map(|(i, n)| ((n.source.document.clone(), n.source.pointer.clone()), i))
        .collect();
    let context = Context { plan, indices };
    files.push(OutFile {
        path: "php/src/Models.php".into(),
        content: context.models(),
    });
    files.push(OutFile {
        path: "php/src/Codecs.php".into(),
        content: context.codecs(),
    });
    files
}

impl Context<'_> {
    fn head(&self) -> String {
        format!(
            "<?php\ndeclare(strict_types=1);\n\nnamespace {};\n\n",
            self.plan.config.namespace
        )
    }
    fn index(&self, id: &SchemaId) -> usize {
        self.indices[&(id.document().to_string(), id.pointer().to_owned())]
    }
    fn name(&self, id: &SchemaId) -> &str {
        &self.plan.models.nodes[id].name
    }
    fn ty(&self, id: &SchemaId, doc: bool) -> String {
        self.plan.models.type_name(id, doc)
    }
    fn field_type(&self, id: &SchemaId, required: bool, doc: bool) -> String {
        if required {
            self.ty(id, doc)
        } else {
            union(vec![self.ty(id, doc), "Absent".into()])
        }
    }
    fn extra_type(&self, extras: &Extras) -> String {
        format!(
            "array<array-key, {}>",
            match extras {
                Extras::Typed(id) => self.ty(id, true),
                _ => "JsonValue".into(),
            }
        )
    }
}

struct RuntimeContext<'a> {
    config: &'a PhpConfig,
    program: &'a OwnedProgram,
}

impl RuntimeContext<'_> {
    fn head(&self) -> String {
        format!(
            "<?php\ndeclare(strict_types=1);\n\nnamespace {};\n\n",
            self.config.namespace
        )
    }

    fn config(&self) -> String {
        let c = self.config;
        let p = &self.program.limits;
        // ua/v1 attribution: constant parts compile at generation time; an absent
        // descriptor keeps the empty disabled sentinel so the runtime emits no header.
        let attribution = match &c.attribution {
            Some(descriptor) => format!(
                "    /** ua/v1 attribution compiled from package identity and source; the runtime supplies the language version. */\n    public const ATTRIBUTION_SUSPECT_VERSION = {};\n    public const ATTRIBUTION_SDK_NAME = {};\n    public const ATTRIBUTION_SDK_VERSION = {};\n    public const ATTRIBUTION_SPEC_VERSION = {};\n    public const ATTRIBUTION_LANGUAGE = {};\n",
                php(&descriptor.suspect_version),
                php(&descriptor.sdk_name),
                php(&descriptor.sdk_version),
                php(&descriptor.spec_version),
                php(&descriptor.language)
            ),
            None => "    /** An empty suspect version disables the automatic attribution header. */\n    public const ATTRIBUTION_SUSPECT_VERSION = \"\";\n    public const ATTRIBUTION_SDK_NAME = \"\";\n    public const ATTRIBUTION_SDK_VERSION = \"\";\n    public const ATTRIBUTION_SPEC_VERSION = \"\";\n    public const ATTRIBUTION_LANGUAGE = \"\";\n"
                .into(),
        };
        format!(
            "{}/** Immutable generated ceilings and ua/v1 attribution constants. @internal */\nfinal class RuntimeConfig\n{{\n    public const MAX_REQUEST_BYTES = {};\n    public const MAX_RESPONSE_BYTES = {};\n    public const MAX_CAPTURE_BYTES = {};\n    public const MAX_HEADER_BYTES = {};\n    public const MAX_CONVERSION_BYTES = {};\n    public const MAX_JSON_BYTES = {};\n    public const MAX_DEPTH = {};\n    public const MAX_NODES = {};\n    public const MAX_SCHEMA_DEPTH = {};\n    public const MAX_NUMBER_BYTES = {};\n    public const MAX_EVALUATION_STEPS = {};\n    public const MAX_EQUALITY_STEPS = {};\n{}}}\n",
            self.head(),
            c.max_request_bytes,
            c.max_response_bytes,
            c.max_capture_bytes,
            c.max_header_bytes,
            c.max_conversion_bytes,
            c.max_request_bytes.max(c.max_response_bytes),
            c.max_depth,
            c.max_nodes,
            p.max_depth,
            p.max_number_bytes,
            p.max_evaluation_steps,
            p.max_equality_steps,
            attribution
        )
    }

    fn validation_program(&self) -> String {
        let p = self.program;
        let mut out = self.head();
        writeln!(out, "/** Checked OwnedCompiler instruction graph; numeric literals are lossless. @internal */\nfinal class ValidationProgram\n{{\n    public const VERSION = {};\n    public const PROFILE = {};\n    /** @return list<int> */\n    public static function roots(): array {{ return [{}]; }}\n    /** @return list<ValidationNode> */\n    public static function nodes(): array\n    {{\n        /** @var list<ValidationNode>|null $nodes */\n        static $nodes = null;\n        if ($nodes !== null) {{ return $nodes; }}\n        return $nodes = [", php(p.version), php(p.profile), ints(p.roots.iter().map(|r| r.target))).unwrap();
        for node in &p.nodes {
            writeln!(
                out,
                "            new ValidationNode({}, [",
                php(&source(&node.source))
            )
            .unwrap();
            for check in &node.checks {
                let raw = serde_json::to_value(&check.instruction).unwrap();
                write!(
                    out,
                    "                new ValidationInstruction({}, {}",
                    php(raw["op"].as_str().unwrap()),
                    php(&source(&check.source))
                )
                .unwrap();
                match &check.instruction {
                    ProgramInstruction::DynamicRef {
                        target,
                        initial_resource,
                        anchor,
                    } => write!(
                        out,
                        ", target: {target}, initialResource: {initial_resource}, anchor: {}",
                        anchor.as_ref().map_or("null".into(), |name| php(name))
                    )
                    .unwrap(),
                    ProgramInstruction::Always { value } => {
                        write!(out, ", truth: {}", boolean(*value)).unwrap()
                    }
                    ProgramInstruction::Type { types } => write!(
                        out,
                        ", types: [{}]",
                        strings(types.iter().map(|t| {
                            serde_json::to_value(t)
                                .unwrap()
                                .as_str()
                                .unwrap()
                                .to_owned()
                        }))
                    )
                    .unwrap(),
                    ProgramInstruction::Ref { target } | ProgramInstruction::Not { target } => {
                        write!(out, ", target: {target}").unwrap()
                    }
                    ProgramInstruction::Properties { properties } => write!(
                        out,
                        ", properties: [{}]",
                        properties
                            .iter()
                            .map(|p| format!("[{}, {}]", php(&p.name), p.target))
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                    .unwrap(),
                    ProgramInstruction::AdditionalProperties { declared, target } => write!(
                        out,
                        ", target: {target}, declared: [{}]",
                        strings(declared.iter())
                    )
                    .unwrap(),
                    ProgramInstruction::Required { names } => {
                        write!(out, ", names: [{}]", strings(names.iter())).unwrap()
                    }
                    ProgramInstruction::Items { target, start } => {
                        write!(out, ", target: {target}, start: {start}").unwrap()
                    }
                    ProgramInstruction::PrefixItems { targets }
                    | ProgramInstruction::AllOf { targets }
                    | ProgramInstruction::AnyOf { targets }
                    | ProgramInstruction::OneOf { targets } => {
                        write!(out, ", targets: [{}]", ints(targets.iter().copied())).unwrap()
                    }
                    ProgramInstruction::Bound {
                        value,
                        maximum,
                        exclusive,
                    } => write!(
                        out,
                        ", number: JsonNumber::fromString({}), maximum: {}, exclusive: {}",
                        php(value),
                        boolean(*maximum),
                        boolean(*exclusive)
                    )
                    .unwrap(),
                    ProgramInstruction::MultipleOf { value } => {
                        write!(out, ", number: JsonNumber::fromString({})", php(value)).unwrap()
                    }
                    ProgramInstruction::Count {
                        value,
                        maximum,
                        target,
                    } => write!(
                        out,
                        ", number: JsonNumber::fromString({}), maximum: {}, countTarget: {}",
                        php(value),
                        boolean(*maximum),
                        php(serde_json::to_value(target).unwrap().as_str().unwrap())
                    )
                    .unwrap(),
                    ProgramInstruction::Enum { values } => write!(
                        out,
                        ", literals: [{}]",
                        values
                            .iter()
                            .map(|v| format!("new ValidationLiteral({})", php(&v.to_string())))
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                    .unwrap(),
                    ProgramInstruction::Const { value } => write!(
                        out,
                        ", constant: new ValidationLiteral({})",
                        php(&value.to_string())
                    )
                    .unwrap(),
                    ProgramInstruction::UniqueItems => {}
                    ProgramInstruction::Pattern { program } => {
                        write!(out, ", pattern: new PatternProgram({}, [", program.start).unwrap();
                        for state in &program.states {
                            use suspect_schema::PatternState::*;
                            let (op, more) = match state {
                                Match => ("match", String::new()),
                                Char { ranges, target } => (
                                    "char",
                                    format!(
                                        ", target: {target}, ranges: [{}]",
                                        ranges
                                            .iter()
                                            .map(|r| format!("[{}, {}]", r[0], r[1]))
                                            .collect::<Vec<_>>()
                                            .join(", ")
                                    ),
                                ),
                                Split { first, second } => {
                                    ("split", format!(", first: {first}, second: {second}"))
                                }
                                Jump { target } => ("jump", format!(", target: {target}")),
                                Start { target } => ("start", format!(", target: {target}")),
                                End { target } => ("end", format!(", target: {target}")),
                            };
                            write!(out, "new PatternState({}{more}), ", php(op)).unwrap();
                        }
                        out.push_str("])");
                    }
                    ProgramInstruction::If {
                        condition,
                        then_target,
                        else_target,
                    } => write!(
                        out,
                        ", condition: {condition}, thenTarget: {}, elseTarget: {}",
                        then_target.map_or("null".into(), |v| v.to_string()),
                        else_target.map_or("null".into(), |v| v.to_string())
                    )
                    .unwrap(),
                    ProgramInstruction::DependentRequired { dependencies } => write!(
                        out,
                        ", requirements: [{}]",
                        dependencies
                            .iter()
                            .map(|(trigger, names)| format!(
                                "[{}, [{}]]",
                                php(trigger),
                                strings(names.iter())
                            ))
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                    .unwrap(),
                    ProgramInstruction::DependentSchemas { dependencies } => write!(
                        out,
                        ", dependencies: [{}]",
                        dependencies
                            .iter()
                            .map(|p| format!("[{}, {}]", php(&p.name), p.target))
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                    .unwrap(),
                    ProgramInstruction::Contains {
                        target,
                        minimum,
                        maximum,
                    } => write!(
                        out,
                        ", target: {target}, containsMinimum: {}, containsMaximum: {}",
                        minimum.as_ref().map_or("null".into(), |n| format!(
                            "JsonNumber::fromString({})",
                            php(n)
                        )),
                        maximum.as_ref().map_or("null".into(), |n| format!(
                            "JsonNumber::fromString({})",
                            php(n)
                        ))
                    )
                    .unwrap(),
                    ProgramInstruction::PatternProperties { patterns } => write!(
                        out,
                        ", patterns: [{}]",
                        patterns
                            .iter()
                            .map(|(text, program, target)| format!(
                                "new PatternProperty({}, {}, {target})",
                                php(text),
                                pattern_expression(program)
                            ))
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                    .unwrap(),
                    ProgramInstruction::AdditionalPropertiesWithPatterns { declared, target } => {
                        write!(
                            out,
                            ", target: {target}, declared: [{}]",
                            strings(declared.iter())
                        )
                        .unwrap()
                    }
                    ProgramInstruction::PropertyNames { target }
                    | ProgramInstruction::UnevaluatedProperties { target }
                    | ProgramInstruction::UnevaluatedItems { target } => {
                        write!(out, ", target: {target}").unwrap()
                    }
                }
                out.push_str("),\n");
            }
            out.push_str("            ]),\n");
        }
        out.push_str("        ];\n    }\n");
        if let Some(resources) = &p.resource_context {
            out.push_str("    public static function resources(): ValidationResources {\n        /** @var ValidationResources|null $value */\n        static $value = null;\n        return $value ??= new ValidationResources([\n");
            for resource in &resources.resources {
                writeln!(
                    out,
                    "            new ValidationResource({}, {}, {}, {}, [{}], {}, [{}]),",
                    php(&source(&resource.source)),
                    php(resource.kind),
                    php(&resource.canonical_uri),
                    php(&resource.base_uri),
                    strings(resource.aliases.iter()),
                    resource
                        .declaration_source
                        .as_ref()
                        .map_or("null".into(), |s| php(&source(s))),
                    resource
                        .dynamic_anchors
                        .iter()
                        .map(|(name, at, target)| format!(
                            "[{}, {}, {target}]",
                            php(name),
                            php(&source(at))
                        ))
                        .collect::<Vec<_>>()
                        .join(", ")
                )
                .unwrap();
            }
            writeln!(
                out,
                "        ], [{}]);\n    }}",
                resources
                    .node_scopes
                    .iter()
                    .map(|(resource, root, address)| format!(
                        "[{resource}, {}, {}]",
                        php(&source(root)),
                        php(address)
                    ))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
            .unwrap();
        }
        out.push_str("}\n");
        out
    }
}

fn pattern_expression(program: &suspect_schema::PatternProgram) -> String {
    let mut out = format!("new PatternProgram({}, [", program.start);
    for state in &program.states {
        use suspect_schema::PatternState::*;
        let (op, more) = match state {
            Match => ("match", String::new()),
            Char { ranges, target } => (
                "char",
                format!(
                    ", target: {target}, ranges: [{}]",
                    ranges
                        .iter()
                        .map(|r| format!("[{}, {}]", r[0], r[1]))
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            ),
            Split { first, second } => ("split", format!(", first: {first}, second: {second}")),
            Jump { target } => ("jump", format!(", target: {target}")),
            Start { target } => ("start", format!(", target: {target}")),
            End { target } => ("end", format!(", target: {target}")),
        };
        write!(out, "new PatternState({}{more}), ", php(op)).unwrap();
    }
    out.push_str("])");
    out
}

impl Context<'_> {
    fn models(&self) -> String {
        let mut out = self.head();
        for node in self.plan.models.nodes.values() {
            let name = &node.name;
            match &node.shape {
                Shape::Object { fields, extras } => {
                    doc(
                        &mut out,
                        &format!(
                            "{}\nSource: {}#{}\nMutable fields are revalidated by every encode.",
                            node.description,
                            node.source.document(),
                            node.source.pointer()
                        ),
                        "",
                    );
                    writeln!(
                        out,
                        "final class {name} implements Model\n{{\n    public const SCHEMA = {};",
                        self.index(&node.source)
                    )
                    .unwrap();
                    for f in fields.iter().filter(|f| f.initializer.is_some()) {
                        writeln!(out, "    /** Source-required constant wire member {}. */\n    public readonly {} ${};", comment_inline(&f.wire), self.ty(&f.source, false), f.name).unwrap();
                    }
                    let sorted = sorted_fields(fields);
                    out.push_str("    /**\n");
                    for f in &sorted {
                        writeln!(
                            out,
                            "     * @param {} ${} {}",
                            self.field_type(&f.source, f.required, true),
                            f.name,
                            comment_inline(&f.description)
                        )
                        .unwrap();
                    }
                    if !matches!(extras, Extras::Closed) {
                        writeln!(out, "     * @param {} $extra Additional JSON members, excluding declared wire names.", self.extra_type(extras)).unwrap();
                    }
                    out.push_str("     */\n    public function __construct(\n");
                    for f in &sorted {
                        writeln!(
                            out,
                            "        public {} ${}{},",
                            self.field_type(&f.source, f.required, false),
                            f.name,
                            if f.required { "" } else { " = Absent::Value" }
                        )
                        .unwrap();
                    }
                    if !matches!(extras, Extras::Closed) {
                        out.push_str("        public array $extra = [],\n");
                    }
                    out.push_str("    ) {\n");
                    for f in fields {
                        if let Some(Initializer::EnumCase {
                            type_name,
                            case_name,
                            ..
                        }) = &f.initializer
                        {
                            writeln!(out, "        $this->{} = {type_name}::{case_name};", f.name)
                                .unwrap();
                        }
                    }
                    out.push_str("    }\n");
                    self.model_codecs(&mut out, node);
                    out.push_str("}\n\n");
                }
                Shape::Enum { cases } => {
                    doc(
                        &mut out,
                        &format!(
                            "{}\nExact string literals from {}#{}",
                            node.description,
                            node.source.document(),
                            node.source.pointer()
                        ),
                        "",
                    );
                    writeln!(out, "enum {name}: string implements Model\n{{").unwrap();
                    for (case, value) in cases {
                        writeln!(out, "    case {case} = {};", php(value)).unwrap();
                    }
                    self.model_codecs(&mut out, node);
                    out.push_str("}\n\n");
                }
                _ => {}
            }
        }
        out
    }

    fn model_codecs(&self, out: &mut String, node: &Node) {
        let nullable = if node.nullable { "|null" } else { "" };
        writeln!(out, "    /** Decode and validate the source schema; absence remains explicit. */\n    public static function fromJson(string $json): self{nullable} {{ return Codecs::decode{}($json); }}\n    /** Revalidate mutable native state and serialize exact tokens. */\n    public function toJson(): string {{ return Codecs::encode{}($this); }}", node.name, node.name).unwrap();
    }

    fn codecs(&self) -> String {
        let mut out = self.head();
        out.push_str("/** Source-indexed native codecs. Public encoders always revalidate mutable state. */\nfinal class Codecs\n{\n");
        for node in self.plan.models.nodes.values() {
            let id = &node.source;
            let name = &node.name;
            let ty = self.ty(id, false);
            let doc_ty = self.ty(id, true);
            let index = self.index(id);
            writeln!(out, "    /** Decode exact JSON from {}#{}.\n     * @return {doc_ty}\n     */\n    public static function decode{name}(string $json, ?CodecContext $context = null): {ty}\n    {{\n        $context ??= new CodecContext();\n        return self::from{name}(JsonValue::parse($json, new JsonLimits(control: $context->control)), $context);\n    }}\n    /** @param {doc_ty} $value */\n    public static function encode{name}({ty} $value, ?CodecContext $context = null): string\n    {{\n        $context ??= new CodecContext();\n        return self::to{name}($value, $context)->toJson(new JsonLimits(control: $context->control));\n    }}\n    /** @return {doc_ty} */\n    public static function from{name}(JsonValue $value, ?CodecContext $context = null): {ty}\n    {{\n        $context ??= new CodecContext();\n        $context->validation->check({index}, $value);\n        return self::read{name}($value, $context);\n    }}\n    /** @param {doc_ty} $value */\n    public static function to{name}({ty} $value, ?CodecContext $context = null): JsonValue\n    {{\n        $context ??= new CodecContext();\n        try {{ $json = self::write{name}($value, $context); }}\n        catch (\\Error $e) {{ throw new JsonError('conversion', 'invalid mutable native value', $e); }}\n        $context->validation->check({index}, $json);\n        return $json;\n    }}", comment_inline(&id.document().to_string()), comment_inline(id.pointer())).unwrap();
            writeln!(out, "    /** @return {doc_ty} */\n    private static function read{name}(JsonValue $value, CodecContext $context): {ty}\n    {{\n        $context->enter();\n        try {{").unwrap();
            if node.nullable && !matches!(node.shape, Shape::Types(_)) {
                out.push_str("            if ($value->kind === JsonKind::Null) { return null; }\n");
            }
            out.push_str(&self.read_shape(node));
            out.push_str("        } finally { $context->leave(); }\n    }\n");
            let object = if matches!(node.shape, Shape::Object { .. }) {
                "$value"
            } else {
                ""
            };
            // PHP arrays carry no runtime element type. Private trial converters
            // accept erased collections and check each element; public codecs
            // retain the full PHPDoc type for normal callers and typecheckers.
            let unchecked = ty
                .split('|')
                .map(|part| {
                    if part == "array" {
                        "array<array-key, mixed>"
                    } else {
                        part
                    }
                })
                .collect::<Vec<_>>()
                .join("|");
            writeln!(out, "    /** @param {unchecked} $value */\n    private static function write{name}({ty} $value, CodecContext $context): JsonValue\n    {{\n        $context->enter({object});\n        try {{").unwrap();
            if node.nullable && !matches!(node.shape, Shape::Types(_)) {
                out.push_str("            if ($value === null) { return JsonValue::null(); }\n");
            }
            out.push_str(&self.write_shape(node));
            writeln!(
                out,
                "        }} finally {{ $context->leave({object}); }}\n    }}\n"
            )
            .unwrap();
        }
        out.push_str("}\n");
        out
    }

    fn read_shape(&self, node: &Node) -> String {
        let mut out = String::new();
        match &node.shape {
            Shape::Json => out.push_str("            return $context->json($value);\n"),
            Shape::Null => out.push_str("            return null;\n"),
            Shape::Boolean => out.push_str("            return $value->asBool();\n"),
            Shape::Number => {
                out.push_str("            return $context->number($value->asNumber());\n")
            }
            Shape::String => {
                out.push_str("            return $context->string($value->asString());\n")
            }
            Shape::Enum { .. } => writeln!(
                out,
                "            return {}::from($value->asString());",
                node.name
            )
            .unwrap(),
            Shape::Ref(id) => writeln!(
                out,
                "            return self::read{}($value, $context);",
                self.name(id)
            )
            .unwrap(),
            Shape::Array { item } => {
                out.push_str("            $items = [];\n            foreach ($value->asArray() as $item) {\n");
                writeln!(
                    out,
                    "                $items[] = {};",
                    item.as_ref()
                        .map(|id| format!("self::read{}($item, $context)", self.name(id)))
                        .unwrap_or_else(|| "$context->json($item)".into())
                )
                .unwrap();
                out.push_str("            }\n            return $items;\n");
            }
            Shape::Object { fields, extras } => {
                out.push_str("            $members = $value->asObject();\n");
                if !matches!(extras, Extras::Closed) {
                    let conversion = match extras {
                        Extras::Typed(id) => {
                            format!("self::read{}($item, $context)", self.name(id))
                        }
                        _ => "$context->json($item)".into(),
                    };
                    if fields.is_empty() {
                        writeln!(out, "            $extra = [];\n            foreach ($members as $key => $item) {{\n                $context->bytes(strlen((string) $key));\n                $extra[$key] = {conversion};\n            }}").unwrap();
                    } else {
                        writeln!(out, "            $declared = [{}];\n            $extra = [];\n            foreach ($members as $key => $item) {{\n                $context->bytes(strlen((string) $key));\n                if (!isset($declared[$key])) {{ $extra[$key] = {conversion}; }}\n            }}", fields.iter().map(|f| format!("{} => true", php(&f.wire))).collect::<Vec<_>>().join(", ")).unwrap();
                    }
                } else {
                    out.push_str("            foreach ($members as $key => $_) { $context->bytes(strlen((string) $key)); }\n");
                }
                writeln!(out, "            return new {}(", node.name).unwrap();
                for f in sorted_fields(fields) {
                    let decode = format!(
                        "self::read{}($members[{}], $context)",
                        self.name(&f.source),
                        php(&f.wire)
                    );
                    writeln!(
                        out,
                        "                {}: {},",
                        f.name,
                        if f.required {
                            decode
                        } else {
                            format!(
                                "array_key_exists({}, $members) ? {decode} : Absent::Value",
                                php(&f.wire)
                            )
                        }
                    )
                    .unwrap();
                }
                if !matches!(extras, Extras::Closed) {
                    out.push_str("                extra: $extra,\n");
                }
                out.push_str("            );\n");
            }
            Shape::Union(branches) => {
                out.push_str("            $selected = -1;\n");
                for (i, id) in branches.iter().enumerate() {
                    writeln!(out, "            if ($context->validation->matches({}, $value){}) {{ $selected = {i}; }}", self.index(id), if i == 0 { "" } else { " && $selected === -1" }).unwrap();
                }
                out.push_str("            return match ($selected) {\n");
                for (i, id) in branches.iter().enumerate() {
                    writeln!(
                        out,
                        "                {i} => self::read{}($value, $context),",
                        self.name(id)
                    )
                    .unwrap();
                }
                out.push_str("                default => throw new JsonError('conversion', 'no native union branch accepts the value'),\n            };\n");
            }
            Shape::Types(types) => {
                out.push_str("            $context->json($value);\n");
                out.push_str("            return match ($value->kind) {\n");
                let mut emitted = std::collections::BTreeSet::new();
                for ty in types {
                    let (kind, expr) = match ty.as_str() {
                        "null" => ("Null", "null"),
                        "boolean" => ("Boolean", "$value->asBool()"),
                        "string" => ("String", "$value->asString()"),
                        "integer" | "number" => ("Number", "$value->asNumber()"),
                        "object" => ("Object", "$value"),
                        "array" => ("Array", "$value->asArray()"),
                        _ => unreachable!(),
                    };
                    if emitted.insert(kind) {
                        writeln!(out, "                JsonKind::{kind} => {expr},").unwrap();
                    }
                }
                out.push_str("                default => throw new JsonError('conversion', 'unexpected JSON kind'),\n            };\n");
            }
        }
        out
    }

    fn write_shape(&self, node: &Node) -> String {
        let mut out = String::new();
        match &node.shape {
            Shape::Json => out.push_str("            return $context->json($value);\n"),
            Shape::Null => out.push_str("            return JsonValue::null();\n"),
            Shape::Boolean => out.push_str("            return JsonValue::fromBool($value);\n"),
            Shape::Number => out
                .push_str("            return JsonValue::fromNumber($context->number($value));\n"),
            Shape::String => out
                .push_str("            return JsonValue::fromString($context->string($value));\n"),
            Shape::Enum { .. } => out.push_str(
                "            return JsonValue::fromString($context->string($value->value));\n",
            ),
            Shape::Ref(id) => writeln!(
                out,
                "            return self::write{}($value, $context);",
                self.name(id)
            )
            .unwrap(),
            Shape::Array { item } => {
                out.push_str("            if (!array_is_list($value)) { throw new JsonError('conversion', 'expected native list'); }\n            $items = [];\n            foreach ($value as $item) {\n");
                let element = item
                    .as_ref()
                    .map(|id| self.ty(id, false))
                    .unwrap_or_else(|| "JsonValue".into());
                writeln!(out, "                if (!({})) {{ throw new JsonError('conversion', 'unexpected native list element'); }}", native_guard(&element, "$item")).unwrap();
                writeln!(
                    out,
                    "                $items[] = {};",
                    item.as_ref()
                        .map(|id| format!("self::write{}($item, $context)", self.name(id)))
                        .unwrap_or_else(|| "$context->json($item)".into())
                )
                .unwrap();
                out.push_str("            }\n            return JsonValue::fromArray($items);\n");
            }
            Shape::Object { fields, extras } => {
                out.push_str("            $members = [];\n");
                for f in fields {
                    let assignment = format!(
                        "$context->bytes({}); $members[{}] = self::write{}($value->{}, $context);",
                        f.wire.len(),
                        php(&f.wire),
                        self.name(&f.source),
                        f.name
                    );
                    if f.required {
                        writeln!(out, "            {assignment}").unwrap();
                    } else {
                        writeln!(
                            out,
                            "            if ($value->{} !== Absent::Value) {{ {assignment} }}",
                            f.name
                        )
                        .unwrap();
                    }
                }
                if !matches!(extras, Extras::Closed) {
                    let conversion = match extras {
                        Extras::Typed(id) => {
                            format!("self::write{}($item, $context)", self.name(id))
                        }
                        _ => "$context->json($item)".into(),
                    };
                    if fields.is_empty() {
                        writeln!(out, "            foreach ($value->extra as $key => $item) {{\n                $context->bytes(strlen((string) $key));\n                $members[$key] = {conversion};\n            }}").unwrap();
                    } else {
                        writeln!(out, "            $declared = [{}];\n            foreach ($value->extra as $key => $item) {{\n                $context->bytes(strlen((string) $key));\n                if (isset($declared[$key])) {{ throw new JsonError('conversion', 'extra member collides with a declared wire name'); }}\n                $members[$key] = {conversion};\n            }}", fields.iter().map(|f| format!("{} => true", php(&f.wire))).collect::<Vec<_>>().join(", ")).unwrap();
                    }
                }
                out.push_str("            return JsonValue::fromObject($members);\n");
            }
            Shape::Union(branches) => {
                for id in branches {
                    let guard = self.ty(id, false) != self.ty(&node.source, false);
                    if guard {
                        writeln!(
                            out,
                            "            if ({}) {{",
                            native_guard(&self.ty(id, false), "$value")
                        )
                        .unwrap();
                    }
                    writeln!(out, "                try {{\n                    $candidate = self::write{}($value, $context);\n                    if ($context->validation->matches({}, $candidate)) {{ return $candidate; }}\n                }} catch (JsonError $e) {{ if ($e->kind !== 'conversion') {{ throw $e; }} }}\n                catch (\\TypeError $e) {{ /* Try the next native list/union representation. */ }}", self.name(id), self.index(id)).unwrap();
                    if guard {
                        out.push_str("            }\n");
                    }
                }
                out.push_str("            throw new JsonError('conversion', 'no native union branch accepts the value');\n");
            }
            Shape::Types(types) => {
                let mut emitted = std::collections::BTreeSet::new();
                let mut arms = Vec::new();
                for ty in types {
                    let (kind, guard, expr) = match ty.as_str() {
                        "null" => ("null", "$value === null", "JsonValue::null()"),
                        "boolean" => ("bool", "is_bool($value)", "JsonValue::fromBool($value)"),
                        "string" => (
                            "string",
                            "is_string($value)",
                            "JsonValue::fromString($value)",
                        ),
                        "integer" | "number" => (
                            "number",
                            "$value instanceof JsonNumber",
                            "JsonValue::fromNumber($value)",
                        ),
                        "object" => (
                            "object",
                            "$value instanceof JsonValue && $value->kind === JsonKind::Object",
                            "$value",
                        ),
                        "array" => ("array", "is_array($value)", "$context->array($value)"),
                        _ => unreachable!(),
                    };
                    if emitted.insert(kind) {
                        arms.push((kind, guard, expr));
                    }
                }
                for (index, (kind, guard, expr)) in arms.iter().enumerate() {
                    let result = if *kind == "array" {
                        (*expr).to_owned()
                    } else {
                        format!("$context->json({expr})")
                    };
                    if index + 1 == arms.len() {
                        if *kind == "object" {
                            out.push_str("            if ($value->kind !== JsonKind::Object) { throw new JsonError('conversion', 'expected JSON object'); }\n");
                        }
                        writeln!(out, "            return {result};").unwrap();
                    } else {
                        writeln!(out, "            if ({guard}) {{ return {result}; }}").unwrap();
                    }
                }
            }
        }
        out
    }

    fn operations(&self) -> String {
        let mut out = self.head();
        for op in &self.plan.operations {
            doc(
                &mut out,
                &format!(
                    "Arguments for {}. {}\nSource: {}#{}",
                    op.operation_id,
                    op.wire.description,
                    op.source.document(),
                    op.source.pointer()
                ),
                "",
            );
            writeln!(out, "final class {}\n{{\n    /**", op.input_type).unwrap();
            let args = self.arguments(op);
            for (name, id, required) in &args {
                let description = op
                    .parameters
                    .iter()
                    .find(|p| p.name == *name)
                    .map(|p| p.description.as_str())
                    .or_else(|| {
                        op.body
                            .as_ref()
                            .filter(|b| b.name == *name)
                            .map(|b| b.description.as_str())
                    })
                    .unwrap_or_default();
                writeln!(
                    out,
                    "     * @param {} ${name} {}",
                    self.field_type(id, *required, true),
                    comment_inline(description)
                )
                .unwrap();
            }
            out.push_str("     */\n    public function __construct(\n");
            for (name, id, required) in &args {
                writeln!(
                    out,
                    "        public {} ${name}{},",
                    self.field_type(id, *required, false),
                    if *required { "" } else { " = Absent::Value" }
                )
                .unwrap();
            }
            out.push_str("    ) {}\n}\n\n");
            writeln!(out, "/** Documented API failures for {}. */\nabstract class {} extends ApiError\n{{\n    public function __construct(HttpResponse $response)\n    {{\n        parent::__construct({}, {}, $response);\n    }}\n}}\n", comment_inline(&op.operation_id), op.error_type, php(&op.operation_id), php(&format!("{}#{}", op.source.document(), op.source.pointer()))).unwrap();
            for response in &op.wire.responses {
                let success = (200..300).contains(&response.status);
                let name = if success {
                    &op.success_types
                } else {
                    &op.error_types
                }
                .iter()
                .find(|(status, _)| *status == response.status)
                .unwrap()
                .1
                .as_str();
                let ty = self.ty(&response.schema, false);
                let doc_ty = self.ty(&response.schema, true);
                doc(
                    &mut out,
                    &format!(
                        "{} exact status {}.\n{}\nSource: {}#{}",
                        op.operation_id,
                        response.status,
                        op.responses
                            .iter()
                            .find(|r| r.status == response.status)
                            .expect("allocated response")
                            .description,
                        response.source.document(),
                        response.source.pointer()
                    ),
                    "",
                );
                if success {
                    writeln!(out, "final readonly class {name}\n{{\n    public const STATUS = {};\n    /** @param {doc_ty} $body */\n    public function __construct(public {ty} $body, public HttpResponse $response) {{}}\n}}\n", response.status).unwrap();
                } else {
                    writeln!(out, "final class {name} extends {}\n{{\n    public const STATUS = {};\n    /** @param {doc_ty} $body */\n    public function __construct(public readonly {ty} $body, HttpResponse $response)\n    {{\n        parent::__construct($response);\n    }}\n}}\n", op.error_type, response.status).unwrap();
                }
            }
        }
        out
    }

    fn arguments<'a>(&self, op: &'a PlannedOperation) -> Vec<(&'a str, &'a SchemaId, bool)> {
        let mut args: Vec<_> = op
            .parameters
            .iter()
            .map(|p| (p.name.as_str(), &p.schema, p.required))
            .collect();
        if let Some(body) = &op.wire.body {
            args.push(("body", &body.schema, body.required));
        }
        args.sort_by_key(|(_, _, required)| !*required);
        args
    }

    fn client(&self) -> String {
        let mut out = self.head();
        out.push_str("/** Native operation seam for application dependency injection. */\ninterface ClientInterface\n{\n");
        for op in &self.plan.operations {
            let result = if op.success_types.is_empty() {
                "never".into()
            } else {
                union(
                    op.success_types
                        .iter()
                        .map(|(_, name)| name.clone())
                        .collect(),
                )
            };
            let default = if self.arguments(op).iter().any(|(_, _, required)| *required) {
                "".into()
            } else {
                format!(" = new {}()", op.input_type)
            };
            writeln!(out, "    public function {}({} $input{default}, ?RequestOptions $options = null): {result};", op.method_name, op.input_type).unwrap();
        }
        out.push_str("}\n\n/** Synchronous source-selected client. One invocation sends once. */\nfinal class Client implements ClientInterface\n{\n    /** Credentials and transport are explicit client-wide dependencies. */\n    public function __construct(\n        private readonly Credentials $credentials,\n        private readonly Transport $transport = new CurlTransport(),\n        private readonly ClientOptions $options = new ClientOptions(),\n    ) {}\n");
        for op in &self.plan.operations {
            let result = if op.success_types.is_empty() {
                "never".into()
            } else {
                union(
                    op.success_types
                        .iter()
                        .map(|(_, name)| name.clone())
                        .collect(),
                )
            };
            let mut tags = vec!["@throws SdkError".to_owned()];
            tags.extend(
                op.error_types
                    .iter()
                    .map(|(_, name)| format!("@throws {name}")),
            );
            doc_tags(
                &mut out,
                &format!(
                    "{}\nSource: {}#{}",
                    op.wire.description,
                    op.source.document(),
                    op.source.pointer()
                ),
                "    ",
                &tags,
            );
            let default = if self.arguments(op).iter().any(|(_, _, required)| *required) {
                "".into()
            } else {
                format!(" = new {}()", op.input_type)
            };
            writeln!(out, "    public function {}({} $input{default}, ?RequestOptions $options = null): {result}\n    {{\n        try {{\n        $call = new CallContext($this->options, $options);\n        $context = new CodecContext($call->control);\n        $path = {};\n        $query = [];\n        $queryBytes = 0;\n        $body = null;", op.method_name, op.input_type, php(&op.wire.path)).unwrap();
            if !self.arguments(op).is_empty() {
                out.push_str("        try {\n");
            }
            for p in &op.parameters {
                let name = &p.name;
                let indent = if p.required {
                    "            "
                } else {
                    "                "
                };
                if !p.required {
                    writeln!(out, "            if ($input->{name} !== Absent::Value) {{").unwrap();
                }
                writeln!(
                    out,
                    "{indent}$value = Codecs::to{}($input->{name}, $context);",
                    self.name(&p.schema)
                )
                .unwrap();
                if p.location == ParameterLocation::Path {
                    writeln!(out, "{indent}$path = str_replace({}, Wire::segment($value->asString(), $this->options->maxRequestBytes), $path);", php(&format!("{{{}}}", p.wire_name))).unwrap();
                } else {
                    writeln!(out, "{indent}Wire::query($query, $queryBytes, {}, $value, {}, {}, $this->options->maxRequestBytes);", php(&p.wire_name), boolean(p.array), boolean(p.explode)).unwrap();
                }
                if !p.required {
                    out.push_str("            }\n");
                }
            }
            if let Some(body) = &op.wire.body {
                let assignment = format!(
                    "$body = Codecs::to{}($input->body, $context)->toJson(new JsonLimits(maxBytes: $this->options->maxRequestBytes, control: $call->control));",
                    self.name(&body.schema)
                );
                if body.required {
                    writeln!(out, "            {assignment}").unwrap();
                } else {
                    writeln!(
                        out,
                        "            if ($input->body !== Absent::Value) {{ {assignment} }}"
                    )
                    .unwrap();
                }
            }
            if !self.arguments(op).is_empty() {
                out.push_str("        } catch (ValidationError|JsonError|\\Error $e) {\n            throw new SdkError('request_validation', 'request does not satisfy the source contract', previous: $e);\n        }\n");
            }
            writeln!(out, "        $response = $this->exchange({}, {}, {}, $path, $query, $body, $call);\n        $context = new CodecContext($call->control);\n        switch ($response->status) {{", php(&op.wire.method), php(&op.wire.server), php(&op.wire.security_scheme_name)).unwrap();
            for response in &op.wire.responses {
                let success = (200..300).contains(&response.status);
                let name = if success {
                    &op.success_types
                } else {
                    &op.error_types
                }
                .iter()
                .find(|(s, _)| *s == response.status)
                .unwrap()
                .1
                .as_str();
                writeln!(out, "            case {}:\n                Wire::jsonResponse($response, $call->maxCaptureBytes);\n                try {{ $decoded = Codecs::decode{}($response->body, $context); }}\n                catch (ValidationError|JsonError|\\Error $e) {{ throw new SdkError('response_validation', 'response does not satisfy the declared schema', $response->capture($call->maxCaptureBytes), $e); }}\n                $call->check();\n                {} new {name}($decoded, $response);", response.status, self.name(&response.schema), if success { "return" } else { "throw" }).unwrap();
            }
            writeln!(out, "            default: throw new SdkError('unexpected_status', 'response status is not declared by this operation', $response->capture($call->maxCaptureBytes));\n        }}\n        }} catch (SdkError $e) {{ throw $e->at({}, {}); }}\n    }}\n", php(&op.operation_id), php(&format!("{}#{}", op.source.document(), op.source.pointer()))).unwrap();
        }
        out.push_str(CLIENT_EXCHANGE);
        out.push_str("}\n");
        out
    }
}

const CLIENT_EXCHANGE: &str = r#"    /** @param list<string> $query */
    private function exchange(string $method, string $server, string $scheme, string $path, array $query, ?string $body, CallContext $call): HttpResponse
    {
        $call->check();
        $server = $this->options->serverUrl ?? $server;
        Wire::server($server);
        $url = rtrim($server, '/') . $path . ($query === [] ? '' : '?' . implode('&', $query));
        if (strlen($url) > $this->options->maxRequestBytes || ($body !== null && strlen($body) > $this->options->maxRequestBytes)) {
            throw new SdkError('resource_limit', 'request URL/body exceeds byte ceiling');
        }
        $headers = ['Accept' => 'application/json', 'Accept-Encoding' => 'identity', 'Authorization' => $this->credentials->bearer($scheme)];
        if ($body !== null) { $headers['Content-Type'] = 'application/json'; }
        Wire::headers($headers, $this->options->maxHeaderBytes);
        $request = new HttpRequest($method, $url, $headers, $body, $call->remainingMilliseconds(), $call->maxResponseBytes, $this->options->maxHeaderBytes, $call->maxCaptureBytes, $call->control);
        try { $response = $this->transport->send($request); }
        catch (SdkError $e) { throw $e; }
        catch (\Throwable $e) { throw new SdkError('transport', 'custom transport failed', previous: $e); }
        try { $call->check(); }
        catch (SdkError $e) { throw $e->withCapture($response->capture($call->maxCaptureBytes, true)); }
        if (strlen($response->body) > $call->maxResponseBytes || $response->headerBytes > $this->options->maxHeaderBytes) { throw new SdkError('resource_limit', 'transport returned an oversized response', $response->capture($call->maxCaptureBytes, true)); }
        Wire::contentEncoding($response, $call->maxCaptureBytes);
        return $response;
    }
"#;

fn sorted_fields(fields: &[Field]) -> Vec<&Field> {
    let mut fields: Vec<_> = fields.iter().filter(|f| f.initializer.is_none()).collect();
    fields.sort_by_key(|f| !f.required);
    fields
}
fn native_guard(ty: &str, value: &str) -> String {
    ty.split('|')
        .map(|part| match part {
            "null" => format!("{value} === null"),
            "string" => format!("is_string({value})"),
            "bool" => format!("is_bool({value})"),
            "array" => format!("is_array({value})"),
            name => format!("{value} instanceof {name}"),
        })
        .collect::<Vec<_>>()
        .join(" || ")
}
fn source(source: &ProgramSource) -> String {
    format!("{}#{}", source.document, source.pointer)
}
fn boolean(value: bool) -> &'static str {
    if value { "true" } else { "false" }
}
fn ints(values: impl IntoIterator<Item = usize>) -> String {
    values
        .into_iter()
        .map(|v| v.to_string())
        .collect::<Vec<_>>()
        .join(", ")
}
fn strings(values: impl IntoIterator<Item = impl AsRef<str>>) -> String {
    values
        .into_iter()
        .map(|v| php(v.as_ref()))
        .collect::<Vec<_>>()
        .join(", ")
}

/// PHP double-quoted literal: JSON escaping alone would interpolate `$` and
/// PHP does not understand JSON's `\uNNNN` escape syntax.
fn php(value: &str) -> String {
    let mut out = String::from("\"");
    for c in value.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '$' => out.push_str("\\$"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_ascii_control() => write!(out, "\\x{:02x}", c as u32).unwrap(),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}
fn comment_inline(text: &str) -> String {
    text.replace("*/", "* /")
        .replace("?>", "? >")
        .replace("<?", "< ?")
        .replace('@', "&#64;")
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect()
}
fn doc(out: &mut String, text: &str, indent: &str) {
    doc_tags(out, text, indent, &[]);
}
fn doc_tags(out: &mut String, text: &str, indent: &str, tags: &[String]) {
    writeln!(out, "{indent}/**").unwrap();
    for line in text.lines() {
        writeln!(out, "{indent} * {}", comment_inline(line)).unwrap();
    }
    for tag in tags {
        writeln!(out, "{indent} * {tag}").unwrap();
    }
    writeln!(out, "{indent} */").unwrap();
}
