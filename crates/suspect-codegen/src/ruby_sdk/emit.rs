//! One emitter over retained model and protocol descriptors.
use super::{
    ExtraFields, ModelShape, NativeType, PackageConfig, RecordBinding, SampleValue, ScalarKind,
    SdkPlan,
};
use crate::{OutFile, examples::ExampleRole, http_examples, http_protocol as wire};
use serde_json::{Value, json};
use std::{collections::BTreeSet, fmt::Write};
use suspect_ir::contract::SourceId;

fn q(s: &str) -> String {
    serde_json::to_string(s).unwrap().replace('#', "\\#")
}
fn src(s: &SourceId) -> String {
    format!("{}#{}", s.document(), s.pointer())
}
fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('@', "&#64;")
        .replace('{', "&#123;")
        .replace('}', "&#125;")
        .replace(['\r', '\n'], " ")
}
fn literal(v: &Value) -> String {
    match v {
        Value::Null => "nil".into(),
        Value::Bool(v) => v.to_string(),
        Value::String(s) => q(s),
        Value::Number(n) => format!("JsonNumber.new({})", q(&n.to_string())),
        _ => format!("Json.parse({})", q(&v.to_string())),
    }
}
fn exact(v: &impl serde::Serialize) -> String {
    format!(
        "Json.parse({}, max_bytes: 16 * 1024 * 1024, max_work: 128 * 1024 * 1024)",
        q(&serde_json::to_string(v).unwrap())
    )
}
fn list(v: impl IntoIterator<Item = String>) -> String {
    format!("[{}]", v.into_iter().collect::<Vec<_>>().join(", "))
}
fn docs(out: &mut String, indent: &str, description: &str, source: &SourceId) {
    if !description.is_empty() {
        writeln!(out, "{indent}# {}", esc(description)).unwrap();
    }
    writeln!(out, "{indent}# Source: <code>{}</code>.", esc(&src(source))).unwrap();
}
fn codec_type(plan: &SdkPlan, index: usize) -> String {
    format!("Types::{}", plan.models().symbol(index).unwrap().type_name)
}
fn yard_model(plan: &SdkPlan, index: usize) -> String {
    fn visit(plan: &SdkPlan, i: usize, seen: &mut BTreeSet<usize>, types: &mut BTreeSet<String>) {
        if !seen.insert(i) {
            return;
        }
        let s = plan.models().symbol(i).unwrap();
        match &s.shape {
            ModelShape::Alias(i) => visit(plan, *i, seen, types),
            ModelShape::Union { branches, .. } => {
                for i in branches {
                    visit(plan, *i, seen, types)
                }
            }
            ModelShape::Object { nullable, .. } => {
                types.insert(format!("Models::{}", s.name));
                if *nullable {
                    types.insert("nil".into());
                }
            }
            ModelShape::Array { nullable, .. } => {
                types.insert("Array".into());
                if *nullable {
                    types.insert("nil".into());
                }
            }
            ModelShape::Scalar(k) => {
                for k in k {
                    types.insert(
                        match k {
                            ScalarKind::Null => "nil",
                            ScalarKind::Boolean => "Boolean",
                            ScalarKind::String => "String",
                            _ => "Integer, JsonNumber",
                        }
                        .into(),
                    );
                }
            }
            ModelShape::Literal(values) => {
                for v in values {
                    types.insert(
                        match v {
                            Value::Null => "nil",
                            Value::Bool(_) => "Boolean",
                            Value::String(_) => "String",
                            Value::Number(_) => "Integer, JsonNumber",
                            Value::Array(_) => "Array",
                            Value::Object(_) => "Hash",
                        }
                        .into(),
                    );
                }
            }
            ModelShape::Never => {
                types.insert("void".into());
            }
            _ => {
                types.insert("nil, Boolean, String, Integer, JsonNumber, Array, Hash".into());
            }
        }
    }
    let mut out = BTreeSet::new();
    visit(plan, index, &mut BTreeSet::new(), &mut out);
    out.into_iter().collect::<Vec<_>>().join(", ")
}
fn yard_value(plan: &SdkPlan, t: &NativeType) -> String {
    match t {
        NativeType::Codec(i) => yard_model(plan, *i),
        NativeType::Json => "nil, Boolean, String, Integer, JsonNumber, Array, Hash".into(),
        NativeType::Bytes => "Bytes".into(),
        NativeType::NoContent => "NoContent".into(),
        NativeType::Scalar(s) => match s {
            wire::ScalarType::String => "String",
            wire::ScalarType::Boolean => "Boolean",
            _ => "Integer, JsonNumber",
        }
        .into(),
        NativeType::Record(n) => format!("Models::{n}"),
        NativeType::Array(_) => "Array".into(),
        NativeType::Part(t) => format!("Part, {}", yard_value(plan, t)),
        NativeType::Stream(_) => "ItemStream, Enumerable".into(),
        NativeType::Union(v) => v
            .iter()
            .map(|v| yard_value(plan, v))
            .collect::<Vec<_>>()
            .join(", "),
    }
}
pub(super) fn native_type(plan: &SdkPlan, t: &NativeType, request: bool) -> String {
    match t {
        NativeType::Codec(i) => codec_type(plan, *i),
        NativeType::Json => "json_value".into(),
        NativeType::Bytes => "Bytes".into(),
        NativeType::NoContent => "NoContent".into(),
        NativeType::Scalar(s) => match s {
            wire::ScalarType::String => "String",
            wire::ScalarType::Boolean => "bool",
            _ => "Integer | JsonNumber",
        }
        .into(),
        NativeType::Record(n) => format!("Models::{n}"),
        NativeType::Array(t) => format!("Array[{}]", native_type(plan, t, request)),
        NativeType::Part(t) => {
            let t = native_type(plan, t, request);
            format!("{t} | Part[{t}]")
        }
        NativeType::Stream(t) => format!(
            "{}[{}]",
            if request { "Enumerable" } else { "ItemStream" },
            native_type(plan, t, request)
        ),
        NativeType::Union(types) => {
            if types.is_empty() {
                "bot".into()
            } else {
                types
                    .iter()
                    .map(|t| native_type(plan, t, request))
                    .collect::<Vec<_>>()
                    .join(" | ")
            }
        }
    }
}
fn response_type(plan: &SdkPlan, r: &super::PlannedResponse) -> String {
    if r.body_forbidden {
        return "NoContent".into();
    }
    let mut types = r
        .media
        .iter()
        .map(|m| native_type(plan, &m.value_type, false))
        .collect::<BTreeSet<_>>();
    if r.media.is_empty() {
        types.insert(
            if r.wire.media().is_empty() {
                "Bytes"
            } else {
                "NoContent"
            }
            .into(),
        );
    }
    if matches!(
        r.status,
        wire::ResponseStatus::Range(_) | wire::ResponseStatus::Default
    ) {
        types.insert("NoContent".into());
    }
    types.into_iter().collect::<Vec<_>>().join(" | ")
}
fn model_type(plan: &SdkPlan, s: &super::ModelSymbol) -> String {
    if s.signature_uninhabited {
        return "bot".into();
    }
    match &s.shape {
        ModelShape::Json | ModelShape::RefinedJson | ModelShape::Dynamic { .. } => {
            "json_value".into()
        }
        ModelShape::Never => "bot".into(),
        ModelShape::Alias(i) => codec_type(plan, *i),
        ModelShape::Literal(values) => values
            .iter()
            .map(|v| match v {
                Value::Null => "nil".into(),
                Value::Bool(v) => v.to_string(),
                Value::String(v) => serde_json::to_string(v).unwrap(),
                Value::Number(_) => "Integer | JsonNumber".into(),
                Value::Array(_) => "Array[json_value]".into(),
                Value::Object(_) => "Hash[String, json_value]".into(),
            })
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>()
            .join(" | "),
        ModelShape::Scalar(kinds) => kinds
            .iter()
            .map(|k| match k {
                ScalarKind::Null => "nil",
                ScalarKind::Boolean => "bool",
                ScalarKind::String => "String",
                _ => "Integer | JsonNumber",
            })
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>()
            .join(" | "),
        ModelShape::Object { nullable, .. } => format!(
            "Models::{}{}",
            s.name,
            if *nullable { " | nil" } else { "" }
        ),
        ModelShape::Array {
            items,
            prefix,
            nullable,
        } => {
            let mut kinds = prefix
                .iter()
                .map(|i| codec_type(plan, *i))
                .collect::<Vec<_>>();
            kinds.push(items.map_or("json_value".into(), |i| codec_type(plan, i)));
            format!(
                "Array[{}]{}",
                kinds.join(" | "),
                if *nullable { " | nil" } else { "" }
            )
        }
        ModelShape::Union { branches, .. } => branches
            .iter()
            .map(|i| codec_type(plan, *i))
            .collect::<Vec<_>>()
            .join(" | "),
    }
}
pub(super) fn package(plan: &SdkPlan, package: &PackageConfig) -> Vec<OutFile> {
    let mut files = Vec::new();
    let mut add = |path: String, content: String| {
        files.push(OutFile {
            path: format!("ruby/{path}"),
            content,
        })
    };
    let ns = &package.namespace;
    let req = &package.require_name;
    let mut entry = format!(
        "# frozen_string_literal: true\n# Native source-bound SDK.\nmodule {ns}\n  VERSION = {}.freeze\nend\n",
        q(&package.version)
    );
    let mut requires = Vec::<&str>::new();
    for name in [
        "policy",
        "json",
        "program_guard",
        "resource_guard",
        "validation",
        "validation_v2",
        "validation_v3",
        "program",
        "codecs",
        "models",
        "wire",
        "payload",
        "streams",
        "attribution",
        "http",
    ] {
        requires.push(name);
    }
    // The generated OAuth lifecycle module is emitted only for a configured
    // policy with at least one usable scheme; no-policy output stays
    // byte-identical, and the require appears exactly when the file does.
    if plan.oauth().is_some() {
        requires.push("oauth");
    }
    // The generated incoming receipt module is emitted only for a declared
    // webhook/callback; receipt-less output stays byte-identical, and the
    // require appears exactly when the file does, after the lifecycle module
    // it sits beside.
    if plan.incoming().is_some_and(|incoming| !incoming.is_empty()) {
        requires.push("incoming");
    }
    requires.extend(["wire_models", "client"]);
    for name in requires {
        writeln!(entry, "require_relative {}", q(&format!("{req}/{name}"))).unwrap();
    }
    if let Some(oauth) = plan.oauth() {
        add(
            format!("lib/{req}/oauth.rb"),
            super::oauth::runtime(oauth, plan.operations()).replace("__NAMESPACE__", ns),
        );
    }
    if let Some(incoming) = plan.incoming()
        && !incoming.is_empty()
    {
        add(
            format!("lib/{req}/incoming.rb"),
            super::incoming::runtime(incoming).replace("__NAMESPACE__", ns),
        );
    }
    if let Some(env) = plan.credential_env() {
        writeln!(
            entry,
            "require_relative {}",
            q(&format!("{req}/credential_env"))
        )
        .unwrap();
        add(
            format!("lib/{req}/credential_env.rb"),
            credential_environment(plan, ns),
        );
        // Only configuration names and physical binding provenance are emitted.
        add(
            format!("lib/{req}/credential-env.json"),
            serde_json::to_string_pretty(env).unwrap() + "\n",
        );
    }
    add(format!("lib/{req}.rb"), entry);
    for (name, code) in [
        ("json", include_str!("json.rb")),
        ("program_guard", include_str!("program_guard.rb")),
        ("resource_guard", include_str!("resource_guard.rb")),
        ("validation", include_str!("validation.rb")),
        ("validation_v2", include_str!("validation_v2.rb")),
        ("validation_v3", include_str!("validation_v3.rb")),
        ("codecs", include_str!("codecs.rb")),
        ("wire", include_str!("wire.rb")),
        ("payload", include_str!("payload.rb")),
        ("streams", include_str!("streams.rb")),
        ("http", include_str!("http.rb")),
    ] {
        add(
            format!("lib/{req}/{name}.rb"),
            code.replace("__NAMESPACE__", ns),
        );
    }
    let c = plan.config();
    let mut policy = format!(
        "# frozen_string_literal: true\nmodule {ns}\n  # @api private\n  module Internal\n    POLICY = {{\n"
    );
    for (k, v) in [
        ("max_request_bytes", c.max_request_bytes),
        ("max_response_bytes", c.max_response_bytes),
        ("max_capture_bytes", c.max_capture_bytes),
        ("max_header_bytes", c.max_header_bytes),
        ("max_url_bytes", c.max_url_bytes),
        ("max_response_chunks", c.max_response_chunks),
        ("max_json_depth", c.max_json_depth),
        ("max_json_work", c.max_json_work),
        ("max_conversion_steps", c.max_conversion_steps),
        ("max_number_bytes", 4096),
        ("max_part_bytes", c.max_part_bytes),
        ("max_parts", c.max_parts),
        ("max_stream_item_bytes", c.max_stream_item_bytes),
        ("max_stream_items", c.max_stream_items),
    ] {
        writeln!(policy, "      {k}: {v},").unwrap();
    }
    policy.push_str("    }.freeze\n  end\nend\n");
    add(format!("lib/{req}/policy.rb"), policy);
    add(
        format!("lib/{req}/attribution.rb"),
        attribution_constants(plan, ns),
    );
    add(
        format!("lib/{req}/validation-program.json"),
        serde_json::to_string(plan.program()).unwrap() + "\n",
    );
    add(
        format!("lib/{req}/http-protocol.json"),
        serde_json::to_string(plan.protocol()).unwrap() + "\n",
    );
    let mut program = format!(
        "# frozen_string_literal: true\nmodule {ns}\n  # @api private\n  module Internal\n    PROGRAM = load_program(::File.binread(::File.join(__dir__, 'validation-program.json'), 16 * 1024 * 1024 + 1))\n    PROTOCOL = freeze_tree(Json.parse(::File.binread(::File.join(__dir__, 'http-protocol.json'), 16 * 1024 * 1024 + 1), max_bytes: 16 * 1024 * 1024, max_work: 128 * 1024 * 1024))\n    SCHEMA_INDICES = {{\n"
    );
    for root in &plan.program().roots {
        writeln!(
            program,
            "      {} => {},",
            q(&format!("{}#{}", root.source.document, root.source.pointer)),
            root.target
        )
        .unwrap();
    }
    program.push_str("    }.freeze\n  end\nend\n");
    add(format!("lib/{req}/program.rb"), program);
    add(format!("lib/{req}/models.rb"), models(plan, ns));
    add(format!("lib/{req}/wire_models.rb"), wire_models(plan, ns));
    add(format!("lib/{req}/client.rb"), client(plan, ns));
    add(format!("sig/{req}.rbs"), signatures(plan, ns));
    add(
        "examples.json".into(),
        http_examples::manifest(plan.examples()),
    );
    add(
        "EXAMPLES.md".into(),
        http_examples::markdown(plan.examples(), "ruby examples/contract_examples.rb"),
    );
    let (samples, quickstart) = examples(plan, package);
    add("examples/contract_examples.rb".into(), samples);
    if let Some(s) = quickstart {
        add("examples/quickstart.rb".into(), s);
    }
    add("source-map.json".into(), source_map(plan, package));
    add("README.md".into(), guide(plan, package));
    add(
        "RUNTIME.md".into(),
        include_str!("GUIDE.md").replace("__NAMESPACE__", ns),
    );
    add(".yardopts".into(),"--markup markdown\n--markup-provider redcarpet\n--no-private\n--fail-on-warning\nlib/**/*.rb\n-\nREADME.md\nRUNTIME.md\nEXAMPLES.md\n".into());
    add(
        format!("{}.gemspec", package.name),
        format!(
            "# frozen_string_literal: true\nGem::Specification.new do |s|\n  s.name = {}\n  s.version = {}\n  s.summary = 'Native Ruby SDK with source-bound HTTP and exact codecs'\n  s.authors = ['SDK package maintainer']\n  s.required_ruby_version = '>= 3.3.12'\n  s.files = Dir['lib/**/*.rb', 'lib/**/*.json', 'sig/**/*.rbs', 'examples/**/*.rb'] + ['README.md','RUNTIME.md','EXAMPLES.md','source-map.json','examples.json','.yardopts']\n  s.require_paths = ['lib']\n  s.add_dependency 'net-http', '>= 0.4.1', '< 1.0'\n  s.add_dependency 'uri', '>= 0.13.3', '< 2.0'\n  s.add_dependency 'openssl', '>= 3.2', '< 5.0'\n  s.add_dependency 'timeout', '>= 0.4.1', '< 1.0'\n  s.add_dependency 'base64', '>= 0.2', '< 1.0'\n  s.add_dependency 'securerandom', '>= 0.3', '< 1.0'\nend\n",
            q(&package.name),
            q(&package.version)
        ),
    );
    files.sort_by(|a, b| a.path.cmp(&b.path));
    files
}

fn credential_environment(plan: &SdkPlan, namespace: &str) -> String {
    let env = plan
        .credential_env()
        .expect("configured environment policy");
    let bindings = env.bindings().iter().map(|binding| {
        // Binding/ambiguity admission belongs to credential_env::plan. Read the
        // already-bound declaration's existing attachment for native byte guards.
        let requirement = plan
            .protocol()
            .operations()
            .iter()
            .flat_map(|operation| operation.security().alternatives())
            .flat_map(|alternative| alternative.requirements())
            .find(|requirement| {
                requirement.name() == binding.name()
                    && requirement.scheme().use_site().source()
                        == binding.scheme().use_site().source()
            })
            .expect("retained bound credential declaration");
        let attachment = match requirement.credential() {
            wire::CredentialHook::Bearer { .. } => ":bearer",
            wire::CredentialHook::ApiKey {
                location: wire::ParameterLocation::Header,
                ..
            } => ":header",
            wire::CredentialHook::ApiKey {
                location: wire::ParameterLocation::Query,
                ..
            } => ":query",
            wire::CredentialHook::ApiKey {
                location: wire::ParameterLocation::Cookie,
                ..
            } => ":cookie",
            _ => unreachable!("shared v1 credential-env string attachment admission"),
        };
        format!(
            "[{}, {}, {attachment}]",
            q(binding.name()),
            q(binding.variable())
        )
    });
    include_str!("credential_env.rb")
        .replace("__NAMESPACE__", namespace)
        .replace("__CREDENTIAL_ENV_BINDINGS__", &list(bindings))
}
/// ua/v1 attribution constants compiled into every generated gem. An empty
/// suspect version is the disabled sentinel the runtime refuses to assemble.
fn attribution_constants(plan: &SdkPlan, ns: &str) -> String {
    let mut out = format!(
        "# frozen_string_literal: true\nmodule {ns}\n  # @api private\n  module Internal\n"
    );
    match plan.attribution() {
        Some(attribution) => {
            writeln!(
                out,
                "    # ua/v1 attribution: every request identifies suspect as the generator and\n    # the SDK or a caller-supplied application as the client.\n    ATTRIBUTION = {{\n      template_version: {},\n      suspect_version: {},\n      sdk_name: {},\n      sdk_version: {},\n      spec_version: {},\n      language: {},\n    }}.freeze",
                q(match attribution.template_version {
                    crate::attribution::AttributionTemplateVersion::V1 => "v1",
                }),
                q(&attribution.suspect_version),
                q(&attribution.sdk_name),
                q(&attribution.sdk_version),
                q(&attribution.spec_version),
                q(&attribution.language),
            )
            .unwrap();
        }
        None => writeln!(
            out,
            "    # An empty suspect version disables the automatic attribution header.\n    ATTRIBUTION = {{template_version: \"\", suspect_version: \"\", sdk_name: \"\", sdk_version: \"\", spec_version: \"\", language: \"\"}}.freeze"
        )
        .unwrap(),
    }
    out.push_str("  end\nend\n");
    out
}
fn models(plan: &SdkPlan, ns: &str) -> String {
    let mut out = format!(
        "# frozen_string_literal: true\nmodule {ns}\n  # @api private\n  module Internal\n    SHAPES = freeze_tree({{\n"
    );
    for s in plan.models().symbols() {
        write!(
            out,
            "      {} => {{source: {}, ",
            s.schema_index,
            q(&src(&s.source))
        )
        .unwrap();
        match &s.shape {
            ModelShape::Json | ModelShape::RefinedJson => out.push_str("kind: :json"),
            ModelShape::Dynamic { initial_target, initial_resource, anchor, candidates } => write!(out,
                "kind: :dynamic, initial_target: {initial_target}, initial_resource: {initial_resource}, anchor: {}, candidates: {}",
                anchor.as_ref().map_or("nil".into(), |name| q(name)), list(candidates.iter().map(usize::to_string))
            ).unwrap(),
            ModelShape::Literal(_) => out.push_str("kind: :literal"),
            ModelShape::Never => out.push_str("kind: :never"),
            ModelShape::Alias(i) => write!(out, "kind: :alias, target: {i}").unwrap(),
            ModelShape::Scalar(k) => write!(
                out,
                "kind: :scalar, types: {}",
                list(k.iter().map(|k| q(match k {
                    ScalarKind::Null => "null",
                    ScalarKind::Boolean => "boolean",
                    ScalarKind::Integer => "integer",
                    ScalarKind::Number => "number",
                    ScalarKind::String => "string",
                })))
            )
            .unwrap(),
            ModelShape::Array {
                items,
                prefix,
                nullable,
            } => write!(
                out,
                "kind: :array, items: {}, prefix: {}, nullable: {nullable}",
                items.map_or("nil".into(), |i| i.to_string()),
                list(prefix.iter().map(usize::to_string))
            )
            .unwrap(),
            ModelShape::Union {
                branches,
                exclusive,
            } => write!(
                out,
                "kind: :union, branches: {}, exclusive: {exclusive}",
                list(branches.iter().map(usize::to_string))
            )
            .unwrap(),
            ModelShape::Object {
                fields,
                extras,
                nullable,
            } => {
                write!(
                    out,
                    "kind: :object, nullable: {nullable}, extras: {}, fields: [",
                    match extras {
                        ExtraFields::Closed => ":closed".into(),
                        ExtraFields::Json => ":json".into(),
                        ExtraFields::Scoped { .. } => ":json".into(),
                        ExtraFields::Typed(i) => i.to_string(),
                    }
                )
                .unwrap();
                for f in fields {
                    write!(
                        out,
                        "{{wire: {}, ivar: {}, target: {}, required: {}}},",
                        q(&f.wire_name),
                        q(&format!("@{}", f.name)),
                        f.schema_index,
                        f.required
                    )
                    .unwrap();
                }
                out.push(']');
            }
        }
        out.push_str("},\n");
    }
    out.push_str("    })\n  end\n  # RBS type-alias namespace.\n  module Types; end\n  # Native JSON keyword models.\n  module Models\n");
    for s in plan.models().symbols() {
        let ModelShape::Object { fields, extras, .. } = &s.shape else {
            continue;
        };
        docs(&mut out, "    ", &s.description, &s.source);
        writeln!(
            out,
            "    # @see Codecs::{}\n    class {} < Model\n      SCHEMA_INDEX = {}",
            s.name, s.name, s.schema_index
        )
        .unwrap();
        let mut args = Vec::new();
        for f in fields {
            docs(&mut out, "      ", &f.description, &f.source);
            writeln!(out,"      # Wire member <code>{}</code>; {}.\n      # @return [{}{}]\n      attr_accessor :{}",esc(&f.wire_name),if f.required{"required"}else{"UNSET preserves omission"},yard_model(plan,f.schema_index),if f.required{""}else{", Unset"},f.name).unwrap();
            args.push(format!(
                "{}:{}",
                f.name,
                if let Some(v) = &f.literal {
                    format!(" {}", literal(v))
                } else if f.required {
                    String::new()
                } else {
                    " UNSET".into()
                }
            ));
        }
        if !matches!(extras, ExtraFields::Closed) {
            out.push_str("      # Additional wire members, validated on encode.\n      attr_accessor :extra_fields\n");
            args.push("extra_fields: {}".into());
        }
        for f in fields {
            writeln!(
                out,
                "      # @param {} [{}{}] source-bound wire member",
                f.name,
                yard_model(plan, f.schema_index),
                if f.required { "" } else { ", Unset" }
            )
            .unwrap();
        }
        writeln!(
            out,
            "      # Construct and validate a source-bound model.\n      def initialize({})",
            args.join(", ")
        )
        .unwrap();
        for f in fields {
            writeln!(out, "        @{} = {}", f.name, f.name).unwrap();
        }
        if !matches!(extras, ExtraFields::Closed) {
            out.push_str("        @extra_fields = extra_fields\n");
        }
        out.push_str("        validate!\n      end\n    end\n");
    }
    out.push_str("  end\n  # @api private\n  module Internal\n    MODEL_CLASSES = {\n");
    for s in plan.models().symbols() {
        if matches!(s.shape, ModelShape::Object { .. }) {
            writeln!(out, "      {} => Models::{},", s.schema_index, s.name).unwrap();
        }
    }
    out.push_str("    }.freeze\n  end\n  # Every codec is bound to its exact source schema.\n  module Codecs\n");
    for s in plan.models().symbols() {
        docs(&mut out, "    ", &s.description, &s.source);
        writeln!(out, "    {} = Codec.new({})", s.name, s.schema_index).unwrap();
    }
    out.push_str("  end\nend\n");
    out
}
fn record_key(record: &super::NativeRecord) -> String {
    format!(
        "{}:{}",
        if matches!(record.binding, RecordBinding::Headers(_)) {
            "headers"
        } else {
            "media"
        },
        src(&record.source)
    )
}
fn wire_models(plan: &SdkPlan, ns: &str) -> String {
    let mut out = format!("# frozen_string_literal: true\nmodule {ns}\n  module Models\n");
    for r in plan.records() {
        docs(
            &mut out,
            "    ",
            "Native part/header record; byte-bearing aggregates are not JSON.",
            &r.source,
        );
        writeln!(
            out,
            "    class {} < WireModel\n      RECORD_KEY = {}.freeze",
            r.name,
            q(&record_key(r))
        )
        .unwrap();
        let mut args = Vec::new();
        for f in &r.fields {
            docs(&mut out, "      ", "", &f.source);
            writeln!(
                out,
                "      # @return [{}{}]\n      attr_accessor :{}",
                yard_value(plan, &f.value_type),
                if f.required { "" } else { ", Unset" },
                f.name
            )
            .unwrap();
            args.push(format!(
                "{}:{}",
                f.name,
                if f.required { "" } else { " UNSET" }
            ));
        }
        if r.additional.is_some() {
            out.push_str("      attr_accessor :extra_fields\n");
            args.push("extra_fields: {}".into());
        }
        for f in &r.fields {
            writeln!(
                out,
                "      # @param {} [{}{}] source-bound wire member",
                f.name,
                yard_value(plan, &f.value_type),
                if f.required { "" } else { ", Unset" }
            )
            .unwrap();
        }
        writeln!(out, "      def initialize({})", args.join(", ")).unwrap();
        for f in &r.fields {
            writeln!(out, "        @{} = {}", f.name, f.name).unwrap();
        }
        if r.additional.is_some() {
            out.push_str("        @extra_fields = extra_fields\n");
        }
        out.push_str("        validate!\n      end\n    end\n");
    }
    out.push_str("  end\n  # @api private\n  module Internal\n    RECORDS = {\n");
    for r in plan.records() {
        let (kind, wire) = match &r.binding {
            RecordBinding::Headers(h) => ("headers", exact(h)),
            RecordBinding::Media(m) => ("media", exact(m)),
        };
        writeln!(out,"      {} => {{class: Models::{}, kind: :{kind}, wire: {wire}, additional: {}, fields: [",q(&record_key(r)),r.name,r.additional.is_some()).unwrap();
        for f in &r.fields {
            writeln!(
                out,
                "        {{wire: {}, ivar: {}, required: {}, source: {}}},",
                q(&f.wire_name),
                q(&format!("@{}", f.name)),
                f.required,
                q(&src(&f.source))
            )
            .unwrap();
        }
        out.push_str("      ]},\n");
    }
    out.push_str(
        "    }\n    RECORDS.each_value { |r| freeze_tree(r) }\n    RECORDS.freeze\n  end\nend\n",
    );
    out
}
const OPTIONS: &[&str] = &[
    "timeout",
    "cancellation",
    "max_response_bytes",
    "max_capture_bytes",
    "content_type",
    "accept",
    "security",
    "server",
    "server_variables",
    "document_url",
];
fn client(plan: &SdkPlan, ns: &str) -> String {
    let mut out = format!("# frozen_string_literal: true\nmodule {ns}\n");
    for op in plan.operations() {
        docs(&mut out, "  ", "Declared API error family.", &op.source);
        writeln!(out, "  class {} < ApiError; end", op.error_class).unwrap();
        for r in &op.responses {
            docs(
                &mut out,
                "  ",
                &format!(
                    "Declared {} response; actual status is retained.",
                    r.status_key
                ),
                &r.source,
            );
            if r.can_succeed() {
                writeln!(
                    out,
                    "  # @!attribute [r] data\n  #   @return [{}]",
                    if r.body_forbidden {
                        "NoContent".into()
                    } else if r.media.is_empty() {
                        "Bytes".into()
                    } else {
                        r.media
                            .iter()
                            .map(|m| yard_value(plan, &m.value_type))
                            .collect::<Vec<_>>()
                            .join(", ")
                    }
                )
                .unwrap();
                writeln!(out, "  class {} < ApiResponse; end", r.class_name).unwrap();
            }
            if r.can_fail() {
                writeln!(
                    out,
                    "  # @!attribute [r] data\n  #   @return [{}]",
                    if r.body_forbidden {
                        "NoContent".into()
                    } else if r.media.is_empty() {
                        "Bytes".into()
                    } else {
                        r.media
                            .iter()
                            .map(|m| yard_value(plan, &m.value_type))
                            .collect::<Vec<_>>()
                            .join(", ")
                    }
                )
                .unwrap();
                writeln!(out, "  class {} < {}; end", r.error_class, op.error_class).unwrap();
            }
        }
    }
    // The typed event classes, completion carriers, events wrappers and the
    // shared raw-envelope framer are emitted only for discriminated declared
    // SSE event streams; every other document stays byte-identical.
    if let Some(events) = plan.stream_events()
        && !events.operations.is_empty()
    {
        out.push_str(&super::stream_events::classes(events));
    }
    out.push_str("  # @api private\n  module Internal\n    OPERATIONS = [\n");
    for (i, op) in plan.operations().iter().enumerate() {
        let stream = op.responses.iter().any(|r| {
            r.media
                .iter()
                .any(|m| matches!(m.value_type, NativeType::Stream(_)))
        });
        writeln!(out,"      {{id: {}, source: {}, wire: PROTOCOL['operations'][{i}], streaming: {stream}, responses: {{",q(&op.operation_id),q(&src(&op.source))).unwrap();
        for r in &op.responses {
            writeln!(
                out,
                "        {} => {{success: {}, error: {}}},",
                q(&r.status_key),
                if r.can_succeed() {
                    r.class_name.as_str()
                } else {
                    "nil"
                },
                if r.can_fail() {
                    r.error_class.as_str()
                } else {
                    "nil"
                }
            )
            .unwrap();
        }
        out.push_str("      }},\n");
    }
    out.push_str("    ].freeze\n  end\n  class Client\n");
    for (i, op) in plan.operations().iter().enumerate() {
        docs(&mut out, "    ", &op.description, &op.source);
        for p in &op.parameters {
            writeln!(
                out,
                "    # @param {} [{}{}] wire parameter <code>{}</code>",
                p.keyword,
                yard_value(plan, &p.value_type),
                if p.required { "" } else { ", Unset" },
                esc(&p.wire_name)
            )
            .unwrap();
        }
        if let Some(b) = &op.body {
            writeln!(
                out,
                "    # @param body [{}{}] source-selected request representation",
                b.media
                    .iter()
                    .map(|m| yard_value(plan, &m.value_type))
                    .collect::<Vec<_>>()
                    .join(", "),
                if b.required { "" } else { ", Unset" }
            )
            .unwrap();
        }
        writeln!(out,"    # {} <code>{}</code>.\n    # @raise [{}] Validated declared errors.\n    # @raise [RequestError, ResponseError, TransportError, TimeoutError, CancelledError, ResourceLimitError]",esc(&op.method),esc(&op.path),op.error_class).unwrap();
        let mut args = op
            .parameters
            .iter()
            .map(|p| format!("{}:{}", p.keyword, if p.required { "" } else { " UNSET" }))
            .collect::<Vec<_>>();
        if let Some(b) = &op.body {
            args.push(format!("body:{}", if b.required { "" } else { " UNSET" }));
        }
        for opt in OPTIONS {
            args.push(format!(
                "{opt}: {}",
                if *opt == "cancellation" {
                    "nil"
                } else {
                    "UNSET"
                }
            ));
        }
        writeln!(
            out,
            "    def {}({})\n      call_operation({i}, {}, {}, {})\n    end",
            op.method_name,
            args.join(", "),
            list(op.parameters.iter().map(|p| p.keyword.clone())),
            if op.body.is_some() { "body" } else { "UNSET" },
            OPTIONS
                .iter()
                .map(|o| format!("{o}: {o}"))
                .collect::<Vec<_>>()
                .join(", ")
        )
        .unwrap();
    }
    if let Some(pagination) = plan.pagination() {
        out.push('\n');
        out.push_str(&super::pagination::client_methods(plan, pagination));
    }
    // The typed events methods and the shared events exchange are emitted only
    // for discriminated declared SSE event streams.
    if let Some(events) = plan.stream_events()
        && !events.operations.is_empty()
    {
        out.push_str(&super::stream_events::client_methods(plan, events));
    }
    out.push_str("  end\nend\n");
    out
}
fn signatures(plan: &SdkPlan, ns: &str) -> String {
    let mut out = include_str!("runtime.rbs").replace("__NAMESPACE__", ns);
    writeln!(out, "\nmodule {ns}\n  module Types").unwrap();
    for s in plan.models().symbols() {
        writeln!(
            out,
            "    # Source: {}\n    type {} = {}",
            esc(&src(&s.source)),
            s.type_name,
            model_type(plan, s)
        )
        .unwrap();
    }
    out.push_str("  end\n  module Models\n");
    for s in plan.models().symbols() {
        let ModelShape::Object { fields, extras, .. } = &s.shape else {
            continue;
        };
        writeln!(
            out,
            "    class {} < Model\n      SCHEMA_INDEX: Integer",
            s.name
        )
        .unwrap();
        let mut args = Vec::new();
        for f in fields {
            let t = format!(
                "{}{}",
                codec_type(plan, f.schema_index),
                if f.required { "" } else { " | Unset" }
            );
            writeln!(out, "      attr_accessor {}: {t}", f.name).unwrap();
            args.push(format!(
                "{}{}: {t}",
                if f.required && f.literal.is_none() {
                    ""
                } else {
                    "?"
                },
                f.name
            ));
        }
        if !matches!(extras, ExtraFields::Closed) {
            let t = if let ExtraFields::Typed(i) = extras {
                codec_type(plan, *i)
            } else {
                "json_value".into()
            };
            writeln!(out, "      attr_accessor extra_fields: Hash[String, {t}]").unwrap();
            args.push(format!("?extra_fields: Hash[String, {t}]"));
        }
        writeln!(
            out,
            "      def initialize: ({}) -> void\n    end",
            args.join(", ")
        )
        .unwrap();
    }
    for r in plan.records() {
        writeln!(
            out,
            "    class {} < WireModel\n      RECORD_KEY: String",
            r.name
        )
        .unwrap();
        let mut args = Vec::new();
        for f in &r.fields {
            let t = format!(
                "{}{}",
                native_type(plan, &f.value_type, true),
                if f.required { "" } else { " | Unset" }
            );
            writeln!(out, "      attr_accessor {}: {t}", f.name).unwrap();
            args.push(format!(
                "{}{}: {t}",
                if f.required { "" } else { "?" },
                f.name
            ));
        }
        if let Some(t) = &r.additional {
            let t = native_type(plan, t, true);
            writeln!(out, "      attr_accessor extra_fields: Hash[String, {t}]").unwrap();
            args.push(format!("?extra_fields: Hash[String, {t}]"));
        }
        writeln!(
            out,
            "      def initialize: ({}) -> void\n    end",
            args.join(", ")
        )
        .unwrap();
    }
    out.push_str("  end\n  module Codecs\n");
    for s in plan.models().symbols() {
        writeln!(out, "    {}: Codec[Types::{}]", s.name, s.type_name).unwrap();
    }
    out.push_str("  end\n");
    for op in plan.operations() {
        let err = op
            .responses
            .iter()
            .filter(|r| r.can_fail())
            .map(|r| response_type(plan, r))
            .collect::<Vec<_>>();
        writeln!(
            out,
            "  class {} < ApiError[{}]\n  end",
            op.error_class,
            if err.is_empty() {
                "bot".into()
            } else {
                err.join(" | ")
            }
        )
        .unwrap();
        for r in &op.responses {
            if r.can_succeed() {
                writeln!(
                    out,
                    "  class {} < ApiResponse[{}]",
                    r.class_name,
                    response_type(plan, r)
                )
                .unwrap();
                if let Some(h) = &r.headers_class {
                    writeln!(out, "    attr_reader typed_headers: Models::{h}").unwrap();
                }
                out.push_str("  end\n");
            }
            if r.can_fail() {
                writeln!(
                    out,
                    "  class {} < {}\n    attr_reader data: {}",
                    r.error_class,
                    op.error_class,
                    response_type(plan, r)
                )
                .unwrap();
                if let Some(h) = &r.headers_class {
                    writeln!(out, "    attr_reader typed_headers: Models::{h}").unwrap();
                }
                out.push_str("  end\n");
            }
        }
    }
    // The typed event RBS declarations are emitted only for discriminated
    // declared SSE event streams.
    if let Some(events) = plan.stream_events()
        && !events.operations.is_empty()
    {
        out.push_str(&super::stream_events::signatures(events));
    }
    out.push_str("  class Client\n");
    for op in plan.operations() {
        let mut args = op
            .parameters
            .iter()
            .map(|p| {
                format!(
                    "{}{}: {}{}",
                    if p.required { "" } else { "?" },
                    p.keyword,
                    native_type(plan, &p.value_type, true),
                    if p.required { "" } else { " | Unset" }
                )
            })
            .collect::<Vec<_>>();
        if let Some(b) = &op.body {
            let types = b
                .media
                .iter()
                .map(|m| native_type(plan, &m.value_type, true))
                .collect::<Vec<_>>()
                .join(" | ");
            args.push(format!(
                "{}body: {types}{}",
                if b.required { "" } else { "?" },
                if b.required { "" } else { " | Unset" }
            ));
        }
        args.extend(
            [
                "?timeout: Float | Integer | Unset",
                "?cancellation: CancellationToken?",
                "?max_response_bytes: Integer | Unset",
                "?max_capture_bytes: Integer | Unset",
                "?content_type: String | Unset",
                "?accept: String | Unset",
                "?security: Integer | Unset",
                "?server: Integer | String | Unset",
                "?server_variables: Hash[String, String] | Unset",
                "?document_url: String | Unset",
            ]
            .map(str::to_owned),
        );
        let returns = op
            .responses
            .iter()
            .filter(|r| r.can_succeed())
            .map(|r| r.class_name.clone())
            .collect::<Vec<_>>();
        writeln!(
            out,
            "    def {}: ({}) -> {}",
            op.method_name,
            args.join(", "),
            if returns.is_empty() {
                "bot".into()
            } else {
                format!("({})", returns.join(" | "))
            }
        )
        .unwrap();
    }
    if let Some(pagination) = plan.pagination() {
        out.push_str(&super::pagination::signatures(plan, pagination));
    }
    // The typed events method signatures are emitted only for discriminated
    // declared SSE event streams.
    if let Some(events) = plan.stream_events()
        && !events.operations.is_empty()
    {
        out.push_str(&super::stream_events::client_signatures(events));
    }
    if let Some(oauth) = plan.oauth() {
        out.push_str(&super::oauth::signatures(oauth));
    }
    if let Some(incoming) = plan.incoming()
        && !incoming.is_empty()
    {
        out.push_str(&super::incoming::signatures(incoming));
    }
    out.push_str("  end\nend\n");
    out
}
fn sample(plan: &SdkPlan, v: &SampleValue, ns: &str) -> String {
    match v {
        SampleValue::Json(Value::Number(n)) => {
            format!("{ns}::JsonNumber.new({})", q(&n.to_string()))
        }
        SampleValue::Json(Value::Array(_) | Value::Object(_)) => {
            if let SampleValue::Json(v) = v {
                format!("{ns}::Json.parse({})", q(&v.to_string()))
            } else {
                unreachable!()
            }
        }
        SampleValue::Json(v) => literal(v),
        SampleValue::Array(v) => list(v.iter().map(|v| sample(plan, v, ns))),
        SampleValue::Decoded {
            schema_index,
            value,
        } => format!(
            "{ns}::Codecs::{}.decode_json({})",
            plan.models().symbol(*schema_index).unwrap().name,
            q(&value.to_string())
        ),
        SampleValue::Object {
            schema_index,
            fields,
            extras,
        } => {
            let mut args = fields
                .iter()
                .map(|(k, v)| format!("{k}: {}", sample(plan, v, ns)))
                .collect::<Vec<_>>();
            if !extras.is_empty() {
                args.push(format!(
                    "extra_fields: {{{}}}",
                    extras
                        .iter()
                        .map(|(k, v)| format!("{} => {}", q(k), sample(plan, v, ns)))
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            }
            format!(
                "{ns}::Models::{}.new({})",
                plan.models().symbol(*schema_index).unwrap().name,
                args.join(", ")
            )
        }
    }
}
fn examples(plan: &SdkPlan, p: &PackageConfig) -> (String, Option<String>) {
    let ns = &p.namespace;
    let mut out = format!(
        "# frozen_string_literal: true\n# Validated source examples; all exchanges are offline.\nrequire {}\n",
        q(&p.require_name)
    );
    let bindings = http_examples::bindings(plan.examples());
    let mut responses = Vec::new();
    let mut calls = Vec::new();
    let mut quick = None;
    for (oi, op) in plan.operations().iter().enumerate() {
        let Some(binding) = bindings.get(&op.source) else {
            continue;
        };
        for (i, e) in binding.operation.entries.iter().enumerate() {
            let Some(s) = plan.models().source_symbol(&e.schema) else {
                continue;
            };
            let n = plan
                .native_examples()
                .iter()
                .find(|n| n.operation_source == op.source && n.entry_index == i)
                .unwrap();
            writeln!(out,"# {} example: {}\nvalue_{oi}_{i} = {}\n{ns}::Codecs::{}.encode_json(value_{oi}_{i})",http_examples::origin(&e.origin),esc(&src(&e.schema)),sample(plan,&n.value,ns),s.name).unwrap();
        }
        // Complete ordinary JSON/text calls use actual slots. Parts/items remain
        // independently executable codec examples; bytes need explicit fixtures.
        let b = op.body.as_ref();
        let body_media = b.and_then(|b| {
            (b.media.len() == 1 && matches!(b.media[0].value_type, NativeType::Codec(_)))
                .then_some(&b.media[0])
        });
        if b.is_some_and(|b| b.required) && body_media.is_none() {
            continue;
        }
        let slots = op
            .parameters
            .iter()
            .map(|p| http_examples::InputSlot {
                name: &p.keyword,
                required: p.required,
                container: p.source.clone(),
            })
            .chain(body_media.into_iter().map(|m| http_examples::InputSlot {
                name: "body",
                required: b.unwrap().required,
                container: m.source.clone(),
            }));
        let Some(mut inputs) = binding.bind(slots) else {
            continue;
        };
        let mut unavailable = false;
        inputs.retain(|input| {
            if let Some(p) = op.parameters.iter().find(|p| p.keyword == input.name) {
                let value = &binding.operation.entries[input.example_index].value;
                if p.wire.serialize(value).is_err() || matches!(p.value_type, NativeType::Record(_))
                {
                    if input.required {
                        unavailable = true;
                    }
                    return false;
                }
            }
            true
        });
        if unavailable {
            continue;
        }
        // Credential acquisition/token-type choices are caller fixture inputs.
        // Do not fabricate them when lowering a schema-only source example.
        let alternatives = op.wire.security().alternatives();
        if alternatives.len() > 1
            || alternatives
                .iter()
                .flat_map(|a| a.requirements())
                .any(|r| !matches!(r.credential(), wire::CredentialHook::Bearer { .. }))
        {
            continue;
        }
        let response = binding.operation.entries.iter().find(|e| {
            matches!(e.role, ExampleRole::Response { status: 200..=299 })
                && op.responses.iter().any(|r| {
                    r.status_key
                        == match &e.role {
                            ExampleRole::Response { status } => status.to_string(),
                            _ => String::new(),
                        }
                        && r.headers.iter().all(|h| !h.required)
                        && r.media.iter().any(|m| {
                            matches!(m.value_type, NativeType::Codec(_))
                                && m.wire.media_type().declared() == e.media_type
                        })
                })
        });
        let Some(response) = response else { continue };
        let ExampleRole::Response { status } = response.role else {
            unreachable!()
        };
        let response_text = if response.media_type.starts_with("text/") {
            match &response.value {
                Value::String(s) => s.clone(),
                v => v.to_string(),
            }
        } else {
            response.value.to_string()
        };
        responses.push(format!(
            "{} => [{status}, {}, {}]",
            q(&op.operation_id),
            q(&response.media_type),
            q(&response_text)
        ));
        let args = inputs
            .iter()
            .map(|i| format!("{}: value_{oi}_{}", i.name, i.example_index))
            .collect::<Vec<_>>()
            .join(", ");
        calls.push(format!("  client.{}({args})", op.method_name));
        if quick.is_none() && op.body.is_some() {
            let args = inputs
                .iter()
                .map(|i| {
                    let n = plan
                        .native_examples()
                        .iter()
                        .find(|n| {
                            n.operation_source == op.source && n.entry_index == i.example_index
                        })
                        .unwrap();
                    format!("{}: {}", i.name, sample(plan, &n.value, ns))
                })
                .collect::<Vec<_>>()
                .join(", ");
            quick = Some(format!("client.{}({args})", op.method_name));
        }
    }
    writeln!(out,"class ContractExampleTransport\n  attr_reader :calls\n  def initialize\n    @calls = []\n    @responses = {{{}}}\n  end\n  def exchange(request:, context:)\n    context.check!\n    @calls << request.operation_id\n    status, media, bytes = @responses.fetch(request.operation_id)\n    yield {ns}::WireResponse.new(status: status, headers: {{'Content-Type' => media}}, body: bytes)\n  end\nend",responses.join(", ")).unwrap();
    let auth = plan
        .operations()
        .iter()
        .flat_map(|op| op.wire.security().alternatives())
        .flat_map(|a| a.requirements())
        .filter(|r| matches!(r.credential(), wire::CredentialHook::Bearer { .. }))
        .map(|r| r.name())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .map(|n| format!("{} => 'example-token'", q(n)))
        .collect::<Vec<_>>()
        .join(", ");
    writeln!(out,"transport = ContractExampleTransport.new\n{ns}::Client.open(auth: {{{auth}}}, transport: transport, server_url: 'http://127.0.0.1') do |client|\n{}\nend\nraise 'missing example exchanges' unless transport.calls.length == {}\nputs 'Source-bound Ruby examples passed ({} offline exchanges)'",calls.join("\n"),calls.len(),calls.len()).unwrap();
    let quick=quick.map(|call|format!("# frozen_string_literal: true\nrequire {}\nmodule {ns}\n  module Quickstart\n    def self.run(auth:, transport: nil, server_url: nil)\n      Client.open(auth: auth, transport: transport, server_url: server_url) do |client|\n        {call}\n      end\n    end\n  end\nend\n",q(&p.require_name)));
    if quick.is_some() {
        writeln!(out,"require_relative 'quickstart'\n{ns}::Quickstart.run(auth: {{{auth}}}, transport: transport, server_url: 'http://127.0.0.1')\nputs 'Native keyword quickstart passed through the installed gem'").unwrap();
    }
    (out, quick)
}
fn source_map(plan: &SdkPlan, p: &PackageConfig) -> String {
    format!("{}\n",serde_json::to_string_pretty(&json!({"version":"suspect.ruby-native-plan.v2","protocolVersion":plan.protocol().version(),"adapter":plan.protocol().capabilities(),"package":{"name":p.name,"require":p.require_name,"namespace":p.namespace},"models":plan.models().symbols().map(|s|json!({"source":src(&s.source),"schemaIndex":s.schema_index,"name":s.name,"typeName":s.type_name})).collect::<Vec<_>>(),"records":plan.records().iter().map(|r|json!({"source":src(&r.source),"name":r.name,"fields":r.fields.iter().map(|f|json!({"name":f.name,"wire":f.wire_name,"required":f.required,"type":native_type(plan,&f.value_type,true)})).collect::<Vec<_>>()})).collect::<Vec<_>>(),"operations":plan.operations().iter().map(|o|json!({"source":src(&o.source),"operationId":o.operation_id,"method":o.method_name,"body":o.body.as_ref().map(|b|b.media.iter().map(|m|json!({"media":m.wire.media_type().declared(),"type":native_type(plan,&m.value_type,true)})).collect::<Vec<_>>()),"responses":o.responses.iter().map(|r|json!({"status":r.status_key,"class":r.class_name,"errorClass":r.error_class,"type":response_type(plan,r)})).collect::<Vec<_>>()})).collect::<Vec<_>>()})).unwrap())
}
fn guide(plan: &SdkPlan, p: &PackageConfig) -> String {
    let ns = &p.namespace;
    let (_, quick) = examples(plan, p);
    let snippet = quick
        .as_deref()
        .and_then(|q| q.lines().find(|l| l.trim_start().starts_with("client.")))
        .unwrap_or("        # Choose a source operation from the reference below.")
        .trim();
    let mut out = format!(
        "# {}\n\nNative Ruby keyword models and HTTP calls. Requires Ruby 3.3.12+.\n\n## Install\n\n```sh\ngem build {}.gemspec\ngem install --local {}-{}.gem\n```\n\n## First request\n\nSupply credentials explicitly under the names declared by your API. A complete source-derived model call is shown below; values and origins are retained in `examples.json`.\n\n```ruby\nrequire {}\n{ns}::Client.open(auth: credentials) do |client|\n  response = {snippet}\n  puts response.status\nend\n```\n\nAnonymous operations accept `Client.new`. Multiple security alternatives require `security: index`; conjunctive credentials are all applied. Supply `BasicCredential` for Basic and an explicit `AuthorizationCredential` via `credential_provider:` for OAuth/OIDC. The provider receives source, scheme, permissions and flow/discovery metadata. It owns acquisition and refresh.\n\nChoose multiple/relative servers with `server:`, `server_variables:` and `document_url:`. An explicit `server_url:` overrides the source choice. Choose request media with `content_type:` and response preference with `accept:`. The selected response uses exact > range > default status precedence and concrete > wildcard media precedence, preserving actual status.\n\nUse `UNSET` for omission and `nil` only for source-permitted JSON null. Byte bodies use `Bytes`; `BytePart.new(bytes:, filename:, content_type:, headers:)` supplies actual bytes and MIME metadata. Forms and multipart have generated keyword body models. `response.typed_headers` holds decoded header models; `response.links` is immutable metadata and invokes nothing.\n\nA sequential response's `data` is an `ItemStream` Enumerator. Use `each` for automatic cleanup, or `next` with explicit `close`. Cancellation and the total deadline remain active while paused. SSE data stays text, retry is metadata, and `[DONE]` is ordinary data.\n\n## Verify and explore\n\n```sh\ngem install yard:0.9.37 redcarpet:3.6.1 rbs:3.9.5 steep:1.10.0\nruby examples/contract_examples.rb\nyard doc\nrbs -I sig validate\n```\n\nInstalled Steep consumers use `library '{}'`. `RUNTIME.md` documents values, failures, bounds, transport and streaming. `EXAMPLES.md` retains example origins/findings; `source-map.json` and the native YARD reference retain detailed bindings.\n\n## Operations\n\n",
        p.name,
        p.name,
        p.name,
        p.version,
        q(&p.require_name),
        p.name
    );
    for o in plan.operations() {
        writeln!(
            out,
            "- `{}` — `{}` `{}`. {}",
            o.method_name,
            esc(&o.method),
            esc(&o.path),
            esc(&o.description)
        )
        .unwrap();
    }
    if let Some(env) = plan.credential_env() {
        writeln!(out, "\n## Runtime environment credentials\n\nThis package has an explicit `credential_env` v1 policy. `{}::Client.new` and `{}::Client.open` snapshot the mapped variables at construction only when `auth:` is omitted. Import and generation do not read their values.\n", p.namespace, p.namespace).unwrap();
        for binding in env.bindings() {
            writeln!(
                out,
                "- Scheme <code>{}</code> reads variable <code>{}</code> ({:?}).",
                esc(binding.name()),
                esc(binding.variable()),
                binding.kind()
            )
            .unwrap();
        }
        out.push_str("\nThe entire explicit `auth:` argument wins. Empty hashes and missing members are not supplemented; explicit `nil` keeps the existing `ArgumentError`. Missing, empty, invalid or unavailable environment values stay missing. Anonymous calls remain usable; a protected operation without its required credential raises a secret-free `RequestError` before transport. OR/AND and explicit `security:` selection keep their source rules. Changing the environment affects a new client, not one already created.\n");
        if let Some(operation) = plan
            .operations()
            .iter()
            .find(|operation| operation.operation_id == "getCurrentKey")
            .or_else(|| {
                plan.operations().iter().find(|operation| {
                    operation.parameters.iter().all(|p| !p.required)
                        && operation.body.as_ref().is_none_or(|b| !b.required)
                })
            })
        {
            writeln!(out, "\n```ruby\nrequire {}\n{}::Client.open do |client|\n  response = client.{}\n  puts response.status\nend\n```\n\nThe call uses the source-declared server when no override is supplied. Binding names and physical provenance are recorded in `lib/{}/credential-env.json`; credential values are never packaged.", q(&p.require_name), p.namespace, operation.method_name, p.require_name).unwrap();
        }
    }
    out
}
