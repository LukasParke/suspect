// Independent normative/adversarial expectations. Programs come from OwnedCompiler,
// not from C# code generation or from the C# evaluator's answers.
using Suspect.Csharp.Vectors;
using System.Runtime.InteropServices;
using System.Text;
using System.Text.Json;
using System.Text.Json.Nodes;

static class Probe
{
    private static int _checks;
    private static void Check(bool value, string message) { if (!value) throw new Exception(message); _checks++; }
    private static int Root(string name) => ValidationProgram.Data.GetProperty("roots").EnumerateArray().Single(root => root.GetProperty("source").GetProperty("pointer").GetString() == "/components/schemas/" + name.Replace("~", "~0").Replace("/", "~1")).GetProperty("target").GetInt32();
    private static JsonElement Value(string text) => JsonRuntime.Parse(Encoding.UTF8.GetBytes(text));
    private static void Failure(Action action, CodecErrorKind kind, string label)
    {
        try { action(); throw new Exception("Expected " + label); }
        catch (CodecException error) { Check(error.Kind == kind, label + ": " + error.Kind); }
    }
    private static JsonElement Modified(Action<JsonNode> change)
    {
        var node = JsonNode.Parse(ValidationProgram.Data.GetRawText())!; change(node);
        using var document = JsonDocument.Parse(node.ToJsonString()); return document.RootElement.Clone();
    }
    public static void Main()
    {
        using var vectors = JsonDocument.Parse(File.ReadAllBytes("runtime-contract-v1.json"));
        Check(vectors.RootElement.GetProperty("version").GetString() == "suspect.runtime-contract.v1", "Vector version");
        var cases = vectors.RootElement.GetProperty("cases"); Check(cases.GetArrayLength() == 17, "All 17 shared cases");
        foreach (var item in cases.EnumerateArray())
        {
            var name = item.GetProperty("name").GetString()!; var root = Root(name);
            foreach (var input in item.GetProperty("valid").EnumerateArray()) { new ValidationSession().Check(root, Value(input.GetString()!), ""); Check(true, name); }
            foreach (var input in item.GetProperty("invalid").EnumerateArray()) Failure(() => new ValidationSession().Check(root, Value(input.GetString()!), ""), CodecErrorKind.InvalidValue, name);
        }
        Failure(() => new ValidationSession().Check(Root("Loop"), Value("null"), ""), CodecErrorKind.EvaluationFailure, "Valid union sibling cannot suppress nonproductive recursion");
        Failure(() => new ValidationSession().Check(-1, Value("null"), ""), CodecErrorKind.EvaluationFailure, "Unselected root");
        var emptyWork = Modified(node => node["limits"]!["maxEvaluationSteps"] = 0);
        Failure(() => new ValidationSession(emptyWork).Check(Root("boolean-true"), Value("null"), ""), CodecErrorKind.EvaluationFailure, "Zero visits");
        var negation = Modified(node => node["limits"]!["maxEvaluationSteps"] = 2);
        Failure(() => new ValidationSession(negation).Check(Root("negation"), Value("1"), ""), CodecErrorKind.EvaluationFailure, "Not cannot invert exhaustion");
        var equality = Modified(node => node["limits"]!["maxEqualitySteps"] = 0);
        Failure(() => new ValidationSession(equality).Check(Root("structural-numeric-equality"), Value("{\"a\":[1,null,true]}"), ""), CodecErrorKind.EvaluationFailure, "Zero structural equality visits");
        var numeric = Modified(node => node["limits"]!["maxNumberBytes"] = 0);
        Failure(() => new ValidationSession(numeric).Check(Root("prefix-decimal-comparison"), Value("12"), ""), CodecErrorKind.EvaluationFailure, "Zero numeric operands");
        var unknown = Modified(node => node["nodes"]![Root("boolean-true")]!["checks"]![0]!["op"] = "future-opcode");
        Failure(() => new ValidationSession(unknown).Check(Root("boolean-true"), Value("null"), ""), CodecErrorKind.EvaluationFailure, "Unknown opcode");
        var version = Modified(node => node["version"] = "future-version");
        Failure(() => _ = new ValidationSession(version), CodecErrorKind.EvaluationFailure, "Unknown version");
        foreach (var text in new[] { "1e" + new string('0', 41), "10e-" + new string('0', 40) + "1", "-0.00e-999999999999" })
        { new ValidationSession().Check(Root("mathematical-integers"), Value(text), ""); Check(true, "Symbolic mathematical integer"); }
        Failure(() => new ValidationSession().Check(Root("mathematical-integers"), Value("0.1e" + new string('0', 41)), ""), CodecErrorKind.InvalidValue, "Padded exponent is not a huge exponent");
        var shared = new ValidationSession(Modified(node => node["limits"]!["maxEvaluationSteps"] = 3));
        shared.Check(Root("boolean-true"), Value("null"), "");
        Failure(() => shared.Matches(Root("boolean-true"), Value("null"), ""), CodecErrorKind.EvaluationFailure, "Codec branch trials retain the same evaluation budget");
        var keyword = Value("{\"a/b~\":false}");
        try { new ValidationSession().Check(Root("EscapedPath"), keyword, ""); throw new Exception("Expected escaped path mismatch"); }
        catch (CodecException error)
        {
            Check(error.Kind == CodecErrorKind.InvalidValue && error.InstancePath == "/a~1b~0", "RFC6901 instance pointer");
            Check(error.SchemaSource.EndsWith("/components/schemas/EscapedPath/properties/a~1b~0/type"), "Original escaped schema pointer");
        }
        Console.WriteLine($"PORTABLE VALIDATOR PASS: 17 shared cases, {_checks} assertions; {RuntimeInformation.FrameworkDescription}");
    }
}
