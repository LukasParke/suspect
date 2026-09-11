//! Versioned native resource-scope extension; ordinary v1/v2 assets stay frozen.
use std::collections::{BTreeMap, BTreeSet};
use suspect_ir::contract::{Contract, SchemaId};

pub(super) fn required(contract: &Contract, closure: &[SchemaId]) -> bool {
    closure.iter().any(|id| {
        contract.schema(id).is_some_and(|schema| {
            !schema.ignores_ref_siblings()
                && (contract
                    .resource_scope(id)
                    .is_some_and(|scope| scope.base_source().is_some())
                    || ["$id", "$anchor", "$dynamicAnchor", "$dynamicRef"]
                        .iter()
                        .any(|key| schema.raw().get(*key).is_some()))
        })
    })
}

/// Conservative dependency cone for native conversion, never dynamic binding
/// selection. Context-sensitive union trials and null-domain proofs cannot be
/// evaluated as independent roots while a parent resource scope is absent.
pub(super) fn context_sensitive(contract: &Contract, closure: &[SchemaId]) -> BTreeSet<SchemaId> {
    let mut parents = BTreeMap::<SchemaId, Vec<SchemaId>>::new();
    let mut pending = Vec::new();
    for id in closure {
        let schema = contract.schema(id).expect("checked schema closure");
        if !schema.ignores_ref_siblings() {
            for child in schema.children() {
                parents.entry(child.clone()).or_default().push(id.clone());
            }
            if contract
                .dynamic_reference(id)
                .is_some_and(|reference| reference.dynamic_anchor().is_some())
            {
                pending.push(id.clone());
            }
        }
        for reference in schema.references() {
            if let Some(target) = &reference.target {
                parents.entry(target.clone()).or_default().push(id.clone());
            }
        }
    }
    let mut result = BTreeSet::new();
    while let Some(id) = pending.pop() {
        if result.insert(id.clone()) {
            pending.extend(parents.get(&id).into_iter().flatten().cloned());
        }
    }
    result
}

pub(super) fn runtime() -> Vec<(&'static str, String)> {
    // These are fixed source-template seams, never schema or descriptor text.
    // Requiring exactly one match prevents future edits to the frozen evaluator
    // from silently dropping a resource entry, restoration or guard obligation.
    let mut evaluator = include_str!("ScopedValidationRuntime.cs").to_owned();
    for (before, after) in [
        (
            "// The v2 executable profile. Emission selects this asset only for a checked v2\n// closure; the frozen v1 evaluator is a separate asset. No raw schema is executed.",
            "// V3: the frozen scoped evaluator plus checked ResourceScope entry/lookup hooks.\n// No source schema or URI is executed or acquired at runtime.",
        ),
        (
            "internal sealed class ValidationSession",
            "internal sealed partial class ValidationSession",
        ),
        ("HashSet<(int, Instance)>", "HashSet<(int, Instance, int)>"),
        (
            "_nodes = program.GetProperty(\"nodes\").Clone();",
            "_nodes = program.GetProperty(\"nodes\").Clone();\n        _resourceData = program.GetProperty(\"resourceContext\").Clone();\n        _nodeResources = _resourceData.GetProperty(\"nodeScopes\").EnumerateArray().Select(scope => scope[0].GetInt32()).ToArray();",
        ),
        (
            "if (!_active.Add((index, instance))) throw Failure(source, path, \"Nonproductive recursive schema evaluation\");",
            "var previous = EnterResource(_nodeResources[index], source, path);\n        var identity = (index, instance, _resourceContext);\n        if (!_active.Add(identity)) { LeaveResource(previous); throw Failure(source, path, \"Nonproductive recursive schema evaluation\"); }",
        ),
        (
            "finally { _active.Remove((index, instance)); }",
            "finally { _active.Remove(identity); LeaveResource(previous); }",
        ),
        ("case \"ref\":", "case \"ref\": case \"dynamicRef\":"),
        (
            "var reference = Evaluate(check.GetProperty(\"target\").GetInt32(), instance, path, depth + 1);",
            "var reference = Evaluate(op == \"dynamicRef\" ? DynamicTarget(check, at, path) : check.GetProperty(\"target\").GetInt32(), instance, path, depth + 1);",
        ),
    ] {
        replace_one(&mut evaluator, before, after);
    }
    let mut guard = include_str!("ValidationProgramGuard.cs").to_owned();
    for (before, after) in [
        (
            "internal sealed class ValidationProgramGuard",
            "internal sealed partial class ValidationProgramGuard",
        ),
        (
            "Object(program, \"version\", \"profile\", \"roots\", \"nodes\", \"limits\");",
            "Object(program, \"version\", \"profile\", \"roots\", \"nodes\", \"limits\", \"resourceContext\");",
        ),
        (
            "suspect.validation.experimental.v2",
            "suspect.validation.experimental.v3",
        ),
        (
            "oas31-jsonschema202012-static-applicators",
            "oas31-jsonschema202012-resources-dynamic",
        ),
        (
            "Need(Uri.TryCreate(document, UriKind.Absolute, out var uri) && uri.Fragment.Length == 0 && !document.Contains('#'), \"Source document must be absolute and fragment-free\");",
            "DocumentSource(document);",
        ),
        (
            "foreach (var node in _nodes.EnumerateArray()) Node(node);",
            "Resources(program);\n        foreach (var node in _nodes.EnumerateArray()) Node(node);",
        ),
        (
            "\"always\" => null, \"ref\" => \"$ref\",",
            "\"always\" => null, \"ref\" => \"$ref\", \"dynamicRef\" => \"$dynamicRef\",",
        ),
        (
            "case \"ref\": Object(check, \"source\", \"op\", \"target\"); Target(check.GetProperty(\"target\")); break;",
            "case \"ref\": Object(check, \"source\", \"op\", \"target\"); Target(check.GetProperty(\"target\")); break;\n                case \"dynamicRef\": Dynamic(check); break;",
        ),
    ] {
        replace_one(&mut guard, before, after);
    }
    vec![
        ("ResourceValidationRuntime.g.cs", evaluator),
        ("ResourceScope.cs", include_str!("ResourceScope.cs").into()),
        ("ResourceStaticGuard.g.cs", guard),
        (
            "ResourceProgramGuard.cs",
            include_str!("ResourceProgramGuard.cs").into(),
        ),
    ]
}
fn replace_one(template: &mut String, before: &str, after: &str) {
    assert_eq!(
        template.matches(before).count(),
        1,
        "frozen C# resource extension seam: {before}"
    );
    *template = template.replacen(before, after, 1);
}
