// Independent expectations from unmodified source fixtures, compiled through compile_v3.
using System.Text.Json;
using Resources.Csharp;

internal static class Program
{
    private static int checks;
    private static void Check(bool condition, string reason)
    { if (!condition) throw new Exception(reason); checks++; }
    internal static void Main()
    {
        using var document = JsonDocument.Parse(File.ReadAllBytes("vectors.json"));
        var records = new List<object>();
        foreach (var vector in document.RootElement.GetProperty("cases").EnumerateArray())
        {
            var id = vector.GetProperty("id").GetString()!;
            var program = vector.GetProperty("program");
            var value = JsonRuntime.Parse(JsonRuntime.Bytes(vector.GetProperty("instanceJson").GetString()!));
            var expected = vector.GetProperty("expected").GetString();
            CodecException? error = null;
            void Evaluate()
            {
                try { new ValidationSession(program).Check(vector.GetProperty("rootTarget").GetInt32(), value, ""); }
                catch (CodecException failure) { error = failure; }
            }
            if (vector.TryGetProperty("stackBytes",out var stackBytes))
            { var thread = new Thread(Evaluate,stackBytes.GetInt32()); thread.Start(); thread.Join(); }
            else Evaluate();
            var observed = error is null ? "Valid" : error.Kind == CodecErrorKind.InvalidValue ? "Invalid" : error.Kind == CodecErrorKind.EvaluationFailure ? "EvaluationFailure" : error.Kind.ToString();
            records.Add(new { id, expected, observed, source = error?.SchemaSource, path = error?.InstancePath });
            File.WriteAllText("vector-results.json", JsonSerializer.Serialize(records, new JsonSerializerOptions { WriteIndented = true }));
            Check(observed == expected, $"{id}: expected {expected}, got {observed}; {error}");
            if (error is not null)
            {
                var sources = program.GetProperty("nodes").EnumerateArray().SelectMany(n => n.GetProperty("checks").EnumerateArray().Select(c => c.GetProperty("source")).Append(n.GetProperty("source"))).Select(s => s.GetProperty("document").GetString()+"#"+s.GetProperty("pointer").GetString()).ToHashSet();
                Check(sources.Contains(error.SchemaSource), id + ": findings stay linked to physical source identities");
                if (vector.TryGetProperty("source",out var source)) Check(error.SchemaSource == source.GetString(), id + ": independent keyword source");
                if (vector.TryGetProperty("instancePointer",out var pointer)) Check(error.InstancePath == pointer.GetString(), id + ": independent instance pointer");
            }
            Console.WriteLine($"{id}: {observed}; {error?.SchemaSource}; {error?.InstancePath}");
        }
        Check(records.Count == document.RootElement.GetProperty("expectedCount").GetInt32(), "All original source expectations executed");
        if (document.RootElement.TryGetProperty("nativeControls",out var controls) && controls.GetBoolean()) checks += ResourceValidationControls.Run(document.RootElement.GetProperty("cases"));
        File.WriteAllText("summary.json", JsonSerializer.Serialize(new {checks,cases=records.Count,framework=System.Runtime.InteropServices.RuntimeInformation.FrameworkDescription}));
        Console.WriteLine($"RESOURCE CSHARP PASS: {checks} checks; {records.Count} source cases; {System.Runtime.InteropServices.RuntimeInformation.FrameworkDescription}");
    }
}

namespace Resources.Csharp
{
    internal static class ValidationProgram
    { internal static JsonElement Data => throw new InvalidOperationException("Explicit source program required by this witness"); }
}
