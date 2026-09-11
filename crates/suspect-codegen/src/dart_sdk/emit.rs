//! Package/model rendering from retained Dart descriptors and checked instructions.
use super::{
    Plan, http_emit,
    models::{DartModel, Extras, Shape},
    validation,
};
use crate::OutFile;
use serde_json::Value;
use std::fmt::Write;

pub(super) fn quote(value: &str) -> String {
    serde_json::to_string(value)
        .unwrap()
        .replace('$', "\\$")
        .replace('\u{2028}', "\\u2028")
        .replace('\u{2029}', "\\u2029")
}
pub(super) fn literal(value: &Value) -> String {
    match value {
        Value::Null => "const JsonNull()".into(),
        Value::Bool(v) => format!("const JsonBoolean({v})"),
        Value::Number(v) => format!(
            "JsonNumber.parse({}, maxBytes: 65536)",
            quote(&v.to_string())
        ),
        Value::String(v) => format!("JsonString({})", quote(v)),
        Value::Array(v) => format!(
            "JsonArray([{}])",
            v.iter().map(literal).collect::<Vec<_>>().join(", ")
        ),
        Value::Object(v) => format!(
            "JsonObject({{{}}})",
            v.iter()
                .map(|(k, v)| format!("{}: {}", quote(k), literal(v)))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}
fn doc(out: &mut String, value: &str, indent: &str) {
    let value = value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('[', "&#91;")
        .replace(']', "&#93;")
        .replace('`', "&#96;");
    for line in value.replace(['\r', '\u{2028}', '\u{2029}'], " ").lines() {
        writeln!(out, "{indent}/// {line}").unwrap();
    }
}
pub(super) fn package(plan: &Plan) -> Vec<OutFile> {
    let name = &plan.config.package.name;
    let version = &plan.config.package.version;
    let mut files = Vec::new();
    let mut push = |path: &str, content: String| {
        files.push(OutFile {
            path: format!("dart/{path}"),
            content,
        })
    };
    push(
        "pubspec.yaml",
        format!(
            "name: {name}\nversion: {version}\ndescription: Source-selected exact-value native OpenAPI SDK.\nenvironment:\n  sdk: '>=3.9.4 <4.0.0'\n"
        ),
    );
    push("analysis_options.yaml","analyzer:\n  language:\n    strict-casts: true\n    strict-inference: true\n    strict-raw-types: true\n  errors:\n    unused_import: error\n    unused_local_variable: error\n    unused_element: error\n    dead_code: error\n".into());
    let mut library = format!(
        "/// Exact native models, Future/Stream operations and portable transport.\nlibrary {name};\nimport 'dart:async' hide TimeoutException;\nimport 'dart:collection';\nimport 'dart:convert' show utf8, base64Encode;\nimport 'dart:typed_data';\n"
    );
    if plan.credential_env().is_some() {
        library.push_str("import 'src/environment_stub.dart' if (dart.library.io) 'src/environment_io.dart' as _environment;\n");
    }
    for part in [
        "json",
        "validation",
        "program",
        "codec",
        "models",
        "wire",
        "url",
        "auth",
        "forms",
        "framing",
        "transport",
        "protocol",
        "client",
    ] {
        writeln!(library, "part 'src/{part}.dart';").unwrap();
    }
    if plan.credential_env().is_some() {
        library.push_str("part 'src/environment.dart';\n");
        push(
            "lib/src/environment.dart",
            format!(
                "// Generated from the bound runtime credential policy.\npart of '../{name}.dart';\n\n{}",
                super::environment::render(plan)
            ),
        );
        push(
            "lib/src/environment_io.dart",
            include_str!("environment_io.dart").into(),
        );
        push(
            "lib/src/environment_stub.dart",
            include_str!("environment_stub.dart").into(),
        );
    }
    push(&format!("lib/{name}.dart"), library);
    for (part, source) in [
        ("json", include_str!("json.dart").into()),
        (
            "validation",
            validation::runtime(&plan.program)
                .expect("immutable checked program")
                .into(),
        ),
        (
            "program",
            validation::render(&plan.program).expect("immutable checked program"),
        ),
        (
            "codec",
            format!("{}\n{}", codec_config(plan), include_str!("codec.dart")),
        ),
        ("models", models(plan)),
        ("wire", include_str!("wire.dart").into()),
        ("url", include_str!("url.dart").into()),
        ("auth", include_str!("auth.dart").into()),
        ("forms", include_str!("forms.dart").into()),
        ("framing", include_str!("framing.dart").into()),
        ("transport", include_str!("transport.dart").into()),
        ("protocol", http_emit::data(plan)),
        ("client", http_emit::client(plan)),
    ] {
        push(
            &format!("lib/src/{part}.dart"),
            format!(
                "// Generated from canonical source and checked plans.\npart of '../{name}.dart';\n\n{source}"
            ),
        );
    }
    push(
        &format!("lib/{name}_io.dart"),
        format!(
            "/// Optional standard-library VM transport.\nlibrary;\nimport 'dart:async';\nimport 'dart:convert' show latin1;\nimport 'dart:io';\nimport 'dart:typed_data';\nimport '{name}.dart';\nexport '{name}.dart';\n{}\n{}",
            include_str!("io_transport.dart"),
            include_str!("io_exact.dart")
        ),
    );
    push(
        "README.md",
        format!(
            "{}{}{}",
            include_str!("README.md")
                .replace("__PACKAGE__", name)
                .replace("__VERSION__", version)
                .replace("__OPENAPI__", plan.contract.openapi_version())
                .replace("__QUICKSTART__", http_emit::quickstart(plan).trim_end()),
            if plan.program.version == suspect_schema::OwnedProgram::V3_VERSION {
                format!(
                    "\n{}",
                    include_str!("RESOURCE-VALIDATION.md").replacen(
                        "# Resource and dynamic validation",
                        "## Resource and dynamic validation",
                        1
                    )
                )
            } else if plan.program.version == suspect_schema::OwnedProgram::V2_VERSION {
                format!(
                    "\n{}",
                    include_str!("SCOPED-VALIDATION.md").replacen(
                        "# Scoped schema validation",
                        "## Scoped schema validation",
                        1
                    )
                )
            } else {
                String::new()
            },
            if plan.credential_env().is_some() {
                format!("\n{}", super::environment::guide(plan))
            } else {
                String::new()
            }
        ),
    );
    push(
        "CHANGELOG.md",
        format!("# {version}\n\nGenerated source-selected native SDK; see sdk-manifest.json.\n"),
    );
    push("dartdoc_options.yaml","dartdoc:\n  showUndocumentedCategories: true\n  linkToSource:\n    excludes: ['**/*.dart']\n".into());
    push("doc/API.md", http_emit::reference(plan));
    if plan.credential_env().is_some() {
        push("doc/CREDENTIAL-ENV.md", super::environment::guide(plan));
    }
    if plan.program.version == suspect_schema::OwnedProgram::V2_VERSION {
        push(
            "doc/SCOPED-VALIDATION.md",
            include_str!("SCOPED-VALIDATION.md").into(),
        );
    }
    if plan.program.version == suspect_schema::OwnedProgram::V3_VERSION {
        push(
            "doc/RESOURCE-VALIDATION.md",
            include_str!("RESOURCE-VALIDATION.md").into(),
        );
    }
    push(
        "doc/validation-program.json",
        format!("{}\n", serde_json::to_string_pretty(&plan.program).unwrap()),
    );
    push("example/source_examples.dart", http_emit::examples(plan));
    push("example/quickstart.dart", http_emit::quickstart(plan));
    push("sdk-manifest.json", http_emit::manifest(plan));
    files.sort_by(|a, b| a.path.cmp(&b.path));
    files
}
fn codec_config(plan: &Plan) -> String {
    let c = &plan.config;
    format!(
        "const _decodeLimits = JsonLimits(maxBytes: {}, maxDepth: {}, maxSteps: {}, maxNumberBytes: {});\nconst _encodeLimits = JsonLimits(maxBytes: {}, maxDepth: {}, maxSteps: {}, maxNumberBytes: {});\nconst _maxConversionDepth = {};\nconst _maxConversionSteps = {};\n",
        c.max_response_bytes.max(c.max_request_bytes),
        c.max_json_depth,
        c.max_json_steps,
        c.schema.max_number_bytes,
        c.max_request_bytes,
        c.max_json_depth,
        c.max_json_steps,
        c.schema.max_number_bytes,
        c.max_conversion_depth,
        c.max_conversion_steps
    )
}
fn extra_target(value: Extras) -> Option<usize> {
    match value {
        Extras::Typed(index) => Some(index),
        _ => None,
    }
}
fn extra_type(plan: &Plan, value: Extras) -> String {
    plan.models.optional_ty(extra_target(value))
}
fn decode_expr(target: Option<usize>, value: &str) -> String {
    target.map_or_else(
        || format!("c.json({value})"),
        |index| format!("_decode{index}({value}, c)"),
    )
}
fn encode_expr(target: Option<usize>, value: &str) -> String {
    target.map_or_else(
        || format!("c.json({value})"),
        |index| format!("_encode{index}({value}, c)"),
    )
}
fn nullable(plan: &Plan, model: &DartModel) -> bool {
    plan.models.uses_native_null(model.index)
}
fn models(plan: &Plan) -> String {
    let mut out = String::new();
    if plan.credential_env().is_some() && plan.models.symbols().iter().any(|m| m.deprecated) {
        // Required deprecated source types still participate in generated codecs.
        // Keep the unconfigured emitter's existing bytes and public annotations.
        out.push_str("// Generated codecs must reference required deprecated source types.\n// ignore_for_file: deprecated_member_use_from_same_package\n");
    }
    for model in plan.models.symbols() {
        let name = &model.name;
        doc(&mut out, &model.description, "");
        doc(
            &mut out,
            &format!(
                "Source: {}#{}",
                model.source.document(),
                model.source.pointer()
            ),
            "",
        );
        if model.deprecated {
            out.push_str("@Deprecated('Deprecated by the source contract')\n");
        }
        match &model.shape {
            Shape::Object { fields, extras } => {
                writeln!(
                    out,
                    "final class {name}{} {{",
                    if model.parents.is_empty() {
                        String::new()
                    } else {
                        format!(" implements {}", model.parents.join(", "))
                    }
                )
                .unwrap();
                for field in fields {
                    let ty = plan.models.optional_ty(field.target);
                    if let Some(target) = field.target {
                        doc(&mut out, &plan.models.symbols()[target].description, "  ");
                    }
                    doc(
                        &mut out,
                        &format!(
                            "Wire member {:?}; {}.",
                            field.wire_name,
                            if field.required {
                                "required"
                            } else {
                                "explicit absence/null"
                            }
                        ),
                        "  ",
                    );
                    if let Some(value) = &field.fixed {
                        writeln!(
                            out,
                            "  {ty} get {} => {};",
                            field.name,
                            field.target.map_or_else(
                                || literal(value),
                                |index| native_value(plan, index, value)
                            )
                        )
                        .unwrap();
                    } else {
                        writeln!(
                            out,
                            "  {} {};",
                            if field.required {
                                ty
                            } else {
                                format!("Presence<{ty}>")
                            },
                            field.name
                        )
                        .unwrap();
                    }
                }
                if !matches!(extras, Extras::Closed) {
                    let note = if matches!(extras, Extras::Checked) {
                        "Pattern/scoped wire fields; complete-object codecs check every value."
                    } else {
                        "Additional wire fields; validated again when encoded."
                    };
                    writeln!(
                        out,
                        "  /// {note}\n  final Map<String,{}> extraFields;",
                        extra_type(plan, *extras)
                    )
                    .unwrap();
                }
                if matches!(extras, Extras::Closed) && fields.iter().all(|f| f.fixed.is_some()) {
                    writeln!(out, "  {name}();").unwrap();
                } else {
                    writeln!(out, "  {name}({{").unwrap();
                    for field in fields.iter().filter(|f| f.fixed.is_none()) {
                        writeln!(
                            out,
                            "    {},",
                            if field.required {
                                format!("required this.{}", field.name)
                            } else {
                                format!("this.{} = const Absent()", field.name)
                            }
                        )
                        .unwrap();
                    }
                    if matches!(extras, Extras::Closed) {
                        out.push_str("  });\n");
                    } else {
                        let ty = extra_type(plan, *extras);
                        writeln!(out,"    Map<String,{ty}>? extraFields,\n  }}) : extraFields = Map.of(extraFields ?? <String,{ty}>{{}});").unwrap();
                    }
                }
                out.push_str("}\n");
            }
            Shape::Enum(values) => {
                writeln!(out, "enum {name} {{").unwrap();
                for (wire, member) in values {
                    doc(&mut out, &format!("Wire value {wire:?}."), "  ");
                    writeln!(out, "  {member}({}),", quote(wire)).unwrap();
                }
                writeln!(
                    out,
                    "  ;\n  const {name}(this.wireValue);\n  final String wireValue;\n}}"
                )
                .unwrap();
            }
            Shape::Union {
                targets,
                direct,
                variants,
            } => {
                writeln!(out, "sealed class {name} {{ const {name}._(); }}").unwrap();
                if !direct {
                    for (target, variant) in targets.iter().zip(variants) {
                        writeln!(out,"final class {variant} extends {name} {{\n  {} value;\n  {variant}(this.value) : super._();\n}}",plan.models.ty(*target)).unwrap();
                    }
                }
            }
            Shape::Checked => {
                writeln!(out,"final class {name} {{\n  {name}._(this.value);\n  final JsonValue value;\n  factory {name}.fromJson(JsonValue value) => {}.fromJson(value);\n}}",model.codec_name).unwrap();
            }
            _ => {
                writeln!(out, "typedef {name} = {};", plan.models.ty(model.index)).unwrap();
            }
        }
        writeln!(out,"/// Source-bound codec; every encode validates current mutable values.\nfinal {} = ModelCodec<{}>._({}, {}, _decode{}, _encode{});\n",model.codec_name,plan.models.ty(model.index),validation::source(&plan.program.nodes[model.index].source),model.index,model.index,model.index).unwrap();
        decode(plan, model, &mut out);
        encode(plan, model, &mut out);
    }
    out
}
fn decode(plan: &Plan, model: &DartModel, out: &mut String) {
    let i = model.index;
    let ty = plan.models.ty(i);
    writeln!(
        out,
        "{ty} _decode{i}(JsonValue value, _Conversion c) => c.nest(() {{"
    )
    .unwrap();
    if nullable(plan, model) {
        out.push_str("  if (value is JsonNull) { return null; }\n");
    }
    match &model.shape{
        Shape::Any|Shape::Contextual=>out.push_str("  return c.json(value);\n"),Shape::Never=>out.push_str("  return c.fail('false schema has no native value');\n"),Shape::Null=>out.push_str("  return null;\n"),
        Shape::Boolean=>out.push_str("  return (value as JsonBoolean).value;\n"),Shape::String=>out.push_str("  return c.string((value as JsonString).value);\n"),
        Shape::Number=>out.push_str("  c.json(value); return value as JsonNumber;\n"),Shape::Integer=>out.push_str("  c.json(value); return JsonInteger.parse((value as JsonNumber).token, maxBytes: _decodeLimits.maxNumberBytes);\n"),
        Shape::Alias(target)=>{writeln!(out,"  return _decode{target}(value, c){};",if plan.models.ty(*target).ends_with('?')&&!ty.ends_with('?'){"!"}else{""}).unwrap();}
        Shape::Array(target)=>{writeln!(out,"  final items = (value as JsonArray).values;\n  final result = <{}>[];\n  for (var n=0; n<items.length; n++) {{ result.add(c.child('$n', () => {})); }}\n  return result;",plan.models.optional_ty(*target),decode_expr(*target,"items[n]")).unwrap();}
        Shape::Object{fields,extras}=>{
            if !matches!(extras,Extras::Closed)||fields.iter().any(|f|f.fixed.is_none()){out.push_str("  final object = (value as JsonObject).values;\n");}
            writeln!(out,"  return {}(",model.name).unwrap();
            for f in fields.iter().filter(|f|f.fixed.is_none()){
                let key=quote(&f.wire_name);let expr=format!("c.child({key}, () => {})",decode_expr(f.target,&format!("object[{key}]!")));
                writeln!(out,"    {}: {},",f.name,if f.required{expr}else{format!("object.containsKey({key}) ? Present({expr}) : const Absent()")}).unwrap();
            }
            if !matches!(extras,Extras::Closed){writeln!(out,"    extraFields: <String,{}>{{ for (final entry in object.entries) if (!const <String>[{}].contains(entry.key)) c.string(entry.key): c.child(entry.key, () => {}) }},",extra_type(plan,*extras),fields.iter().map(|f|quote(&f.wire_name)).collect::<Vec<_>>().join(", "),decode_expr(extra_target(*extras),"entry.value")).unwrap();}
            out.push_str("  );\n");
        }
        Shape::Enum(values)=>{out.push_str("  final wire = (value as JsonString).value;\n");for(wire,member)in values{writeln!(out,"  if (wire == {}) {{ return {}.{member}; }}",quote(wire),model.name).unwrap();}out.push_str("  return c.fail('unknown enum value');\n");}
        Shape::Union{targets,direct,variants}=>{
            for(n,target)in targets.iter().enumerate(){writeln!(out,"  if (c.matches({target}, value)) {{ return {}; }}",if *direct{format!("_decode{target}(value,c)")}else{format!("{}(_decode{target}(value,c))",variants[n])}).unwrap();}
            out.push_str("  return c.fail('no union alternative accepted');\n");
        }
        Shape::Checked=>{writeln!(out,"  return {}._(c.json(value));",model.name).unwrap();}
    }
    out.push_str("});\n");
}
fn encode(plan: &Plan, model: &DartModel, out: &mut String) {
    let i = model.index;
    let ty = plan.models.ty(i);
    if ty == "Never" {
        writeln!(
            out,
            "JsonValue _encode{i}(Never value, _Conversion c) => value;\n"
        )
        .unwrap();
        return;
    }
    writeln!(
        out,
        "JsonValue _encode{i}({ty} value, _Conversion c) => c.nest(() {{"
    )
    .unwrap();
    if nullable(plan, model) {
        out.push_str("  if (value == null) { return const JsonNull(); }\n");
    }
    match &model.shape {
        Shape::Any | Shape::Contextual => out.push_str("  return c.json(value);\n"),
        Shape::Never => unreachable!(),
        Shape::Null => out.push_str("  return const JsonNull();\n"),
        Shape::Boolean => out.push_str("  return JsonBoolean(value);\n"),
        Shape::String => out.push_str("  return JsonString(c.string(value));\n"),
        Shape::Number | Shape::Integer => out.push_str("  return c.json(value);\n"),
        Shape::Alias(target) => {
            writeln!(out, "  return _encode{target}(value,c);").unwrap();
        }
        Shape::Array(target) => {
            writeln!(out,"  return c.object(value, () {{ final items=<JsonValue>[];\n    for (var n=0;n<value.length;n++) {{ items.add(c.child('$n', () => {})); }}\n    return JsonArray(items);\n  }});",encode_expr(*target,"value[n]")).unwrap();
        }
        Shape::Object { fields, extras } => {
            out.push_str("  return c.object(value, () {\n    final object=<String,JsonValue>{};\n");
            for (n, f) in fields.iter().enumerate() {
                let key = quote(&f.wire_name);
                if let Some(fixed) = &f.fixed {
                    writeln!(
                        out,
                        "    object[{key}] = c.child({key}, () => c.json({}));",
                        literal(fixed)
                    )
                    .unwrap();
                } else if f.required {
                    writeln!(
                        out,
                        "    object[{key}] = c.child({key}, () => {});",
                        encode_expr(f.target, &format!("value.{}", f.name))
                    )
                    .unwrap();
                } else {
                    writeln!(out,"    final member{n}=value.{}; if(member{n} is Present<{}>) {{ object[{key}] = c.child({key}, () => {}); }}",f.name,plan.models.optional_ty(f.target),encode_expr(f.target,&format!("member{n}.value"))).unwrap();
                }
            }
            if !matches!(extras, Extras::Closed) {
                writeln!(out,"    for(final entry in value.extraFields.entries) {{ c.spend();\n      if(const <String>[{}].contains(entry.key)) {{ c.fail('extra field collides with declared member'); }}\n      object[c.string(entry.key)] = c.child(entry.key, () => {});\n    }}",fields.iter().map(|f|quote(&f.wire_name)).collect::<Vec<_>>().join(", "),encode_expr(extra_target(*extras),"entry.value")).unwrap();
            }
            out.push_str("    return JsonObject(object);\n  });\n");
        }
        Shape::Enum(_) => out.push_str("  return JsonString(value.wireValue);\n"),
        Shape::Union {
            targets,
            direct,
            variants,
        } => {
            if !direct {
                out.push_str("  return c.object(value, () {\n");
            }
            out.push_str("    switch(value) {\n");
            for (n, target) in targets.iter().enumerate() {
                let name = if *direct {
                    &plan.models.symbols()[plan.models.concrete(*target)].name
                } else {
                    &variants[n]
                };
                writeln!(
                    out,
                    "      case {name}(): return _encode{target}({},c);",
                    if *direct { "value" } else { "value.value" }
                )
                .unwrap();
            }
            out.push_str("    }\n");
            if !direct {
                out.push_str("  });\n");
            }
        }
        Shape::Checked => out.push_str("  return c.json(value.value);\n"),
    }
    out.push_str("});\n");
}
pub(super) fn native_value(plan: &Plan, index: usize, value: &Value) -> String {
    let model = &plan.models.symbols()[index];
    if value.is_null() && nullable(plan, model) {
        return "null".into();
    }
    match &model.shape {
        Shape::Alias(target) => native_value(plan, *target, value),
        Shape::Null | Shape::Boolean => value.to_string(),
        Shape::String => quote(value.as_str().expect("validated string")),
        Shape::Number => format!("JsonNumber.parse({})", quote(&value.to_string())),
        Shape::Integer => format!("JsonInteger.parse({})", quote(&value.to_string())),
        Shape::Any | Shape::Contextual => literal(value),
        Shape::Array(target) => format!(
            "<{}>[{}]",
            plan.models.optional_ty(*target),
            value
                .as_array()
                .unwrap()
                .iter()
                .map(|v| target.map_or_else(|| literal(v), |i| native_value(plan, i, v)))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        Shape::Enum(values) => format!(
            "{}.{}",
            model.name,
            values
                .iter()
                .find(|(wire, _)| Some(wire.as_str()) == value.as_str())
                .unwrap()
                .1
        ),
        Shape::Object { fields, extras } => {
            let object = value.as_object().unwrap();
            let mut args = Vec::new();
            for f in fields.iter().filter(|f| f.fixed.is_none()) {
                if let Some(value) = object.get(&f.wire_name) {
                    let v = f
                        .target
                        .map_or_else(|| literal(value), |i| native_value(plan, i, value));
                    args.push(format!(
                        "{}: {}",
                        f.name,
                        if f.required {
                            v
                        } else {
                            format!("Present({v})")
                        }
                    ));
                }
            }
            let extra = object
                .iter()
                .filter(|(key, _)| !fields.iter().any(|f| &f.wire_name == *key))
                .map(|(key, v)| {
                    format!(
                        "{}: {}",
                        quote(key),
                        extra_target(*extras)
                            .map_or_else(|| literal(v), |i| native_value(plan, i, v))
                    )
                })
                .collect::<Vec<_>>();
            if !extra.is_empty() {
                args.push(format!("extraFields: {{{}}}", extra.join(", ")));
            }
            format!("{}({})", model.name, args.join(", "))
        }
        Shape::Union {
            targets,
            direct,
            variants,
        } => {
            for (n, target) in targets.iter().enumerate() {
                if matches!(
                    plan.compiled
                        .validate(&plan.models.symbols()[*target].source, value),
                    suspect_schema::OwnedOutcome::Valid
                ) {
                    let v = native_value(plan, *target, value);
                    return if *direct {
                        v
                    } else {
                        format!("{}({v})", variants[n])
                    };
                }
            }
            format!("{}.fromJson({})", model.codec_name, literal(value))
        }
        Shape::Checked => format!("{}.fromJson({})", model.name, literal(value)),
        Shape::Never => format!("{}.fromJson({})", model.codec_name, literal(value)),
    }
}
