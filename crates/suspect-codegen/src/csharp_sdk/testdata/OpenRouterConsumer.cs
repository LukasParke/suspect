// Independent installed consumer of the five actual public OpenRouter operations.
// Responses are copied unchanged from tests/fixtures/openrouter-five-responses.json.
using Suspect.Csharp.OpenRouter;
using System.IO.Compression;
using System.Net;
using System.Net.Sockets;
using System.Runtime.InteropServices;
using System.Text;
using System.Text.Json;
using System.Xml.Linq;

static class Probe
{
    private static int _checks;
    private static void Check(bool value, string message) { if (!value) throw new Exception(message); _checks++; }
    public static async Task Main(string[] args)
    {
        using var fixtures = JsonDocument.Parse(File.ReadAllBytes("responses.json"));
        var bodies = new[] { "credits", "create", "update", "list", "file" }.Select(name => fixtures.RootElement.GetProperty(name).GetString()!).ToArray();
        await using var server = new RecordingServer(bodies);
        using var client = new Client(new Credentials { ApiKey = "test-key" }, new ClientOptions { ServerUrl = server.Url + "/api/v1" });
        var credits = await client.GetCreditsAsync();
        Check(credits.Data.Data.TotalCredits.Token == "100.50000000000000001", "Exact credits");
        Check(credits.Data.Data.TotalUsage.Token == "25.75", "Usage token");
        var created = await client.CreateKeysAsync(new CreateKeysInput
        {
            Body = new __CREATE__ { Name = "Native Test Key", Limit = new JsonNumber("50.25"), LimitReset = Optional<__CREATE_RESET__>.Present(null) }
        });
        Check(created.Status == 201 && created.Data.Data.Limit!.Value.Token == "50.250", "Native create response and numeric spelling");
        var updated = await client.UpdateKeysAsync(new UpdateKeysInput
        {
            Hash = "fixture-hash", Body = new __UPDATE__ { Name = "Updated Native Key", Limit = new JsonNumber("75.50"), LimitReset = Optional<__UPDATE_RESET__>.Present(null), Disabled = true }
        });
        Check(updated.Data.Data.Limit!.Value.Token == "75.50", "Native update response");
        var files = await client.ListContainerFilesAsync(new ListContainerFilesInput { ContainerId = "sess_abc123", Limit = new JsonInteger("2"), After = "a/b 雪" });
        Check(!files.Data.HasMore && files.Data.Data.Count == 1, "Source list model; no inferred pagination");
        var file = await client.GetContainerFileAsync(new GetContainerFileInput { ContainerId = "sess_abc123", FileId = "a/b 雪" });
        Check(file.Data.Bytes.Token == "123" && file.Data.Bytes.ToBigInteger() == 123, "Exact integral bytes");
        var expected = new[] {
            ("GET", "/api/v1/credits", "Bearer test-key", ""),
            ("POST", "/api/v1/keys", "Bearer test-key", "{\"limit\":50.25,\"limit_reset\":null,\"name\":\"Native Test Key\"}"),
            ("PATCH", "/api/v1/keys/fixture-hash", "Bearer test-key", "{\"disabled\":true,\"limit\":75.50,\"limit_reset\":null,\"name\":\"Updated Native Key\"}"),
            ("GET", "/api/v1/containers/sess_abc123/files?limit=2&after=a%2Fb%20%E9%9B%AA", "Bearer test-key", ""),
            ("GET", "/api/v1/containers/sess_abc123/files/a%2Fb%20%E9%9B%AA", "Bearer test-key", "")
        };
        Check(server.Requests.SequenceEqual(expected), "Independent actual OpenRouter wire requests: " + string.Join(";", server.Requests));
        using var archive = ZipFile.OpenRead(args[0]);
        string Read(string path) { using var reader = new StreamReader(archive.GetEntry(path)!.Open()); return reader.ReadToEnd(); }
        var native = XDocument.Parse(Read("lib/net8.0/Suspect.Csharp.OpenRouter.xml")).Descendants("member").Select(member => (string)member.Attribute("name")!).ToHashSet();
        using var reference = JsonDocument.Parse(Read("docs/reference.json")); var html = Read("docs/index.html");
        foreach (var symbol in reference.RootElement.GetProperty("symbols").EnumerateArray())
        {
            Check(native.Contains(symbol.GetProperty("xmlId").GetString()!), "Actual compiler XML symbol: " + symbol.GetProperty("xmlId"));
            Check(html.Contains("id=\"" + symbol.GetProperty("id").GetString() + "\""), "Browsable symbol");
        }
        Console.WriteLine($"OPENROUTER FIVE PASS: {_checks} checks; {RuntimeInformation.FrameworkDescription}; installed assembly {typeof(Client).Assembly.Location}");
    }
}

sealed class RecordingServer(string[] bodies) : IAsyncDisposable
{
    private readonly TcpListener _listener = new(IPAddress.Loopback, 0);
    private readonly CancellationTokenSource _stop = new();
    private Task? _serve;
    internal readonly System.Collections.Concurrent.ConcurrentQueue<(string Method, string Target, string Auth, string Body)> Requests = new();
    private string? _url;
    internal string Url
    {
        get
        {
            if (_url is not null) return _url;
            _listener.Start(); _url = "http://127.0.0.1:" + ((IPEndPoint)_listener.LocalEndpoint).Port; _serve = Serve(); return _url;
        }
    }
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
                var lines = Encoding.ASCII.GetString(header.ToArray()).Split("\r\n", StringSplitOptions.RemoveEmptyEntries);
                var requestLine = lines[0].Split(' '); var headers = lines.Skip(1).Select(line => line.Split(':', 2)).ToDictionary(pair => pair[0], pair => pair[1].Trim(), StringComparer.OrdinalIgnoreCase);
                if (headers.ContainsKey("Cookie")) throw new Exception("Implicit cookie");
                var body = new byte[headers.TryGetValue("Content-Length", out var length) ? int.Parse(length, System.Globalization.CultureInfo.InvariantCulture) : 0];
                await stream.ReadExactlyAsync(body, _stop.Token);
                if (body.Length != 0 && headers["Content-Type"] != "application/json") throw new Exception("Wrong request media");
                var index = Requests.Count;
                Requests.Enqueue((requestLine[0], requestLine[1], headers["Authorization"], Encoding.UTF8.GetString(body)));
                var output = Encoding.UTF8.GetBytes(bodies[index]); var status = index == 1 ? "201 Created" : "200 OK";
                await stream.WriteAsync(Encoding.ASCII.GetBytes($"HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {output.Length}\r\nSet-Cookie: session=ignored\r\nConnection: close\r\n\r\n"), _stop.Token);
                await stream.WriteAsync(output, _stop.Token);
            }
        }
        catch (OperationCanceledException) when (_stop.IsCancellationRequested) { }
    }
    public async ValueTask DisposeAsync() { _stop.Cancel(); _listener.Stop(); if (_serve is not null) await _serve; _stop.Dispose(); }
}
