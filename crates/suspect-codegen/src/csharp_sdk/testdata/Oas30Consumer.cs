// Independent native consumer of the OpenAPI 3.0 petstore fixture
// (tests/fixtures/oas30/petstore-nullable.openapi.yaml). Expectations come from
// OAS 3.0.3: same-object nullable, Boolean exclusive bounds, and ignored
// Reference Object siblings. Wire bytes are hand-authored.
using Suspect.Csharp.Oas30;
using System.Net;
using System.Runtime.InteropServices;
using System.Text;

static class Probe
{
    private const string PetJson = "{\"id\":1,\"name\":\"Rex\",\"tag\":null,\"owner\":{\"name\":\"Ada\"},\"nickname\":null,\"metadata\":null,\"status\":null,\"weight\":49.5,\"anything\":null}";
    private static int _checks;
    internal static void Check(bool condition, string message)
    { if (!condition) throw new Exception(message); _checks++; }
    internal static CodecException Codec(Action action, CodecErrorKind kind)
    {
        try { action(); throw new Exception("Expected codec failure: " + kind); }
        catch (CodecException error) { Check(error.Kind == kind, "Codec kind: " + error.Kind + " expected " + kind); return error; }
    }
    internal static async Task<SdkException> Failure(Func<Task> action, SdkErrorKind kind)
    {
        try { await action(); throw new Exception("Expected SDK failure: " + kind); }
        catch (SdkException error) { Check(error.Kind == kind, "SDK kind: " + error.Kind + " expected " + kind); return error; }
    }
    private static string Replace(string field, string value)
    {
        var start = PetJson.IndexOf("\"" + field + "\":", StringComparison.Ordinal) + field.Length + 3;
        var end = PetJson.IndexOfAny(new[] { ',', '}' }, PetJson[start] == '{' ? PetJson.IndexOf('}', start) + 1 : start);
        return PetJson[..start] + value + PetJson[end..];
    }
    public static async Task Main()
    {
        Nullable();
        ExclusiveBounds();
        ReferenceSiblings();
        await Petstore();
        Console.WriteLine($"OAS30 PASS: {_checks} checks; {RuntimeInformation.FrameworkDescription}; installed assembly {typeof(Client).Assembly.Location}");
    }
    private static void Nullable()
    {
        var pet = Codecs.DecodePet(PetJson);
        Check(pet.Tag is null, "Scalar nullable decodes null");
        Check(pet.Nickname.HasValue && pet.Nickname.Value is null, "Nullable $ref target decodes present null");
        Check(pet.Metadata.HasValue && pet.Metadata.Value is null, "Object nullable decodes present null");
        Check(pet.Status.HasValue && pet.Status.Value is null, "Enum listing null decodes null");
        Check(pet.Anything.HasValue && pet.Anything.Value.ValueKind == System.Text.Json.JsonValueKind.Null, "Untyped nullable keeps JSON null");
        var encoded = Encoding.UTF8.GetString(Codecs.EncodePet(pet));
        Check(encoded.Contains("\"tag\":null") && encoded.Contains("\"metadata\":null") && encoded.Contains("\"nickname\":null"), "Present nulls re-encode: " + encoded);
        var full = Codecs.DecodePet(Replace("tag", "\"good\"").Replace("\"metadata\":null", "\"metadata\":{\"note\":\"n\"}").Replace("\"status\":null", "\"status\":\"sold\"").Replace("\"nickname\":null", "\"nickname\":\"Rexy\""));
        Check(full.Tag == "good" && full.Metadata.Value!.Note.Value == "n" && full.Status.Value == PetStatus.Sold && full.Nickname.Value == "Rexy", "Non-null nullable values");
        // nullable adds null only to the same-object type; the target's own
        // assertions still apply to non-null values.
        Check(Codec(() => Codecs.DecodePet(Replace("nickname", "\"\"")), CodecErrorKind.InvalidValue).SchemaSource.EndsWith("/components/schemas/Nickname/minLength"), "Nullable target keeps minLength");
        Codec(() => Codecs.DecodePet(Replace("status", "\"lost\"")), CodecErrorKind.InvalidValue);
        Codec(() => Codecs.DecodePet(Replace("name", "null")), CodecErrorKind.InvalidValue);
        var standalone = Codecs.DecodeNickname("null");
        Check(standalone is null, "Nullable component root admits null");
        Check(Encoding.UTF8.GetString(Codecs.EncodeNickname(null)) == "null", "Nullable component root encodes null");
    }
    private static void ExclusiveBounds()
    {
        Check(Codec(() => Codecs.DecodePet(Replace("weight", "50")), CodecErrorKind.InvalidValue).SchemaSource.EndsWith("/components/schemas/Pet/properties/weight/maximum"), "exclusiveMaximum: true excludes 50");
        Check(Codecs.DecodePet(Replace("weight", "49.999")).Weight.Value.Token == "49.999", "Just below the exclusive maximum");
        Check(Codec(() => Codecs.DecodePet(Replace("id", "0")), CodecErrorKind.InvalidValue).SchemaSource.EndsWith("/components/schemas/Pet/properties/id/minimum"), "exclusiveMinimum: true excludes 0");
        var pet = Codecs.DecodePet(PetJson);
        pet.Weight = new JsonNumber("50");
        Codec(() => Codecs.EncodePet(pet), CodecErrorKind.InvalidValue);
    }
    private static void ReferenceSiblings()
    {
        // owner: {$ref: Owner, nullable: true, type: string} applies only Owner.
        Check(Codec(() => Codecs.DecodePet(Replace("owner", "null")), CodecErrorKind.InvalidValue).InstancePath == "/owner", "Ignored nullable sibling");
        Codec(() => Codecs.DecodePet(Replace("owner", "\"Ada\"")), CodecErrorKind.InvalidValue);
        Check(Codecs.DecodePet(PetJson).Owner.Name == "Ada", "Owner object");
    }
    private static async Task Petstore()
    {
        var seen = new List<string>();
        using var handler = new Handler(async (request, token) =>
        {
            var body = request.Content is null ? "" : Encoding.UTF8.GetString(await request.Content.ReadAsByteArrayAsync(token));
            seen.Add(request.Method.Method + " " + request.RequestUri!.OriginalString + " " + body);
            var (status, payload) = request.RequestUri.AbsolutePath switch
            {
                "/v1/pets" when request.Method == HttpMethod.Get => (200, "[" + PetJson + "]"),
                "/v1/pets" => (201, body),
                "/v1/pets/bad" => (200, PetJson.Replace("\"weight\":49.5", "\"weight\":50")),
                _ => (200, PetJson),
            };
            var response = new HttpResponseMessage((HttpStatusCode)status) { Content = new ByteArrayContent(Encoding.UTF8.GetBytes(payload)) };
            response.Content.Headers.TryAddWithoutValidation("Content-Type", "application/json");
            return response;
        });
        using var transport = new HttpClient(handler);
        using var client = new Client(new Credentials(), httpClient: transport);
        await using (var page = await client.ListPetsAsync(new ListPetsInput { Limit = new JsonInteger("100") }))
        {
            Check(page is ListPetsResult.Status200 { Data: [{ Tag: null, Owner.Name: "Ada" }] } && seen[^1].Contains("?limit=100"), "Typed list; inclusive maximum");
        }
        await using (var smallest = await client.ListPetsAsync(new ListPetsInput { Limit = new JsonInteger("1") }))
        {
            Check(smallest.Status == 200 && seen[^1].Contains("?limit=1"), "Just above the exclusive minimum");
        }
        // exclusiveMinimum: true on the limit parameter rejects 0 before transport.
        var before = seen.Count;
        await Failure(() => client.ListPetsAsync(new ListPetsInput { Limit = new JsonInteger("0") }), SdkErrorKind.RequestValidation);
        Check(seen.Count == before, "Request validation precedes transport");
        var created = await client.CreatePetAsync(new CreatePetInput { Body = new Pet { Id = new JsonInteger("2"), Name = "Tom", Tag = null, Owner = new Owner { Name = "Bo" }, Nickname = Optional<string?>.Present(null) } });
        Check(created.Status == 201 && created.Data.Tag is null && created.Data.Nickname.HasValue && created.Data.Nickname.Value is null, "Round-tripped nullable body");
        Check(seen[^1].EndsWith("{\"id\":2,\"name\":\"Tom\",\"nickname\":null,\"owner\":{\"name\":\"Bo\"},\"tag\":null}"), "Request bytes: " + seen[^1]);
        await Failure(() => client.CreatePetAsync(new CreatePetInput { Body = new Pet { Id = new JsonInteger("2"), Name = "Tom", Tag = null, Owner = new Owner { Name = "Bo" }, Weight = new JsonNumber("50") } }), SdkErrorKind.RequestValidation);
        var shown = await client.ShowPetByIdAsync(new ShowPetByIdInput { PetId = "1" });
        Check(shown.Data.Name == "Rex", "Typed show");
        await Failure(() => client.ShowPetByIdAsync(new ShowPetByIdInput { PetId = "bad" }), SdkErrorKind.ResponseDecode);
    }
}

sealed class Handler(Func<HttpRequestMessage, CancellationToken, Task<HttpResponseMessage>> send) : HttpMessageHandler
{
    protected override Task<HttpResponseMessage> SendAsync(HttpRequestMessage request, CancellationToken token) => send(request, token);
}
