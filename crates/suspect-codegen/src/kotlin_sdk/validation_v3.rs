//! V3 adds resource hooks to the frozen v2 kernel; it never changes v2 bytes.
pub(super) fn runtime() -> String {
    let mut kernel = include_str!("ValidationV2.kt").to_owned();
    fn replace(text: &mut String, from: &str, to: &str) {
        assert_eq!(
            text.matches(from).count(),
            1,
            "frozen scoped kernel boundary: {from}"
        );
        *text = text.replacen(from, to, 1);
    }
    replace(
        &mut kernel,
        "suspect.validation.experimental.v2",
        "suspect.validation.experimental.v3",
    );
    replace(
        &mut kernel,
        "oas31-jsonschema202012-static-applicators",
        "oas31-jsonschema202012-resources-dynamic",
    );
    replace(
        &mut kernel,
        "    val roots: Map<SourceLocation, Int>\n    init {",
        "    val roots: Map<SourceLocation, Int>\n    val resources = IndexedResources(value.obj(\"resourceContext\"), nodes)\n    init {",
    );
    replace(
        &mut kernel,
        "                \"ref\" -> \"\\$ref\"",
        "                \"ref\" -> \"\\$ref\"\n                \"dynamicRef\" -> \"\\$dynamicRef\"",
    );
    replace(
        &mut kernel,
        "                \"ref\" -> target(check.number(\"target\"))",
        "                \"ref\" -> target(check.number(\"target\"))\n                \"dynamicRef\" -> resources.checkReference(check)",
    );
    replace(
        &mut kernel,
        "private val active = java.util.IdentityHashMap<JsonValue, MutableSet<Int>>()",
        "private val active = java.util.IdentityHashMap<JsonValue, MutableSet<ResourceCycle>>()\n    private val resourceScope = ResourceScope(program.resources)",
    );
    replace(
        &mut kernel,
        "        val identities = active.getOrPut(value) { mutableSetOf() }\n        if (!identities.add(target)) fail(source, path, \"recursive schema made no instance progress\")",
        r#"        val entered = resourceScope.enter(target) { step(source, path) }
        val identity = ResourceCycle(target, resourceScope.context)
        val identities = active.getOrPut(value) { mutableSetOf() }
        if (!identities.add(identity)) {
            resourceScope.leave(entered)
            fail(source, path, "recursive schema revisited the same instance and resource context")
        }"#,
    );
    replace(
        &mut kernel,
        "            identities.remove(target)\n            if (identities.isEmpty()) active.remove(value)",
        "            identities.remove(identity)\n            if (identities.isEmpty()) active.remove(value)\n            resourceScope.leave(entered)",
    );
    replace(
        &mut kernel,
        "            \"ref\" -> eval(check.number(\"target\"), value, path, depth + 1).let { produced = it.marks; it.valid }",
        r#"            "ref" -> eval(check.number("target"), value, path, depth + 1).let { produced = it.marks; it.valid }
            "dynamicRef" -> {
                val selected = resourceScope.resolve(check) { step(at, path) }
                eval(selected, value, path, depth + 1).let { produced = it.marks; it.valid }
            }"#,
    );
    kernel.push_str(include_str!("ValidationResources.kt"));
    kernel
}
