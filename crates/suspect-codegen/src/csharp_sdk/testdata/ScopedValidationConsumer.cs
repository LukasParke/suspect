// Maintained independent source expectations, compiled through OwnedCompiler::compile_v2.
using System.Text.Json;
using Scoped.Csharp;

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
            try { new ValidationSession(program).Check(program.GetProperty("roots")[0].GetProperty("target").GetInt32(), value, ""); }
            catch (CodecException failure) { error = failure; }
            var observed = error is null ? "Valid" : error.Kind == CodecErrorKind.InvalidValue ? "Invalid" : error.Kind == CodecErrorKind.EvaluationFailure ? "EvaluationFailure" : error.Kind.ToString();
            records.Add(new { id, expected, observed, source = error?.SchemaSource, path = error?.InstancePath });
            File.WriteAllText("vector-results.json", JsonSerializer.Serialize(records, new JsonSerializerOptions { WriteIndented = true }));
            Check(observed == expected, $"{id}: expected {expected}, got {observed}; {error}");
            if (error is not null)
            {
                Check(error.SchemaSource == vector.GetProperty("document").GetString() + "#" + vector.GetProperty("source").GetString(), id + ": actual source keyword");
                Check(error.InstancePath == vector.GetProperty("instancePointer").GetString(), id + ": actual instance pointer " + error.InstancePath);
            }
            Console.WriteLine($"{id}: {observed}; {error?.SchemaSource}; {error?.InstancePath}");
        }
        Check(records.Count == document.RootElement.GetProperty("expectedCount").GetInt32(), "All independent source vectors executed");
        if (document.RootElement.TryGetProperty("admissionPrograms", out var programs)) checks += ScopedValidationControls.Run(programs);
        Console.WriteLine($"SCOPED CSHARP PASS: {checks} checks; {records.Count} source vectors; {System.Runtime.InteropServices.RuntimeInformation.FrameworkDescription}");
    }
}

// Only the test harness supplies this normally embedded resource; execution uses
// the same validation entry point and source programs as generated codecs.
namespace Scoped.Csharp
{
    internal static class ValidationProgram
    { internal static JsonElement Data => throw new InvalidOperationException("Explicit source program required by this witness"); }
}
