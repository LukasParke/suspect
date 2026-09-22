// Independent native consumer of the unchanged M2 fixture. Wire expectations are hand-authored.
using Suspect.Csharp.M2;
using System.IO.Compression;
using System.Net;
using System.Net.Http.Headers;
using System.Net.Sockets;
using System.Runtime.InteropServices;
using System.Text;
using System.Text.Json;
using System.Xml.Linq;

static class Probe
{
    internal const string WidgetJson = "{\"id\":\"w1\",\"amount\":9007199254740993.000000000000000001,\"meta\":null,\"child\":{\"label\":\"root\"},\"payload\":{\"kind\":\"standard\",\"text\":\"plain\"}}";
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
    internal static HttpResponseMessage Response(int status = 200, string body = WidgetJson, string media = "application/json", Stream? stream = null)
    {
        var result = new HttpResponseMessage((HttpStatusCode)status) { Content = stream is null ? new ByteArrayContent(Encoding.UTF8.GetBytes(body)) : new StreamContent(stream) };
        result.Content.Headers.TryAddWithoutValidation("Content-Type", media);
        return result;
    }
    private static JsonElement Json(string text) { using var document = JsonDocument.Parse(text); return document.RootElement.Clone(); }
    public static async Task Main(string[] args)
    {
        CodecsAndNumbers();
        await TypedCalls();
        await FailureBoundaries();
        await CancellationAndCleanup();
        await DefaultTransport();
        Documentation(args[0]);
        Console.WriteLine($"M2 PASS: {_checks} checks; {RuntimeInformation.FrameworkDescription}; installed assembly {typeof(Client).Assembly.Location}");
    }
    private static void CodecsAndNumbers()
    {
        var widget = Codecs.DecodeWidget(WidgetJson);
        Check(widget.Amount.Token == "9007199254740993.000000000000000001", "Exact decimal");
        Check(widget.Meta.HasValue && widget.Meta.Value is null, "Present null");
        Check(widget.Child.HasValue && widget.Child.Value.Label == "root" && !widget.Child.Value.Child.HasValue, "Recursive absence");
        Check(widget.Payload is WidgetPayload.StandardPayload { Value.Text: "plain" }, "Native sealed arm");
        var absent = Codecs.DecodeWidget("{\"id\":\"w1\",\"amount\":1,\"payload\":{\"kind\":\"standard\",\"text\":\"p\"}}");
        Check(!absent.Meta.HasValue, "Absent nullable");
        Check(!Encoding.UTF8.GetString(Codecs.EncodeWidget(absent)).Contains("\"meta\""), "Omit absent");
        absent.Meta = Optional<string?>.Present(null);
        Check(Encoding.UTF8.GetString(Codecs.EncodeWidget(absent)).Contains("\"meta\":null"), "Emit explicit null");
        var input = new WidgetInput { Name = "alpha", Amount = new JsonNumber("1.2300e+004") };
        Check(Encoding.UTF8.GetString(Codecs.EncodeWidgetInput(input)) == "{\"amount\":1.2300e+004,\"name\":\"alpha\"}", "Request numeric spelling");
        input.Extra["arbitrary"] = Json("{\"array\":[true,1.00000000000000000001,null]}");
        var roundTrip = Codecs.DecodeWidgetInput(Codecs.EncodeWidgetInput(input));
        Check(roundTrip.Extra["arbitrary"].GetProperty("array")[1].GetRawText() == "1.00000000000000000001", "Exact generic extras");
        input.Name = "";
        var invalid = Codec(() => Codecs.EncodeWidgetInput(input), CodecErrorKind.InvalidValue);
        Check(invalid.SchemaSource.EndsWith("/components/schemas/WidgetInput/properties/name/minLength"), "Original source identity");
        Check(invalid.InstancePath == "/name", "Instance path");
        input.Name = "x"; input.Extra["name"] = Json("\"shadow\"");
        Codec(() => Codecs.EncodeWidgetInput(input), CodecErrorKind.Conversion);
        input.Extra.Clear(); input.Name = null!;
        Codec(() => Codecs.EncodeWidgetInput(input), CodecErrorKind.Conversion);
        input.Name = "\ud800";
        Codec(() => Codecs.EncodeWidgetInput(input), CodecErrorKind.InvalidJson);
        input.Name = "x"; input.Extra["undefined"] = default;
        Codec(() => Codecs.EncodeWidgetInput(input), CodecErrorKind.Conversion);
        input.Extra.Clear(); input.Extra["large"] = Json("\"" + new string('x', 1_000_001) + "\"");
        Codec(() => Codecs.EncodeWidgetInput(input), CodecErrorKind.ResourceLimit);
        var node = new WidgetNode { Label = "root" }; node.Child = node;
        Codec(() => Codecs.EncodeWidgetNode(node), CodecErrorKind.Conversion);
        Codec(() => Codecs.DecodeWidgetInput("{}"), CodecErrorKind.InvalidValue);
        Codec(() => Codecs.DecodeWidgetInput("{\"name\":\"x\",\"amount\":true}"), CodecErrorKind.InvalidValue);
        Codec(() => Codecs.DecodeWidgetInput("{\"name\":\"x\",\"\\u006eame\":\"y\"}"), CodecErrorKind.InvalidJson);
        Codec(() => Codecs.DecodeWidgetInput("{\"name\":\"\\ud800\"}"), CodecErrorKind.InvalidJson);
        Codec(() => Codecs.DecodeWidgetInput("{\"\\ud800\":0,\"name\":\"x\"}"), CodecErrorKind.InvalidJson);
        Codec(() => Codecs.DecodeWidgetInput(new byte[] { 0xff }), CodecErrorKind.InvalidJson);
        Codec(() => Codecs.DecodeWidgetInput("{\"name\":\"x\"} {}"), CodecErrorKind.InvalidJson);
        Codec(() => Codecs.DecodeWidgetInput("{\"name\":\"x\",\"deep\":" + new string('[', 129) + "0" + new string(']', 129) + "}"), CodecErrorKind.ResourceLimit);
        var integral = new JsonInteger("10e-" + new string('0', 40) + "1");
        Check(integral.ToBigInteger() == 1, "Padded negative exponent");
        Check(new JsonInteger("1e" + new string('0', 41)).ToBigInteger() == 1, "Padded zero exponent");
        Check(!new JsonNumber("0.1e" + new string('0', 41)).IsInteger, "No exponent saturation");
        Codec(() => new JsonInteger("0.1e" + new string('0', 41)), CodecErrorKind.Conversion);
        Check(new JsonNumber("-0.00e+999999").Coefficient == 0 && new JsonNumber("-0.00e+999999").Token == "-0.00e+999999", "Negative zero retained");
        Check(new JsonNumber("1e999999999999").CompareTo(new JsonNumber("9e999999999998")) > 0, "Symbolic huge exponent comparison");
        Check(new JsonNumber("1.0").CompareTo(new JsonNumber("1e0")) == 0 && !new JsonNumber("1.0").Equals(new JsonNumber("1e0")), "Mathematical equality versus token identity");
        Codec(() => new JsonInteger("1e999999999999").ToBigInteger(), CodecErrorKind.ResourceLimit);
        foreach (var token in new[] { "", "01", "+1", "1.", "1e", "NaN", "true", " 1", "1 " }) Codec(() => new JsonNumber(token), CodecErrorKind.InvalidJson);
        Codec(() => new JsonNumber("1e" + new string('0', 4096)), CodecErrorKind.ResourceLimit);
        Check(JsonNumber.FromInteger(new System.Numerics.BigInteger(8)).Token == "8", "Fitting native integer is admitted");
        Codec(() => JsonNumber.FromInteger(System.Numerics.BigInteger.One << 40_000), CodecErrorKind.ResourceLimit);
        input = new WidgetInput { Name = "x", Amount = default(JsonNumber) };
        Codec(() => Codecs.EncodeWidgetInput(input), CodecErrorKind.Conversion);
        var standard = new StandardPayload { Text = "plain" };
        Check(Encoding.UTF8.GetString(Codecs.EncodeStandardPayload(standard)).Contains("\"kind\":\"standard\""), "Singleton literal construction");
        standard.Kind = (StandardPayloadKind)99;
        Codec(() => Codecs.EncodeStandardPayload(standard), CodecErrorKind.Conversion);
    }
    private static async Task TypedCalls()
    {
        var seen = new List<(string Method, string Url, string Auth, string Body)>();
        using var handler = new Handler(async (request, token) =>
        {
            seen.Add((request.Method.Method, request.RequestUri!.OriginalString, request.Headers.Authorization!.ToString(), request.Content is null ? "" : Encoding.UTF8.GetString(await request.Content.ReadAsByteArrayAsync(token))));
            Check(request.Headers.Accept.ToString() == "application/json", "Declared Accept");
            if (request.Content is not null) Check(request.Content.Headers.ContentType!.ToString() == "application/json", "Declared body media");
            var response = Response(body: request.Method == HttpMethod.Get && request.RequestUri.OriginalString.Contains('?') ? "{\"items\":[" + WidgetJson + "]}" : WidgetJson);
            response.Headers.TryAddWithoutValidation("X-Repeated", new[] { "a", "b" });
            return response;
        });
        using var transport = new HttpClient(handler);
        using var client = new Client(new Credentials { ApiKey = "m2-key" }, httpClient: transport);
        CreateWidgetResult created = await client.CreateWidgetAsync(new CreateWidgetInput { Body = new WidgetInput { Name = "alpha" } }, new RequestOptions { Headers = new Dictionary<string, string> { ["X-Trace"] = "explicit" } });
        Check(created.Data.Payload is WidgetPayload.StandardPayload && created.Status == 200, "Typed creation result");
        Check(created.Metadata.Headers["x-repeated"].SequenceEqual(new[] { "a", "b" }), "Repeated immutable headers");
        await client.UpdateWidgetAsync(new UpdateWidgetInput { WidgetId = "w1", Body = new WidgetPatch { Amount = new JsonNumber("0.0000000000000000001") } });
        var page = await client.ListWidgetsAsync(new ListWidgetsInput { Tag = "a/b 雪", Tags = new List<string> { "x,y", "z" }, Labels = new List<string> { "red,blue", "green" }, Limit = new JsonInteger("2.0") });
        Check(page.Data.Items.Count == 1, "Typed page");
        await client.GetWidgetAsync(new GetWidgetInput { WidgetId = "a/b 雪" });
        var expected = new[] {
            ("POST", "https://m2.example.test/api/v1/widgets", "Bearer m2-key", "{\"name\":\"alpha\"}"),
            ("PATCH", "https://m2.example.test/api/v1/widgets/w1", "Bearer m2-key", "{\"amount\":0.0000000000000000001}"),
            ("GET", "https://m2.example.test/api/v1/widgets?tag=a%2Fb%20%E9%9B%AA&tags=x%2Cy&tags=z&labels=red%2Cblue,green&limit=2.0", "Bearer m2-key", ""),
            ("GET", "https://m2.example.test/api/v1/widgets/a%2Fb%20%E9%9B%AA", "Bearer m2-key", "")
        };
        Check(seen.SequenceEqual(expected), "Independent M2 wire requests: " + string.Join(";", seen));
        client.Dispose();
        await Failure(() => client.GetWidgetAsync(new GetWidgetInput { WidgetId = "w1" }), SdkErrorKind.Disposed);
        Check(!handler.Closed, "Injected HttpClient remains caller-owned");
    }
    private static async Task FailureBoundaries()
    {
        Func<HttpRequestMessage, CancellationToken, Task<HttpResponseMessage>> responder = (_, _) => Task.FromResult(Response());
        var sends = 0;
        using var handler = new Handler((r, c) => { sends++; return responder(r, c); });
        using var transport = new HttpClient(handler);
        using var client = new Client(new Credentials { ApiKey = "m2-key" }, new ClientOptions { MaxCaptureBytes = 8 }, transport);
        Task<GetWidgetResult> Call() => client.GetWidgetAsync(new GetWidgetInput { WidgetId = "w1" });
        responder = (_, _) => Task.FromResult(Response(404, "{\"message\":\"private-secret\"}"));
        var api = await Failure(() => Call(), SdkErrorKind.Api);
        Check(api is GetWidgetApiException.Status404 { Data.Message: "private-secret" }, "Declared error arm");
        Check(api.BodyCapture.Length == 8 && api.CaptureTruncated && api.Response?.Status == 404, "Bounded API capture");
        Check(!api.ToString().Contains("private-secret"), "No payload in exception formatting");
        var copy = api.BodyCapture; copy[0] = 0; Check(api.BodyCapture[0] != 0, "Capture is isolated");
        responder = (_, _) => Task.FromResult(Response(404, "{\"message\":null}"));
        var invalid = await Failure(() => Call(), SdkErrorKind.ResponseDecode);
        Check(invalid.CodecError?.Kind == CodecErrorKind.InvalidValue && invalid.Response?.Status == 404, "Invalid declared response is not API failure");
        responder = (_, _) => Task.FromResult(Response(body: "not json"));
        Check((await Failure(() => Call(), SdkErrorKind.ResponseDecode)).CodecError?.Kind == CodecErrorKind.InvalidJson, "JSON boundary");
        // RFC 8259 JSON is UTF-8; an unrecognized charset parameter does not select a different decoder.
        responder = (_, _) => Task.FromResult(Response(media: "application/json; charset=iso-8859-1"));
        Check((await Call()).Data.Id == "w1", "JSON charset parameters do not change UTF-8 decoding");
        foreach (var media in new[] { "text/plain", "application/problem+json", "application/json; charset=utf-8; charset=utf-8", "not a media type" })
        {
            responder = (_, _) => Task.FromResult(Response(media: media));
            await Failure(() => Call(), SdkErrorKind.UnexpectedResponse);
        }
        responder = (_, _) => { var response = Response(); response.Content.Headers.Remove("Content-Type"); return Task.FromResult(response); };
        await Failure(() => Call(), SdkErrorKind.UnexpectedResponse);
        responder = (_, _) => { var response = Response(); response.Content.Headers.TryAddWithoutValidation("Content-Encoding", "gzip"); return Task.FromResult(response); };
        await Failure(() => Call(), SdkErrorKind.UnexpectedResponse);
        responder = (_, _) => Task.FromResult(Response(302));
        var count = sends; Check(await Failure(() => Call(), SdkErrorKind.UnexpectedResponse) is UnexpectedResponseException, "Typed unexpected response"); Check(sends == count + 1, "No retry for redirect");
        responder = (_, _) => throw new InvalidOperationException("private-transport-token");
        var failure = await Failure(() => Call(), SdkErrorKind.Transport);
        Check(!failure.ToString().Contains("private-transport-token") && failure.InnerException is null, "Transport failure redacted");
        count = sends;
        failure = await Failure(() => client.CreateWidgetAsync(new CreateWidgetInput { Body = new WidgetInput { Name = "" } }), SdkErrorKind.RequestValidation);
        Check(sends == count && failure.OperationSource.EndsWith("/paths/~1widgets/post") && failure.CodecError?.InstancePath == "/name", "No send on validation failure");
        await Failure(() => client.GetWidgetAsync(new GetWidgetInput { WidgetId = null! }), SdkErrorKind.RequestValidation);
        await Failure(() => client.GetWidgetAsync(null!), SdkErrorKind.RequestRepresentation);
        foreach (var token in new[] { "", "====", "bad=x", "bad\r\nheader", "bad token" })
        {
            using var unauthenticated = new Client(new Credentials { ApiKey = token }, httpClient: transport);
            await Failure(() => unauthenticated.GetWidgetAsync(new GetWidgetInput { WidgetId = "w1" }), SdkErrorKind.Authentication);
        }
        foreach (var name in new[] { "Authorization", "aCcEpT", "Content-Length", "Host", "Cookie", "Proxy-Authorization", "Transfer-Encoding", "X Bad" })
            await Failure(() => client.GetWidgetAsync(new GetWidgetInput { WidgetId = "w1" }, new RequestOptions { Headers = new Dictionary<string, string> { [name] = "value" } }), SdkErrorKind.RequestRepresentation);
        await Failure(() => client.GetWidgetAsync(new GetWidgetInput { WidgetId = "w1" }, new RequestOptions { Headers = new Dictionary<string, string> { ["X-Test"] = "a\r\nb" } }), SdkErrorKind.RequestRepresentation);
        await Failure(() => client.GetWidgetAsync(new GetWidgetInput { WidgetId = "w1" }, new RequestOptions { Headers = new Dictionary<string, string> { ["X-Test"] = "a", ["x-test"] = "b" } }), SdkErrorKind.RequestRepresentation);
        Check(sends == count, "All request failures remain before send");
        transport.DefaultRequestHeaders.Add("Cookie", "implicit=value");
        await Failure(() => Call(), SdkErrorKind.RequestRepresentation);
        transport.DefaultRequestHeaders.Clear();
        responder = (_, _) => { var response = Response(); response.Headers.TryAddWithoutValidation("X-Large", new string('x', 65_536)); return Task.FromResult(response); };
        await Failure(() => Call(), SdkErrorKind.ResourceLimit);
        responder = (_, _) => { var response = Response(); response.Headers.TryAddWithoutValidation("X-Many", Enumerable.Repeat("x", 257)); return Task.FromResult(response); };
        await Failure(() => Call(), SdkErrorKind.ResourceLimit);
        foreach (var server in new[] { "http://127.0.0.1:bad/api", "https://user:secret@example.test", "https://example.test/api/../escape", "https://example.test/api/%2E%2E", "https://example.test/api/%2e%2e%2fprivate", "https://example.test?x=y", "https://example.test/#fragment" })
        {
            try { using var bad = new Client(new Credentials(), new ClientOptions { ServerUrl = server }, transport); throw new Exception("Accepted invalid server"); }
            catch (SdkException error) { Check(error.Kind == SdkErrorKind.RequestRepresentation, "Invalid server classification"); }
        }
    }
    private static async Task CancellationAndCleanup()
    {
        using (var entered = new CancellationTokenSource())
        using (var cancellation = new CancellationTokenSource())
        {
            var pending = new TaskCompletionSource<HttpResponseMessage>(TaskCreationOptions.RunContinuationsAsynchronously);
            using var transport = new HttpClient(new Handler((_, _) => { entered.Cancel(); return pending.Task; }));
            using var client = new Client(new Credentials { ApiKey = "token" }, httpClient: transport);
            var call = client.GetWidgetAsync(new GetWidgetInput { WidgetId = "x" }, cancellationToken: cancellation.Token);
            Check(entered.IsCancellationRequested, "Send entered"); cancellation.Cancel();
            try { await call.WaitAsync(TimeSpan.FromSeconds(2)); throw new Exception("Expected cancellation"); }
            catch (OperationCanceledException error) { Check(error.CancellationToken == cancellation.Token, "Caller cancellation token preserved"); }
            var late = new ControlStream(Encoding.UTF8.GetBytes(WidgetJson)); pending.SetResult(Response(stream: late));
            await late.Disposed.Task.WaitAsync(TimeSpan.FromSeconds(2)); Check(true, "Late ignored-cancellation response disposed");
        }
        var blocked = new ControlStream(Array.Empty<byte>(), block: true, throwOnClose: true);
        using (var transport = new HttpClient(new Handler((_, _) => Task.FromResult(Response(stream: blocked)))))
        using (var client = new Client(new Credentials { ApiKey = "token" }, httpClient: transport))
        using (var cancellation = new CancellationTokenSource())
        {
            var call = client.GetWidgetAsync(new GetWidgetInput { WidgetId = "x" }, cancellationToken: cancellation.Token);
            await blocked.Entered.Task.WaitAsync(TimeSpan.FromSeconds(2)); cancellation.Cancel();
            try { await call.WaitAsync(TimeSpan.FromSeconds(2)); throw new Exception("Expected body cancellation"); }
            catch (OperationCanceledException) { Check(blocked.Disposed.Task.IsCompleted, "Cleanup cannot replace body cancellation"); }
        }
        foreach (var bodyPhase in new[] { false, true })
        {
            var stream = new ControlStream(Array.Empty<byte>(), block: true);
            var pending = new TaskCompletionSource<HttpResponseMessage>(TaskCreationOptions.RunContinuationsAsynchronously);
            using var transport = new HttpClient(new Handler((_, _) => bodyPhase ? Task.FromResult(Response(stream: stream)) : pending.Task));
            using var client = new Client(new Credentials { ApiKey = "token" }, new ClientOptions { Timeout = TimeSpan.FromMilliseconds(30) }, transport);
            await Failure(async () => await client.GetWidgetAsync(new GetWidgetInput { WidgetId = "x" }).WaitAsync(TimeSpan.FromSeconds(2)), SdkErrorKind.Timeout);
            if (bodyPhase) Check(stream.Disposed.Task.IsCompleted, "Deadline body disposed");
            else { pending.SetResult(Response(stream: stream)); await stream.Disposed.Task.WaitAsync(TimeSpan.FromSeconds(2)); }
        }
        using (var transport = new HttpClient(new Handler(async (_, token) => { await Task.Delay(Timeout.Infinite, token); return Response(); })) { Timeout = TimeSpan.FromMilliseconds(30) })
        using (var client = new Client(new Credentials { ApiKey = "token" }, httpClient: transport))
            await Failure(() => client.GetWidgetAsync(new GetWidgetInput { WidgetId = "x" }), SdkErrorKind.Timeout);
        var over = new ControlStream(Encoding.UTF8.GetBytes("abcdefgh"), throwOnClose: true);
        using (var transport = new HttpClient(new Handler((_, _) => Task.FromResult(Response(stream: over)))))
        using (var client = new Client(new Credentials { ApiKey = "token" }, new ClientOptions { MaxResponseBytes = 2, MaxCaptureBytes = 1 }, transport))
        {
            var error = await Failure(() => client.GetWidgetAsync(new GetWidgetInput { WidgetId = "x" }), SdkErrorKind.ResourceLimit);
            Check(over.BytesRead == 3 && over.Disposed.Task.IsCompleted && error.BodyCapture.SequenceEqual(new byte[] { (byte)'a' }) && error.CaptureTruncated, "Bounded reads, capture and primary failure survive failing close");
        }
        var closer = new ControlStream(Encoding.UTF8.GetBytes(WidgetJson), throwOnClose: true);
        using (var transport = new HttpClient(new Handler((_, _) => Task.FromResult(Response(stream: closer)))))
        using (var client = new Client(new Credentials { ApiKey = "token" }, httpClient: transport))
        {
            var error = await Failure(() => client.GetWidgetAsync(new GetWidgetInput { WidgetId = "x" }), SdkErrorKind.Transport);
            Check(!error.ToString().Contains("private-close-secret"), "Standalone cleanup failure typed and redacted");
        }
        using var cancelled = new CancellationTokenSource(); cancelled.Cancel();
        var sends = 0;
        using var http = new HttpClient(new Handler((_, _) => { sends++; return Task.FromResult(Response()); }));
        using var sdk = new Client(new Credentials { ApiKey = "token" }, httpClient: http);
        try { await sdk.CreateWidgetAsync(new CreateWidgetInput { Body = new WidgetInput { Name = "" } }, cancellationToken: cancelled.Token); throw new Exception("Expected pre-cancelled call"); }
        catch (OperationCanceledException) { Check(sends == 0, "Pre-cancelled call precedes validation and send"); }
    }
    private static async Task DefaultTransport()
    {
        await using var server = new Loopback();
        using var client = new Client(new Credentials { ApiKey = "m2-key" }, new ClientOptions { ServerUrl = server.Url + "/api/v1" });
        await Failure(() => client.GetWidgetAsync(new GetWidgetInput { WidgetId = "a/b 雪" }), SdkErrorKind.UnexpectedResponse);
        Check((await client.GetWidgetAsync(new GetWidgetInput { WidgetId = ".." })).Data.Id == "w1", "Default transport response");
        await client.GetWidgetAsync(new GetWidgetInput { WidgetId = "." });
        await Failure(() => client.GetWidgetAsync(new GetWidgetInput { WidgetId = "retry" }), SdkErrorKind.UnexpectedResponse);
        var seen = server.Requests.ToArray();
        Check(seen.Length == 4, "No redirects or retries");
        Check(seen.Select(s => s.Split("\r\n")[0]).SequenceEqual(new[] { "GET /api/v1/widgets/a%2Fb%20%E9%9B%AA HTTP/1.1", "GET /api/v1/widgets/%2E%2E HTTP/1.1", "GET /api/v1/widgets/%2E HTTP/1.1", "GET /api/v1/widgets/retry HTTP/1.1" }), "Actual escaped request targets");
        foreach (var request in seen)
        {
            Check(request.Contains("Authorization: Bearer m2-key\r\n", StringComparison.OrdinalIgnoreCase), "Explicit bearer on default transport");
            Check(!request.Contains("\r\nCookie:", StringComparison.OrdinalIgnoreCase) && !request.Contains("Proxy-Authorization:", StringComparison.OrdinalIgnoreCase), "No persisted cookies or implicit proxy auth");
        }
        await using var oversized = new Loopback(largeHeaders: true);
        using var bounded = new Client(new Credentials { ApiKey = "token" }, new ClientOptions { ServerUrl = oversized.Url, MaxHeaderBytes = 1024 });
        await Failure(() => bounded.GetWidgetAsync(new GetWidgetInput { WidgetId = "x" }), SdkErrorKind.ResourceLimit);
    }
    private static void Documentation(string package)
    {
        using var archive = ZipFile.OpenRead(package);
        string Read(string path) { using var stream = archive.GetEntry(path)!.Open(); using var reader = new StreamReader(stream); return reader.ReadToEnd(); }
        var native = XDocument.Parse(Read("lib/net8.0/Suspect.Csharp.M2.xml")).Descendants("member").Select(e => (string)e.Attribute("name")!).ToHashSet();
        using var reference = JsonDocument.Parse(Read("docs/reference.json")); var html = Read("docs/index.html");
        foreach (var symbol in reference.RootElement.GetProperty("symbols").EnumerateArray())
        {
            Check(native.Contains(symbol.GetProperty("xmlId").GetString()!), "Native compiler documentation identity: " + symbol.GetProperty("xmlId"));
            Check(html.Contains("id=\"" + symbol.GetProperty("id").GetString() + "\""), "Browsable native symbol anchor");
        }
        Check(archive.GetEntry("examples/Program.cs") is not null && archive.GetEntry("examples/examples.json") is not null, "Packaged executable samples and provenance");
    }
}

sealed class Handler(Func<HttpRequestMessage, CancellationToken, Task<HttpResponseMessage>> send) : HttpMessageHandler
{
    internal bool Closed;
    protected override Task<HttpResponseMessage> SendAsync(HttpRequestMessage request, CancellationToken token) => send(request, token);
    protected override void Dispose(bool disposing) { Closed = true; base.Dispose(disposing); }
}

sealed class ControlStream(byte[] bytes, bool block = false, bool throwOnClose = false) : Stream
{
    private int _position;
    internal int BytesRead => _position;
    internal TaskCompletionSource Disposed { get; } = new(TaskCreationOptions.RunContinuationsAsynchronously);
    internal TaskCompletionSource Entered { get; } = new(TaskCreationOptions.RunContinuationsAsynchronously);
    public override bool CanRead => true; public override bool CanWrite => false; public override bool CanSeek => false;
    public override long Length => throw new NotSupportedException(); public override long Position { get => _position; set => throw new NotSupportedException(); }
    public override int Read(byte[] buffer, int offset, int count) => ReadAsync(buffer.AsMemory(offset, count)).AsTask().GetAwaiter().GetResult();
    public override async ValueTask<int> ReadAsync(Memory<byte> buffer, CancellationToken token = default)
    {
        Entered.TrySetResult();
        if (block) await Task.Delay(Timeout.Infinite, token);
        var count = Math.Min(buffer.Length, bytes.Length - _position); bytes.AsMemory(_position, count).CopyTo(buffer); _position += count; return count;
    }
    protected override void Dispose(bool disposing) { Disposed.TrySetResult(); base.Dispose(disposing); if (throwOnClose) throw new IOException("private-close-secret"); }
    public override void Flush() { } public override long Seek(long offset, SeekOrigin origin) => throw new NotSupportedException();
    public override void SetLength(long value) => throw new NotSupportedException(); public override void Write(byte[] buffer, int offset, int count) => throw new NotSupportedException();
}

sealed class Loopback : IAsyncDisposable
{
    private readonly TcpListener _listener = new(IPAddress.Loopback, 0);
    private readonly CancellationTokenSource _stop = new();
    private readonly Task _serve;
    private readonly bool _largeHeaders;
    internal readonly System.Collections.Concurrent.ConcurrentQueue<string> Requests = new();
    internal string Url { get; }
    internal Loopback(bool largeHeaders = false) { _largeHeaders = largeHeaders; _listener.Start(); Url = "http://127.0.0.1:" + ((IPEndPoint)_listener.LocalEndpoint).Port; _serve = Serve(); }
    private async Task Serve()
    {
        try
        {
            while (!_stop.IsCancellationRequested)
            {
                using var connection = await _listener.AcceptTcpClientAsync(_stop.Token); using var stream = connection.GetStream();
                var header = new List<byte>(); var one = new byte[1];
                while (header.Count < 65_536)
                {
                    if (await stream.ReadAsync(one, _stop.Token) == 0) throw new Exception("Incomplete request"); header.Add(one[0]);
                    if (header.Count >= 4 && header[^4] == 13 && header[^3] == 10 && header[^2] == 13 && header[^1] == 10) break;
                }
                Requests.Enqueue(Encoding.ASCII.GetString(header.ToArray())); var n = Requests.Count;
                var body = Encoding.UTF8.GetBytes(Probe.WidgetJson);
                var status = n == 1 ? "302 Found" : n == 4 ? "503 Unavailable" : "200 OK";
                var extra = _largeHeaders ? "X-Large: " + new string('h', 2048) + "\r\n" : "";
                var response = Encoding.ASCII.GetBytes($"HTTP/1.1 {status}\r\n{extra}Content-Type: Application/JSON; charset=\"UTF-8\"\r\nContent-Length: {body.Length}\r\nLocation: /followed\r\nSet-Cookie: session=private-cookie\r\nRetry-After: 0\r\nConnection: close\r\n\r\n");
                await stream.WriteAsync(response, _stop.Token); await stream.WriteAsync(body, _stop.Token);
            }
        }
        catch (OperationCanceledException) when (_stop.IsCancellationRequested) { }
    }
    public async ValueTask DisposeAsync() { _stop.Cancel(); _listener.Stop(); await _serve; _stop.Dispose(); }
}
