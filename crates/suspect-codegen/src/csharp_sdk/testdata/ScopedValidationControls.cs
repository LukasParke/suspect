using System.Text.Json;
using System.Text.Json.Nodes;
using Scoped.Csharp;

internal static class ScopedValidationControls
{
    internal static int Run(JsonElement vectors)
    {
        var checks = 0;
        var records = new List<object>();
        void Check(bool value, string reason) { if (!value) throw new Exception(reason); checks++; }
        JsonObject Copy(string id) => JsonNode.Parse(vectors.EnumerateArray().Single(c => c.GetProperty("id").GetString() == id).GetProperty("program").GetRawText())!.AsObject();
        JsonObject Find(JsonObject program, string op) => program["nodes"]!.AsArray().SelectMany(n => n!["checks"]!.AsArray()).Single(c => c!["op"]!.GetValue<string>() == op)!.AsObject();
        JsonObject First(string id) => Copy(id);
        void Reject(string name, string id, Action<JsonObject> mutate)
        {
            var copy = Copy(id); mutate(copy);
            using var document = JsonDocument.Parse(copy.ToJsonString());
            try { _ = new ValidationSession(document.RootElement); throw new Exception(name + ": malformed program accepted"); }
            catch (CodecException error)
            {
                Check(error.Kind == CodecErrorKind.EvaluationFailure, name + ": guard failure is noninvertible");
                records.Add(new { name, outcome = error.Kind.ToString(), source = error.SchemaSource });
                File.WriteAllText("admission-results.json", JsonSerializer.Serialize(records, new JsonSerializerOptions { WriteIndented = true }));
            }
        }
        const string conditional = "if-then-selected";
        const string patterned = "pattern-overlap-accepts-and-excludes-extra";
        Reject("unknown-version", conditional, p => p["version"] = "future");
        Reject("mismatched-profile", conditional, p => p["profile"] = "oas31-jsonschema202012-static-subset");
        Reject("v2-op-in-v1-envelope", conditional, p => { p["version"] = "suspect.validation.experimental.v1"; p["profile"] = "oas31-jsonschema202012-static-subset"; });
        Reject("unknown-unexecuted-op", conditional, p => Find(p, "if")["op"] = "future");
        Reject("missing-operand", conditional, p => Find(p, "if").Remove("elseTarget"));
        Reject("unknown-operand", conditional, p => Find(p, "if")["surprise"] = true);
        Reject("condition-child-identity", conditional, p => Find(p, "if")["condition"] = p["roots"]![0]!["target"]!.DeepClone());
        Reject("branch-target-outside-graph", conditional, p => Find(p, "if")["thenTarget"] = 1000000);
        Reject("duplicate-root", conditional, p => p["roots"]!.AsArray().Add(p["roots"]![0]!.DeepClone()));
        Reject("duplicate-schema-identity", conditional, p => p["nodes"]![1]!["source"] = p["nodes"]![0]!["source"]!.DeepClone());
        Reject("invalid-source-pointer", conditional, p => p["nodes"]![0]!["source"]!["pointer"] = "/bad~2");
        Reject("relative-source-uri", conditional, p => p["nodes"]![0]!["source"]!["document"] = "relative.json");
        Reject("negative-work-limit", conditional, p => p["limits"]!["maxEvaluationSteps"] = -1);
        Reject("excessive-work-limit", conditional, p => p["limits"]!["maxEvaluationSteps"] = 1000001);
        Reject("fractional-work-limit", conditional, p => p["limits"]!["maxEvaluationSteps"] = 1.5);
        Reject("excessive-depth-limit", conditional, p => p["limits"]!["maxDepth"] = 129);
        Reject("dependent-required-tuple", "dependent-required-null-is-present", p => Find(p, "dependentRequired")["dependencies"]![0]!.AsArray().Add("extra"));
        Reject("duplicate-dependency-trigger", "dependent-required-null-is-present", p => { var a = Find(p, "dependentRequired")["dependencies"]!.AsArray(); a.Add(a[0]!.DeepClone()); });
        Reject("duplicate-required-name", "dependent-required-null-is-present", p => { var a = Find(p, "dependentRequired")["dependencies"]![0]![1]!.AsArray(); a.Add(a[0]!.DeepClone()); });
        Reject("dependency-schema-child-identity", "dependent-schema-whole-object", p => Find(p, "dependentSchemas")["dependencies"]![0]!["target"] = p["roots"]![0]!["target"]!.DeepClone());
        foreach (var (name, token) in new[] { ("negative-count", "-1"), ("fractional-count", "1.5"), ("invalid-count-token", "01"), ("spaced-count-token", "1 ") })
            Reject(name, "contains-marks-all-matches", p => Find(p, "contains")["minimum"] = token);
        Reject("source-number-budget", "contains-marks-all-matches", p => { p["limits"]!["maxNumberBytes"] = 1; Find(p, "contains")["minimum"] = "10"; });
        Reject("missing-pattern-tuple-member", patterned, p => Find(p, "patternProperties")["patterns"]![0]!.AsArray().RemoveAt(2));
        Reject("duplicate-property-pattern", patterned, p => { var a = Find(p, "patternProperties")["patterns"]!.AsArray(); a.Add(a[0]!.DeepClone()); });
        Reject("unknown-pattern-version", patterned, p => Find(p, "patternProperties")["patterns"]![0]![1]!["version"] = "future");
        Reject("invalid-pattern-edge", patterned, p => Find(p, "patternProperties")["patterns"]![0]![1]!["start"] = 99999);
        Reject("invalid-pattern-range", patterned, p => { var states = Find(p, "patternProperties")["patterns"]![0]![1]!["states"]!.AsArray(); var c = states.First(s => s!["op"]!.GetValue<string>() == "char")!; c["ranges"]![0]![0] = 0xd800; c["ranges"]![0]![1] = 0xdfff; });
        Reject("wrong-additional-opcode", patterned, p => Find(p, "additionalPropertiesWithPatterns")["op"] = "additionalProperties");
        Reject("undeclared-adjacent-name", patterned, p => Find(p, "additionalPropertiesWithPatterns")["declared"]!.AsArray().Add("imagined"));
        Reject("missing-adjacent-patterns", patterned, p => { foreach (var n in p["nodes"]!.AsArray()) { var a = n!["checks"]!.AsArray(); var c = a.FirstOrDefault(c => c!["op"]!.GetValue<string>() == "patternProperties"); if (c is not null) { a.Remove(c); break; } } });
        Reject("property-name-child-identity", "property-names-checks-key-not-value", p => Find(p, "propertyNames")["target"] = p["roots"]![0]!["target"]!.DeepClone());
        Reject("unevaluated-before-other-checks", "nested-members-do-not-mark-parent", p => { var root = p["nodes"]![p["roots"]![0]!["target"]!.GetValue<int>()]!["checks"]!.AsArray(); var c = root.Single(c => c!["op"]!.GetValue<string>() == "unevaluatedProperties")!; root.Remove(c); root.Insert(0, c); });
        Reject("prefix-target-child-identity", "prefix-and-contains-annotations-combine", p => Find(p, "prefixItems")["targets"]![0] = p["roots"]![0]!["target"]!.DeepClone());

        // Operand admission must not accidentally evaluate deferred literals.
        var source = First(conditional); var target = source["roots"]![0]!["target"]!.GetValue<int>();
        ValidationSession session;
        using (var document = JsonDocument.Parse(source.ToJsonString())) session = new ValidationSession(document.RootElement);
        session.Check(target, JsonRuntime.Parse(JsonRuntime.Bytes("{\"kind\":\"s\",\"value\":\"owned\"}")), "");
        Check(true, "Validation program owns its storage after source document disposal");
        try { session.Check(target, JsonRuntime.Parse(JsonRuntime.Bytes("{\"kind\":\"s\",\"value\":7}")), ""); throw new Exception("fresh instance accepted invalid data"); }
        catch (CodecException error) { Check(error.Kind == CodecErrorKind.InvalidValue && error.InstancePath == "/value", "Same-path calls keep distinct instance identities"); }

        foreach (var (limit, value) in new[] { ("maxDepth", 0), ("maxEvaluationSteps", 0), ("maxEqualitySteps", 0) })
        {
            var copy = Copy("if-condition-failure"); copy["limits"]![limit] = value;
            using var document = JsonDocument.Parse(copy.ToJsonString());
            try { new ValidationSession(document.RootElement).Check(document.RootElement.GetProperty("roots")[0].GetProperty("target").GetInt32(), JsonRuntime.Parse("1"u8), ""); throw new Exception("zero limit accepted work"); }
            catch (CodecException error) { Check(error.Kind == CodecErrorKind.EvaluationFailure, "Zero " + limit + " is not unlimited"); }
        }

        Exception? threadFailure = null;
        var recursive = Copy("recursive-ref-annotations-with-instance-progress");
        var nested = "{}"; for (var i = 0; i < 90; i++) nested = "{\"next\":" + nested + "}";
        var thread = new Thread(() =>
        {
            try
            {
                using var program = JsonDocument.Parse(recursive.ToJsonString());
                new ValidationSession(program.RootElement).Check(program.RootElement.GetProperty("roots")[0].GetProperty("target").GetInt32(), JsonRuntime.Parse(JsonRuntime.Bytes(nested)), "");
                threadFailure = new Exception("Deep evaluation did not fail");
            }
            catch (CodecException error) { if (error.Kind != CodecErrorKind.EvaluationFailure || !error.SchemaSource.EndsWith("#/components/schemas/Root", StringComparison.Ordinal) || error.InstancePath != string.Concat(Enumerable.Repeat("/next", 64))) threadFailure = error; }
            catch (Exception error) { threadFailure = error; }
        }, 2 * 1024 * 1024);
        thread.Start(); Check(thread.Join(TimeSpan.FromSeconds(10)), "Normal-stack depth witness completed");
        Check(threadFailure is null, "Normal-stack evaluation fails cleanly at source depth: " + threadFailure);
        Console.WriteLine($"SCOPED CONTROL PASS: {checks} checks; {records.Count} malformed-program refusals; bounded 2 MiB native stack");
        return checks;
    }
}
