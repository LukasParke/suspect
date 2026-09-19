// Independently authored source values, installed SDK calls and CLR type/doc witnesses.
using Scoped.Csharp;
using System.IO.Compression;
using System.Net;
using System.Reflection;
using System.Runtime.CompilerServices;
using System.Text;
using System.Text.Json;
using System.Xml.Linq;
using Invoice = Scoped.Csharp.__INVOICE__;
using InvoiceKind = Scoped.Csharp.__INVOICE_KIND__;
using Settings = Scoped.Csharp.__SETTINGS__;
using SettingsMode = Scoped.Csharp.__SETTINGS_MODE__;
using Labels = Scoped.Csharp.__LABELS__;
using Node = Scoped.Csharp.__NODE__;
using Maybe = Scoped.Csharp.__MAYBE__;

internal static class Probe
{
    private static int checks;
    private static void Check(bool value, string reason) { if (!value) throw new Exception(reason); checks++; }
    private static JsonElement Json(string text) { using var doc = JsonDocument.Parse(text); return doc.RootElement.Clone(); }
    private static CodecException Invalid(Action action, CodecErrorKind kind, string? suffix = null, string? path = null)
    {
        try { action(); throw new Exception("Expected codec failure " + kind); }
        catch (CodecException error)
        {
            Check(error.Kind == kind, "Codec failure category: " + error.Kind);
            if (suffix is not null) Check(error.SchemaSource.EndsWith(suffix, StringComparison.Ordinal), "Original schema keyword: " + error.SchemaSource);
            if (path is not null) Check(error.InstancePath == path, "Instance pointer: " + error.InstancePath);
            return error;
        }
    }
    private static async Task<SdkException> Failure(Func<Task> action, SdkErrorKind kind, string? suffix = null)
    {
        try { await action(); throw new Exception("Expected SDK failure " + kind); }
        catch (SdkException error)
        {
            Check(error.Kind == kind, "SDK failure category: " + error.Kind);
            if (suffix is not null) Check(error.CodecError?.SchemaSource.EndsWith(suffix, StringComparison.Ordinal) == true, "SDK retains source codec finding");
            return error;
        }
    }
    private static Invoice Business() => new()
    {
        Id = "invoice-1", Kind = InvoiceKind.Business, Company = "Acme",
        Amount = new JsonNumber("9007199254740993.000000000000000001"),
        Extra = new Dictionary<string,JsonElement>(StringComparer.Ordinal) { ["x-line-count"] = Json("2e0") }
    };
    internal static async Task Main(string[] args)
    {
        using var handler = new SourceHandler();
        using var http = new HttpClient(handler);
        using var client = new Client(new Credentials(), httpClient: http);
        var invoice = Business();
        await using (var result = await client.CreateInvoiceAsync(new CreateInvoiceInput { Body = invoice }))
        {
            Check(result.Data.Id == "invoice-1" && result.Data.Kind == InvoiceKind.Business, "Source operation result has native named fields");
            Check(result.Data.Amount.Token == "9007199254740993.000000000000000001", "Exact response numeric token");
            Check(!result.Data.Note.HasValue && !result.Data.External.HasValue, "Absent nullable fields remain absent");
            Check(result.Data.Extra["x-line-count"].GetRawText() == "2e0", "Patterned extras survive closed unmatched tail");
            var sent = Json(handler.Requests.Last().Body);
            Check(sent.GetProperty("amount").GetRawText() == invoice.Amount.Token, "Exact native mutable encode");
            Check(sent.GetProperty("x-line-count").GetRawText() == "2e0" && !sent.TryGetProperty("note", out _), "Pattern key and omission on request");
        }
        var before = handler.Requests.Count;
        invoice.Company = default;
        await Failure(() => client.CreateInvoiceAsync(new CreateInvoiceInput { Body = invoice }), SdkErrorKind.RequestValidation, "/Invoice/then/required");
        Check(handler.Requests.Count == before, "Conditional requirement rejects before transport");
        invoice = Business(); invoice.Extra["x-line-count"] = Json("-1");
        Invalid(() => Codecs.Encode__INVOICE_CODEC__(invoice), CodecErrorKind.InvalidValue, "/Invoice/patternProperties/^x-.*-count$/minimum", "/x-line-count");
        invoice.Extra["x-line-count"] = Json("1e-400");
        Invalid(() => Codecs.Encode__INVOICE_CODEC__(invoice), CodecErrorKind.InvalidValue, "/Invoice/patternProperties/^x-/type", "/x-line-count");
        invoice = Business(); invoice.Extra["other"] = Json("1");
        Invalid(() => Codecs.Encode__INVOICE_CODEC__(invoice), CodecErrorKind.InvalidValue, "/Invoice/additionalProperties", "/other");
        invoice = Business(); invoice.Extra["company"] = Json("\"collision\"");
        Invalid(() => Codecs.Encode__INVOICE_CODEC__(invoice), CodecErrorKind.Conversion);
        invoice = Business(); invoice.External = Optional<string?>.Present(null);
        Invalid(() => Codecs.Encode__INVOICE_CODEC__(invoice), CodecErrorKind.InvalidValue, "/Invoice/dependentRequired/external", "");
        invoice.Note = Optional<string?>.Present(null);
        var retained = Codecs.Decode__INVOICE_CODEC__(Codecs.Encode__INVOICE_CODEC__(invoice));
        Check(retained.Note.HasValue && retained.Note.Value is null && retained.External.HasValue && retained.External.Value is null, "Present null participates in dependentRequired without default insertion");
        retained.Kind = InvoiceKind.Personal;
        Invalid(() => Codecs.Encode__INVOICE_CODEC__(retained), CodecErrorKind.InvalidValue, "/Invoice/else/not", "");
        retained.Company = default;
        Check(Codecs.Decode__INVOICE_CODEC__(Codecs.Encode__INVOICE_CODEC__(retained)).Kind == InvoiceKind.Personal, "Only the selected conditional branch runs");
        retained.Audit = false;
        Invalid(() => Codecs.Encode__INVOICE_CODEC__(retained), CodecErrorKind.InvalidValue, "/Invoice/dependentSchemas/audit/properties/audit/const", "/audit");
        retained.Audit = true;
        Codecs.Encode__INVOICE_CODEC__(retained);
        Check(true, "Dependent schema evaluates the whole object");
        invoice = Business(); invoice.Amount = new JsonNumber("1e100000000000000000000");
        Check(Encoding.UTF8.GetString(Codecs.Encode__INVOICE_CODEC__(invoice)).Contains("1e100000000000000000000", StringComparison.Ordinal), "Huge symbolic exponent never expands");
        invoice.Amount = new JsonNumber("-1e-400");
        Invalid(() => Codecs.Encode__INVOICE_CODEC__(invoice), CodecErrorKind.InvalidValue, "/Invoice/properties/amount/minimum", "/amount");
        invoice.Amount = default;
        Invalid(() => Codecs.Encode__INVOICE_CODEC__(invoice), CodecErrorKind.Conversion);

        var settings = new Settings { Mode = SettingsMode.Extended, Base = new JsonInteger("1.00"), Extra = new() { ["extra"] = Json("\"retained\""), ["flag"] = Json("true"), ["peer"] = Json("\"present\"") } };
        await using (var result = await client.ReplaceSettingsAsync(new ReplaceSettingsInput { Body = settings }))
        {
            Check(result.Data.Base.Token == "1.00" && result.Data.Extra["extra"].GetString() == "retained", "Successful branch annotations preserve conditional-only fields");
            Check(result.Data.Extra.ContainsKey("flag") && result.Data.Extra.ContainsKey("peer"), "Successful dependent schema annotations propagate");
        }
        settings.Extra["unknown"] = Json("null");
        Invalid(() => Codecs.Encode__SETTINGS_CODEC__(settings), CodecErrorKind.InvalidValue, "/Settings/unevaluatedProperties", "/unknown");
        settings.Extra.Remove("unknown"); settings.Extra.Remove("peer");
        Invalid(() => Codecs.Encode__SETTINGS_CODEC__(settings), CodecErrorKind.InvalidValue, "/Settings/dependentSchemas/flag/required", "");
        settings.Extra.Remove("flag"); settings.Mode = SettingsMode.Basic;
        Invalid(() => Codecs.Encode__SETTINGS_CODEC__(settings), CodecErrorKind.InvalidValue, "/Settings/unevaluatedProperties", "/extra");
        settings.Extra.Clear(); Codecs.Encode__SETTINGS_CODEC__(settings);
        Check(true, "Mutation is rechecked under the newly selected branch");

        var sequence = new List<JsonElement> { Json("\"lead\""), Json("9007199254740993e0") };
        await using (var result = await client.ReplaceSequenceAsync(new ReplaceSequenceInput { Body = sequence }))
        { Check(result.Data[0].GetString() == "lead" && result.Data[1].GetRawText() == "9007199254740993e0", "Heterogeneous prefix and matched item retain order and exact numbers"); }
        sequence.Add(Json("false"));
        Invalid(() => Codecs.Encode__SEQUENCE_CODEC__(sequence), CodecErrorKind.InvalidValue, "/Sequence/unevaluatedItems", "/2");
        sequence.RemoveAt(2); sequence.Add(Json("2.00")); sequence.Add(Json("3e0"));
        Invalid(() => Codecs.Encode__SEQUENCE_CODEC__(sequence), CodecErrorKind.InvalidValue, "/Sequence/maxContains", "");
        sequence.RemoveRange(1, 3);
        Invalid(() => Codecs.Encode__SEQUENCE_CODEC__(sequence), CodecErrorKind.InvalidValue);

        var labels = new Labels { Title = "labels", Version = new JsonInteger("2.0"), Extra = new() { ["Upper"] = Json("9007199254740993e0"), ["lower"] = Json("\"text\""), ["Null"] = Json("null") } };
        await using (var result = await client.ReplaceLabelsAsync(new ReplaceLabelsInput { Body = labels }))
        {
            Check(result.Data.Version.Value.Token == "2.0" && result.Data.Extra["Upper"].GetRawText() == "9007199254740993e0", "Named and patterned source fields both survive");
            Check(result.Data.Extra["lower"].GetString() == "text" && result.Data.Extra["Null"].ValueKind == JsonValueKind.Null, "Additional and pattern domains stay distinct");
        }
        labels.Extra["lower"] = Json("3");
        Invalid(() => Codecs.Encode__LABELS_CODEC__(labels), CodecErrorKind.InvalidValue, "/Labels/additionalProperties/type", "/lower");
        labels.Extra["lower"] = Json("\"text\""); labels.Extra[""] = Json("\"empty-name\"");
        Invalid(() => Codecs.Encode__LABELS_CODEC__(labels), CodecErrorKind.InvalidValue, "/Labels/propertyNames/minLength", "/");
        var keys = Codecs.Decode__LABELS_CODEC__("{\"title\":\"keys\",\"A\":1,\"a\":\"text\",\"é\":\"one\",\"é\":\"two\"}");
        Check(keys.Extra.Count == 4 && keys.Extra.Comparer.Equals(StringComparer.Ordinal), "Decoded case and normalization-distinct keys retain ordinal identity");
        Check(Codecs.Decode__LABELS_CODEC__(Codecs.Encode__LABELS_CODEC__(keys)).Extra.Count == 4, "Key identity survives mutable encoding");
        Invalid(() => Codecs.Decode__LABELS_CODEC__("{\"title\":\"x\",\"Upper\":1,\"\\u0055pper\":2}"), CodecErrorKind.InvalidJson);

        var carrier = Codecs.Decode__VALUE_CODEC__("{\"a\":1.00,\"b\":null}");
        await using (var result = await client.EvaluateValueAsync(new EvaluateValueInput { Body = carrier }))
        { Check(result.Data.GetProperty("a").GetRawText() == "1.00" && result.Data.GetProperty("b").ValueKind == JsonValueKind.Null, "Checked JSON carrier unions all successful annotations"); }
        Invalid(() => Codecs.Encode__VALUE_CODEC__(Json("{\"a\":1,\"c\":2}")), CodecErrorKind.InvalidValue, "/ScopedValue/unevaluatedProperties", "/c");
        Invalid(() => Codecs.Encode__VALUE_CODEC__(default), CodecErrorKind.Conversion);
        Check(Codecs.Decode__VALUE_CODEC__("null").ValueKind == JsonValueKind.Null, "Untyped applicators do not imply an object domain");
        var buffer = Encoding.UTF8.GetBytes("{\"a\":1e0}"); var owned = Codecs.Decode__VALUE_CODEC__(buffer); Array.Fill(buffer, (byte)' ');
        Check(owned.GetProperty("a").GetRawText() == "1e0", "Decoded carrier owns storage independently of input bytes");
        JsonElement disposed; using (var document = JsonDocument.Parse("{\"a\":1}")) disposed = document.RootElement;
        Invalid(() => Codecs.Encode__VALUE_CODEC__(disposed), CodecErrorKind.Conversion);

        var node = new Node { Name = "root", Next = new Node { Name = "leaf" } };
        await using (var result = await client.ReplaceNodeAsync(new ReplaceNodeInput { Body = node }))
        { Check(result.Data.Next.Value.Name == "leaf" && !result.Data.Next.Value.Next.HasValue, "Productive reference recursion and absent terminal field"); }
        node.Next = node;
        Invalid(() => Codecs.Encode__NODE_CODEC__(node), CodecErrorKind.Conversion);
        Invalid(() => Codecs.Decode__NODE_CODEC__("{\"name\":\"root\",\"next\":{\"name\":\"leaf\",\"unknown\":1}}"), CodecErrorKind.InvalidValue, "/Node/unevaluatedProperties", "/next/unknown");

        await using (var result = await client.OptionalValueAsync())
        { Check(result.Data is null && handler.Requests.Last().Body == "" && !handler.Requests.Last().HasContent, "Optional body absence is distinct from JSON null"); }
        await using (var result = await client.OptionalValueAsync(new OptionalValueInput { Body = Optional<Maybe?>.Present(null) }))
        { Check(result.Data is null && handler.Requests.Last().Body == "null" && handler.Requests.Last().HasContent, "Present-null body uses its nullable native type"); }
        await using (var result = await client.OptionalValueAsync(new OptionalValueInput { Body = new Maybe { Flag = false, Peer = Optional<string?>.Present(null) } }))
        { Check(Json(handler.Requests.Last().Body).GetProperty("peer").ValueKind == JsonValueKind.Null, "Null peer counts as present under dependentRequired"); }
        Invalid(() => Codecs.Encode__MAYBE_CODEC__(new Maybe { Flag = false }), CodecErrorKind.InvalidValue, "/Maybe/dependentRequired/flag", "");

        await using (var result = await client.EvaluateReferenceAsync(new EvaluateReferenceInput { Body = Json("{\"base\":1e0}") }))
        { Check(result.Data.GetProperty("base").GetRawText() == "1e0", "Ref-sibling carrier preserves same-instance annotations"); }
        Invalid(() => Codecs.Encode__REFERENCE_CODEC__(Json("{\"base\":1,\"extra\":2}")), CodecErrorKind.InvalidValue, "/ScopedReference/unevaluatedProperties", "/extra");

        handler.NextStatus = 422;
        try { await client.CreateInvoiceAsync(new CreateInvoiceInput { Body = Business() }); throw new Exception("Expected typed API response"); }
        catch (__INVOICE_ERROR__ error)
        { await using (error) { Check(error.Data.Kind == InvoiceKind.Business && error.Response?.Status == 422, "Typed API error payload runs the v2 source codec"); } }
        handler.NextBody = "{\"id\":\"bad\",\"kind\":\"business\",\"amount\":1}"; handler.FailClose = true;
        var failure = await Failure(() => client.CreateInvoiceAsync(new CreateInvoiceInput { Body = Business() }), SdkErrorKind.ResponseDecode, "/Invoice/then/required");
        Check(failure.OperationSource.EndsWith("/paths/~1invoices/post", StringComparison.Ordinal) && handler.LastStream!.Closed, "Located v2 response failure survives cleanup");
        Check(!failure.ToString().Contains("private-close", StringComparison.Ordinal), "Cleanup failure does not replace the primary schema error");
        using (var cancelled = new CancellationTokenSource())
        {
            cancelled.Cancel(); before = handler.Requests.Count;
            try { await client.CreateInvoiceAsync(new CreateInvoiceInput { Body = Business() }, cancellationToken: cancelled.Token); throw new Exception("Expected cancellation"); }
            catch (OperationCanceledException error) { Check(error.CancellationToken == cancelled.Token && handler.Requests.Count == before, "Caller cancellation stays outside validation failure"); }
        }
        var concurrent = await Task.WhenAll(Enumerable.Range(0, 32).Select(_ => Task.Run(() => Codecs.Decode__INVOICE_CODEC__(SourceHandler.InvoiceJson))));
        concurrent[0].Extra.Clear();
        Check(concurrent.Skip(1).All(v => v.Extra["x-line-count"].GetRawText() == "2e0"), "Concurrent codec sessions share no mutable scopes or values");

        var required = typeof(Invoice).GetProperty(nameof(Invoice.Amount))!;
        Check(required.PropertyType == typeof(JsonNumber) && required.GetCustomAttribute<RequiredMemberAttribute>() is not null, "Installed CLR exact-number required field");
        Check(typeof(Invoice).GetProperty(nameof(Invoice.Company))!.PropertyType == typeof(Optional<string>), "Conditional requirement does not become unconditional CLR requiredness");
        Check(typeof(Invoice).GetProperty(nameof(Invoice.Extra))!.PropertyType == typeof(Dictionary<string,JsonElement>), "Installed CLR pattern extra carrier");
        Check(typeof(ReplaceSequenceInput).GetProperty("Body")!.PropertyType == typeof(List<JsonElement>), "Installed heterogeneous prefix carrier type");
        Check(typeof(EvaluateValueInput).GetProperty("Body")!.PropertyType == typeof(JsonElement), "Installed runtime-checked source carrier type");
        using (var package = ZipFile.OpenRead(args[0]))
        {
            string Read(string path) { using var reader = new StreamReader(package.GetEntry(path)!.Open()); return reader.ReadToEnd(); }
            var xml = XDocument.Parse(Read("lib/net8.0/Scoped.Csharp.xml")).Descendants("member").Select(m => (string)m.Attribute("name")!).ToHashSet();
            using var reference = JsonDocument.Parse(Read("docs/reference.json")); var html = Read("docs/index.html");
            Check(reference.RootElement.GetProperty("validation").GetProperty("profile").GetString() == "oas31-jsonschema202012-static-applicators", "Rendered reference records executable profile");
            foreach (var symbol in reference.RootElement.GetProperty("symbols").EnumerateArray())
            {
                Check(xml.Contains(symbol.GetProperty("xmlId").GetString()!), "Installed XML symbol: " + symbol.GetProperty("xmlId"));
                Check(html.Contains("id=\"" + symbol.GetProperty("id").GetString() + "\"", StringComparison.Ordinal), "Rendered source/native symbol anchor");
            }
            Check(Read("README.md").Contains("## Scoped source validation", StringComparison.Ordinal) && html.Contains("Checked JSON-value carrier", StringComparison.Ordinal), "Native guide explains source codec obligations");
            using var examples = JsonDocument.Parse(Read("examples/examples.json"));
            Check(examples.RootElement.GetProperty("diagnostics").GetArrayLength() == 0, "All required source-bound example recipes are available");
            Check(examples.RootElement.GetProperty("operations").GetArrayLength() == 8, "Every actual v2 SDK operation has source examples");
        }
        using (var stream = typeof(Codecs).Assembly.GetManifestResourceStream("Suspect.ValidationProgram.json")!)
        using (var program = JsonDocument.Parse(stream))
        {
            Check(program.RootElement.GetProperty("version").GetString() == "suspect.validation.experimental.v2", "Installed package actually executes v2");
            var ops = program.RootElement.GetProperty("nodes").EnumerateArray().SelectMany(n => n.GetProperty("checks").EnumerateArray()).Select(c => c.GetProperty("op").GetString()).ToHashSet();
            foreach (var op in new[] { "if", "dependentRequired", "dependentSchemas", "contains", "patternProperties", "additionalPropertiesWithPatterns", "propertyNames", "unevaluatedProperties", "unevaluatedItems" }) Check(ops.Contains(op), "Installed source instruction " + op);
        }
        Check(handler.Requests.Select(r => r.Path).Distinct().Count() == 8, "All eight source operation paths exercised");
        File.WriteAllText("sdk-results.json", JsonSerializer.Serialize(new { checks, sdkRequests = handler.Requests.Count, operationPaths = handler.Requests.Select(r => r.Path).Distinct().ToArray(), framework = System.Runtime.InteropServices.RuntimeInformation.FrameworkDescription }, new JsonSerializerOptions { WriteIndented = true }));
        Console.WriteLine($"SCOPED SDK PASS: {checks} checks; SDK requests={handler.Requests.Count}; eight source operations; {System.Runtime.InteropServices.RuntimeInformation.FrameworkDescription}");
    }
}

internal sealed record Seen(string Path, string Body, bool HasContent);
internal sealed class SourceHandler : HttpMessageHandler
{
    internal const string InvoiceJson = "{\"id\":\"invoice-1\",\"kind\":\"business\",\"amount\":9007199254740993.000000000000000001,\"company\":\"Acme\",\"x-line-count\":2e0}";
    internal readonly List<Seen> Requests = new();
    internal int NextStatus = 200;
    internal string? NextBody;
    internal bool FailClose;
    internal TrackedStream? LastStream;
    protected override async Task<HttpResponseMessage> SendAsync(HttpRequestMessage request, CancellationToken cancellationToken)
    {
        cancellationToken.ThrowIfCancellationRequested();
        var path = request.RequestUri!.AbsolutePath;
        var body = request.Content is null ? "" : Encoding.UTF8.GetString(await request.Content.ReadAsByteArrayAsync(cancellationToken));
        Requests.Add(new Seen(path, body, request.Content is not null));
        var reply = NextBody ?? path switch
        {
            "/invoices" => InvoiceJson,
            "/settings" => "{\"mode\":\"extended\",\"base\":1.00,\"extra\":\"retained\",\"flag\":true,\"peer\":\"present\"}",
            "/sequences" => "[\"lead\",9007199254740993e0]",
            "/labels" => "{\"title\":\"labels\",\"Version\":2.0,\"Upper\":9007199254740993e0,\"lower\":\"text\",\"Null\":null}",
            "/values" => "{\"a\":1.00,\"b\":null}",
            "/nodes" => "{\"name\":\"root\",\"next\":{\"name\":\"leaf\"}}",
            "/optional" => "null",
            "/references" => "{\"base\":1e0}",
            _ => throw new Exception("Unknown source operation path " + path)
        };
        LastStream = new TrackedStream(Encoding.UTF8.GetBytes(reply), FailClose);
        var response = new HttpResponseMessage((HttpStatusCode)NextStatus) { Content = new StreamContent(LastStream) };
        response.Content.Headers.TryAddWithoutValidation("Content-Type", "application/json");
        NextStatus = 200; NextBody = null; FailClose = false;
        return response;
    }
}
internal sealed class TrackedStream(byte[] bytes, bool failClose) : MemoryStream(bytes)
{
    internal bool Closed;
    protected override void Dispose(bool disposing) { Closed = true; base.Dispose(disposing); if (failClose) throw new IOException("private-close"); }
}
