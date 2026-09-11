using Suspect.Csharp.Adversarial;
using System.IO.Compression;
using System.Net;
using System.Net.Http.Headers;
using System.Runtime.InteropServices;
using System.Text;
using System.Text.Json;

static class Probe
{
    private static int _checks;
    private static void Check(bool value, string message) { if (!value) throw new Exception(message); _checks++; }
    private static CodecException Reject(Action action, CodecErrorKind kind)
    {
        try { action(); throw new Exception("Expected " + kind); }
        catch (CodecException error) { Check(error.Kind == kind, "Expected " + kind + " got " + error.Kind); return error; }
    }
    public static async Task Main(string[] args)
    {
        // The selected arm is invalid while the other arm makes the parent oneOf valid.
        var badArm = Reject(() => Codecs.EncodeChoice(new Choice.Variant1("x")), CodecErrorKind.InvalidValue);
        Check(badArm.SchemaSource.EndsWith("/components/schemas/Choice/oneOf/0/minLength"), "Selected-arm source location");
        Check(Codecs.DecodeChoice(Codecs.EncodeChoice(new Choice.Variant2("x"))) is Choice.Variant2 { Value: "x" }, "Disjoint arm round-trip");
        // Here the selected arm is valid, but both arms match and the parent must reject.
        Reject(() => Codecs.EncodeOverlap(new Overlap.Variant1("xx")), CodecErrorKind.InvalidValue);
        Reject(() => Codecs.DecodeOverlap("\"xx\""), CodecErrorKind.InvalidValue);
        Check(Codecs.DecodeInclusive(Codecs.EncodeInclusive(new Inclusive.Variant2("xx"))) is Inclusive.Variant1, "anyOf accepts overlap; decode picks first matching source arm");
        var mixed = new Mixed.Record(new Record { Name = "ok" });
        Check(Codecs.DecodeMixed(Codecs.EncodeMixed(mixed)) is Mixed.Record { Value.Name: "ok" }, "Literal-before-object union remains round-trippable");
        mixed.Value.Name = "";
        Reject(() => Codecs.EncodeMixed(mixed), CodecErrorKind.InvalidValue);
        Check(Codecs.DecodeMixed("\"auto\"") is Mixed.Variant1, "Literal arm");
        Check(Codecs.DecodeNullableUnion("null") is null, "Nullable union null");
        Check(Encoding.UTF8.GetString(Codecs.EncodeNullableUnion(null)) == "null", "Encode nullable union null");
        Check(Codecs.DecodeNullableUnion("{\"name\":\"ok\"}") is NullableUnion.Record, "Nullable union value");
        Check(Codecs.DecodeNullableEnum("null") is null && Codecs.DecodeNullableEnum("\"a\"") == NullableEnum.A, "Nullable literal enum");
        Reject(() => Codecs.DecodeNullableEnum("true"), CodecErrorKind.InvalidValue);
        Check(Encoding.UTF8.GetString(Codecs.EncodeNumberSet(new JsonNumber("1.00e0"))) == "1.00e0", "Numeric literal membership is mathematical; token retained");
        Reject(() => Codecs.EncodeNumberSet(new JsonNumber("3")), CodecErrorKind.InvalidValue);
        Reject(() => Codecs.DecodeNumberSet("true"), CodecErrorKind.InvalidValue);
        var thing = new Thing { Name = "x", RequiredNullable = null, Choice = new Choice.Variant2("x") };
        var text = Encoding.UTF8.GetString(Codecs.EncodeThing(thing));
        Check(text == "{\"choice\":\"x\",\"name\":\"x\",\"required_nullable\":null}", "Four-state presence: required nullable supplied, optional nullable absent");
        thing.Nullable = Optional<string?>.Present(null); thing.Optional = "value";
        text = Encoding.UTF8.GetString(Codecs.EncodeThing(thing));
        Check(text.Contains("\"nullable\":null") && text.Contains("\"optional\":\"value\""), "Optional presence and null are separate");
        Reject(() => Codecs.DecodeThing("{\"name\":\"x\",\"choice\":\"x\"}"), CodecErrorKind.InvalidValue);
        Reject(() => Codecs.DecodeThing("{\"name\":\"x\",\"choice\":\"x\",\"required_nullable\":null,\"optional\":null}"), CodecErrorKind.InvalidValue);
        Reject(() => Codecs.DecodeThing("{\"name\":\"x\",\"choice\":\"x\",\"required_nullable\":null,\"never\":null}"), CodecErrorKind.InvalidValue);
        var map = Codecs.DecodeIntegerMap("{\"a/b~\":1.0}");
        Check(map.Extra["a/b~"].Token == "1.0", "Typed integer extras");
        map.Extra["bad"] = default;
        Reject(() => Codecs.EncodeIntegerMap(map), CodecErrorKind.Conversion);
        Reject(() => Codecs.DecodeIntegerMap("{\"a/b~\":true}"), CodecErrorKind.InvalidValue);
        var names = Codecs.DecodeNames("{\"a-b\":\"a\",\"a_b\":\"b\",\"getType\":\"g\",\"extra\":\"e\",\"😀\":\"u\",\"a/b~\":\"p\"}");
        using (var document = JsonDocument.Parse(Codecs.EncodeNames(names)))
            Check(document.RootElement.GetProperty("a-b").GetString() == "a" && document.RootElement.GetProperty("a_b").GetString() == "b" && document.RootElement.GetProperty("😀").GetString() == "u", "Colliding, inherited and Unicode native names retain wire identities");
        var sent = new List<string>(); var status = 200;
        using var transport = new HttpClient(new Handler(async (request, token) =>
        {
            sent.Add(request.Content is null ? "absent" : Encoding.UTF8.GetString(await request.Content.ReadAsByteArrayAsync(token)));
            var response = new HttpResponseMessage((HttpStatusCode)status) { Content = new ByteArrayContent(Encoding.UTF8.GetBytes(status == 201 ? "null" : "{\"name\":\"ok\"}")) };
            response.Content.Headers.ContentType = new MediaTypeHeaderValue("application/json"); return response;
        }));
        using var client = new Client(new Credentials { ApiKey = "token" }, httpClient: transport);
        Check(await client.OptionalBodyAsync() is OptionalBodyResult.Status200 { Data.Name: "ok" }, "Multiple exact successes retain typed arms");
        status = 201;
        Check(await client.OptionalBodyAsync(new OptionalBodyInput { Body = Optional<string?>.Present(null) }) is OptionalBodyResult.Status201, "Null-only success and optional null body");
        status = 200;
        await client.OptionalBodyAsync(new OptionalBodyInput { Body = "value" });
        Check(sent.SequenceEqual(new[] { "absent", "null", "\"value\"" }), "No body, null body and string body remain distinct");
        using var archive = ZipFile.OpenRead(args[0]);
        using var reader = new StreamReader(archive.GetEntry("docs/index.html")!.Open());
        var html = reader.ReadToEnd();
        Check(html.Contains("&lt;script&gt;alert(1)&lt;/script&gt;") && !html.Contains("<script>"), "Source prose remains inert in native documentation");
        Console.WriteLine($"ADVERSARIAL PASS: {_checks} checks; {RuntimeInformation.FrameworkDescription}; installed assembly {typeof(Client).Assembly.Location}");
    }
}

sealed class Handler(Func<HttpRequestMessage, CancellationToken, Task<HttpResponseMessage>> send) : HttpMessageHandler
{
    protected override Task<HttpResponseMessage> SendAsync(HttpRequestMessage request, CancellationToken token) => send(request, token);
}
