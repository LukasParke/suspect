// Installed package consumers, independent domain examples and actual TCP bytes.
using Resources.Csharp;
using System.Collections.Concurrent;
using System.IO.Compression;
using System.Net;
using System.Net.Sockets;
using System.Reflection;
using System.Runtime.CompilerServices;
using System.Text;
using System.Text.Json;
using System.Xml.Linq;
using Tree = Resources.Csharp.__TREE__;
using Envelope = Resources.Csharp.__ENVELOPE__;
using NullableEnvelope = Resources.Csharp.__NULLABLEENVELOPE__;

internal static class Probe
{
    private static int checks;
    private static void Check(bool value,string message) {if(!value)throw new Exception(message);checks++;}
    private static JsonElement Json(string text) {using var document=JsonDocument.Parse(text);return document.RootElement.Clone();}
    private static void Invalid(Action action,string source,string path)
    {
        try {action();throw new Exception("Expected source codec refusal");}
        catch(CodecException error){Check(error.Kind==CodecErrorKind.InvalidValue,"Codec failure category");Check(error.SchemaSource==source,"Dynamic finding physical source: "+error.SchemaSource);Check(error.InstancePath==path,"Dynamic finding instance: "+error.InstancePath);}
    }
    internal static async Task Main(string[] args)
    {
        const string entry="http://api.csharp.test/v1/spec/openapi.json#";
        await using var server=new Server();var hosts=new ConcurrentQueue<string>();
        using var handler=new SocketsHttpHandler{AllowAutoRedirect=false,UseProxy=false,ConnectCallback=async(context,token)=>
        {
            hosts.Enqueue(context.DnsEndPoint.Host);
            var socket=new Socket(AddressFamily.InterNetwork,SocketType.Stream,ProtocolType.Tcp);
            await socket.ConnectAsync(IPAddress.Loopback,server.Port,token);return new NetworkStream(socket,true);
        }};
        using var http=new HttpClient(handler);using var client=new Client(new Credentials(),httpClient:http);
        var tree=new Tree{Name="root",Amount=new JsonNumber("9007199254740993.000000000000000001"),Children=new List<JsonElement>{Json("{\"name\":\"leaf\",\"amount\":9007199254740994e0}")}};
        await using(var result=await client.SendTreeAsync(new SendTreeInput{Body=tree}))
        {
            Check(result.Data.Name=="root"&&result.Data.Amount.Token==tree.Amount.Token,"Typed dynamic-tree root and exact decimal token");
            Check(result.Data.Children.Value[0].GetProperty("amount").GetRawText()=="9007199254740994e0","Dynamic leaves preserve native JSON tokens");
            Check(!result.Data.Note.HasValue,"Absent optional value retained");
        }
        tree.Note=Optional<string?>.Present(null);
        Check(Json(Encoding.UTF8.GetString(Codecs.Encode__TREE__(tree))).GetProperty("note").ValueKind==JsonValueKind.Null,"Static nullable member keeps presence beside dynamic children");
        var strict=Json("{\"name\":\"strict\",\"amount\":9007199254740993.00,\"children\":[{\"name\":\"leaf\",\"amount\":9007199254740994}]}");
        await using(var result=await client.SendStrictTreeAsync(new SendStrictTreeInput{Body=strict}))
            Check(result.Data.GetProperty("children")[0].GetProperty("name").GetString()=="leaf","Recursive strict dynamic override on native response");
        var badStrict=Json("{\"name\":\"strict\",\"amount\":9007199254740993,\"children\":[{\"name\":\"leaf\",\"amount\":9007199254740994,\"leak\":1}]}");
        Invalid(()=>Codecs.Encode__STRICT__(badStrict),entry+"/components/schemas/Strict/unevaluatedProperties","/children/0/leak");
        var loose=Codecs.Decode__TREE__(badStrict.GetRawText());
        Check(loose.Children.Value[0].GetProperty("leak").GetInt32()==1,"Unentered Strict candidate does not affect a Tree root");
        var envelope=Codecs.Decode__ENVELOPE__("{\"id\":\"env\",\"value\":1.2e1,\"other\":13.00}");
        await using(var result=await client.SendEnvelopeAsync(new SendEnvelopeInput{Body=envelope}))
        {
            Check(result.Data.Value.GetRawText()=="1.2e1"&&result.Data.Extra["other"].GetRawText()=="13.00","Outer integer binding controls dynamic fields and named extras");
            Check(result.Data.Id=="env","Declared envelope field remains a CLR string");
        }
        envelope.Value=Json("\"fallback-string\"");
        Invalid(()=>Codecs.Encode__ENVELOPE__(envelope),entry+"/components/schemas/Envelope/$defs/Binding/type","/value");
        var before=server.Requests.Count;
        try {await client.SendEnvelopeAsync(new SendEnvelopeInput{Body=envelope});throw new Exception("Expected pretransport validation");}
        catch(SdkException error){Check(error.Kind==SdkErrorKind.RequestValidation&&error.CodecError!.SchemaSource==entry+"/components/schemas/Envelope/$defs/Binding/type","Request uses dynamic source validation");Check(error.OperationSource==entry+"/paths/~1envelope/post","Request finding retains physical operation source");}
        Check(server.Requests.Count==before,"Dynamic invalidity fails before actual transport");
        envelope.Value=Json("12");envelope.Extra["other"]=Json("9");
        Invalid(()=>Codecs.Encode__ENVELOPE__(envelope),entry+"/components/schemas/Envelope/$defs/Binding/minimum","/other");
        envelope.Extra["other"]=Json("13");
        var nullable=new NullableEnvelope{Id="absent"};
        await using(var result=await client.SendNullableEnvelopeAsync(new SendNullableEnvelopeInput{Body=nullable}))
            Check(!result.Data.Value.HasValue&&!Json(server.Requests.Last().Body).TryGetProperty("value",out _),"Context-dependent union preserves optional absence");
        nullable.Value=Json("null");
        await using(var result=await client.SendNullableEnvelopeAsync(new SendNullableEnvelopeInput{Body=nullable}))
            Check(result.Data.Value.HasValue&&result.Data.Value.Value.ValueKind==JsonValueKind.Null,"Parent resource permits present null that standalone union rejects");
        nullable.Value=Json("2.0");
        await using(var result=await client.SendNullableEnvelopeAsync(new SendNullableEnvelopeInput{Body=nullable}))
            Check(result.Data.Value.Value.GetRawText()=="2.0","Dynamic union conversion does not run standalone branch trials");
        nullable.Value=Json("\"wrong-in-parent\"");
        Invalid(()=>Codecs.Encode__NULLABLEENVELOPE__(nullable),"http://storage.csharp.test/schema/choice.json#/anyOf","/value");
        var choice=Codecs.Decode__CHOICE__("\"standalone\"");
        await using(var result=await client.SendChoiceAsync(new SendChoiceInput{Body=choice}))
            Check(result.Data.GetString()=="standalone","Standalone union uses initial string target after outer call scope restores");
        Invalid(()=>Codecs.Decode__CHOICE__("null"),"http://storage.csharp.test/schema/choice.json#/anyOf","");
        await using(var result=await client.SendFallbackAsync(new SendFallbackInput{Body=Json("\"fallback\"")}))
            Check(result.Data.GetString()=="fallback","Raw dynamicRef codec is a checked JSON carrier");
        var number=new JsonInteger("9007199254740993e0");
        await using(var result=await client.SendIntegerAsync(new SendIntegerInput{Body=number,__REVISION__=number}))
        {
            Check(result.Data.Token==number.Token&&server.Requests.Last().Body==number.Token,"Static canonical resource retains exact integer native and wire types");
            Check(server.Requests.Last().Revision==number.Token,"Source-linked exact integer header codec");
        }
        Invalid(()=>Codecs.Encode__INTEGER__(new JsonInteger("9007199254740992")),"http://storage.csharp.test/schema/catalog.json#/$defs/a~1b~0% #é/minimum","");
        await using(var result=await client.SendNestedAsync(new SendNestedInput{Body=Json("7.0")}))
            Check(result.Data.GetRawText()=="7.0","Detached nested entry enters indexed parent without executing false resource root");
        Invalid(()=>Codecs.Encode__NESTED__(Json("\"fallback\"")),"http://storage.csharp.test/schema/detached.json#/$defs/Binding/type","");
        server.Responses.Enqueue((422,null));
        try {await client.SendStrictTreeAsync(new SendStrictTreeInput{Body=strict});throw new Exception("Expected typed API error");}
        catch(__STRICT_ERROR__ error){await using(error){Check(error.Data.GetProperty("name").GetString()=="strict"&&error.Response!.Status==422,"Installed typed error payload runs resource program");}}
        server.Responses.Enqueue((200,badStrict.GetRawText()));
        try {await client.SendStrictTreeAsync(new SendStrictTreeInput{Body=strict});throw new Exception("Expected response source validation");}
        catch(SdkException error)
        {
            Check(error.Kind==SdkErrorKind.ResponseDecode&&error.CodecError!.SchemaSource==entry+"/components/schemas/Strict/unevaluatedProperties","Invalid dynamic response retains overriding keyword");
            Check(error.CodecError!.InstancePath=="/children/0/leak"&&Encoding.UTF8.GetString(error.BodyCapture)==badStrict.GetRawText(),"Response source path and explicit exact capture");
        }
        var bytes=Encoding.UTF8.GetBytes(strict.GetRawText());var owned=Codecs.Decode__STRICT__(bytes);Array.Fill(bytes,(byte)' ');
        Check(owned.GetProperty("name").GetString()=="strict","Resource-checked carrier owns input storage");
        var values=await Task.WhenAll(Enumerable.Range(0,24).Select(i=>Task.Run(()=>i%2==0?Codecs.Decode__CHOICE__("\"standalone\""):Codecs.Decode__NULLABLEENVELOPE__("{\"id\":\"n\",\"value\":null}").Value.Value)));
        Check(values.Where((_,i)=>i%2==0).All(v=>v.GetString()=="standalone")&&values.Where((_,i)=>i%2!=0).All(v=>v.ValueKind==JsonValueKind.Null),"Concurrent codec sessions isolate dynamic contexts");
        Check(typeof(Tree).GetProperty(nameof(Tree.Amount))!.PropertyType==typeof(JsonNumber)&&typeof(Tree).GetProperty(nameof(Tree.Amount))!.GetCustomAttribute<RequiredMemberAttribute>() is not null,"Installed CLR exact-number requiredness");
        Check(typeof(Envelope).GetProperty(nameof(Envelope.Value))!.PropertyType==typeof(JsonElement)&&typeof(Envelope).GetProperty(nameof(Envelope.Extra))!.PropertyType==typeof(Dictionary<string,JsonElement>),"Dynamic candidate never becomes a static field/extra CLR type");
        Check(typeof(NullableEnvelope).GetProperty(nameof(NullableEnvelope.Value))!.PropertyType==typeof(Optional<JsonElement>),"Installed optional dynamic union carrier");
        Check(typeof(SendChoiceInput).GetProperty("Body")!.PropertyType==typeof(JsonElement)&&typeof(SendIntegerInput).GetProperty("Body")!.PropertyType==typeof(JsonInteger),"Carrier and static resource types remain distinct");
        using(var package=ZipFile.OpenRead(args[0]))
        {
            string Read(string path){using var reader=new StreamReader(package.GetEntry(path)!.Open());return reader.ReadToEnd();}
            var xmlText=Read("lib/net8.0/Resources.Csharp.xml");var xml=XDocument.Parse(xmlText).Descendants("member").Select(m=>(string)m.Attribute("name")!).ToHashSet();
            using var reference=JsonDocument.Parse(Read("docs/reference.json"));var html=Read("docs/index.html");
            Check(reference.RootElement.GetProperty("validation").GetProperty("profile").GetString()=="oas31-jsonschema202012-resources-dynamic","Installed rendered reference records exact v3 profile");
            foreach(var symbol in reference.RootElement.GetProperty("symbols").EnumerateArray())
            {Check(xml.Contains(symbol.GetProperty("xmlId").GetString()!),"Installed XML native symbol");Check(html.Contains("id=\""+symbol.GetProperty("id").GetString()+"\"",StringComparison.Ordinal),"Rendered source/native anchor");}
            Check(Read("README.md").Contains("Resource-scoped source validation",StringComparison.Ordinal)&&html.Contains("Indexed schema resources",StringComparison.Ordinal),"Installed guide explains dynamic carrier and indexed scope obligations");
            Check(xmlText.Contains("https://schemas.csharp.test/catalog",StringComparison.Ordinal)&&xmlText.Contains("http://storage.csharp.test/schema/catalog.json",StringComparison.Ordinal),"Compiler XML keeps logical metadata separate from physical ownership");
            using var examples=JsonDocument.Parse(Read("examples/examples.json"));Check(examples.RootElement.GetProperty("diagnostics").GetArrayLength()==0&&examples.RootElement.GetProperty("operations").GetArrayLength()==8,"Source-bound installed example coverage");
        }
        using(var stream=typeof(Codecs).Assembly.GetManifestResourceStream("Suspect.ValidationProgram.json")!)
        using(var document=JsonDocument.Parse(stream))
        {
            var program=document.RootElement;var resources=program.GetProperty("resourceContext");
            Check(program.GetProperty("version").GetString()=="suspect.validation.experimental.v3"&&resources.GetProperty("nodeScopes").GetArrayLength()==program.GetProperty("nodes").GetArrayLength(),"Installed executable V3 has aligned node scopes");
            Check(resources.GetProperty("resources").EnumerateArray().Any(r=>r.GetProperty("canonicalUri").GetString()=="https://schemas.csharp.test/catalog"&&r.GetProperty("source").GetProperty("document").GetString()=="http://storage.csharp.test/schema/catalog.json"&&r.GetProperty("aliases").EnumerateArray().Any(a=>a.GetString()=="http://requested.csharp.test/catalog.json")),"Indexed resource retains effective and requested physical aliases");
            Check(resources.GetProperty("nodeScopes").EnumerateArray().Any(s=>s[2].GetString()=="https://schemas.csharp.test/catalog#/$defs/a~1b~0%25%20%23%C3%A9"),"Escaped resource-relative canonical address is copied exactly");
            Check(!program.GetProperty("nodes").EnumerateArray().Any(n=>n.GetProperty("source").GetProperty("document").GetString()=="http://storage.csharp.test/schema/detached.json"&&n.GetProperty("source").GetProperty("pointer").GetString()==""),"Installed nested-entry program did not invent a root assertion");
        }
        Check(server.Requests.Select(r=>r.Target).Distinct().Count()==8,"All eight actual source operation paths exercised");
        Check(server.Requests.All(r=>r.Method=="POST"&&r.Host=="api.csharp.test"&&r.Target.StartsWith("/v1/Api%2Fv3/",StringComparison.Ordinal)),"Physical server document stays independent of schema and logical IDs");
        Check(hosts.Count==server.Requests.Count&&hosts.All(h=>h=="api.csharp.test"),"No runtime schema acquisition occurred");
        File.WriteAllText("sdk-results.json",JsonSerializer.Serialize(new{checks,requests=server.Requests.ToArray(),hosts=hosts.ToArray(),framework=System.Runtime.InteropServices.RuntimeInformation.FrameworkDescription},new JsonSerializerOptions{WriteIndented=true}));
        Console.WriteLine($"RESOURCE SDK PASS: {checks} checks; {server.Requests.Count} TCP requests; eight operations; {System.Runtime.InteropServices.RuntimeInformation.FrameworkDescription}");
    }
}
internal sealed record Seen(string Method,string Target,string Host,string Body,string? Revision);
internal sealed class Server:IAsyncDisposable
{
    private readonly TcpListener listener=new(IPAddress.Loopback,0);private readonly CancellationTokenSource stop=new();private readonly Task worker;
    internal readonly ConcurrentQueue<Seen> Requests=new();internal readonly ConcurrentQueue<(int Status,string? Body)> Responses=new();internal int Port{get;}
    internal Server(){listener.Start();Port=((IPEndPoint)listener.LocalEndpoint).Port;worker=Run();}
    private async Task Run()
    {
        try {while(!stop.IsCancellationRequested)
        {
            using var socket=await listener.AcceptTcpClientAsync(stop.Token);using var stream=socket.GetStream();var bytes=new List<byte>();var one=new byte[1];
            while(bytes.Count<65536){if(await stream.ReadAsync(one,stop.Token)==0)throw new Exception("Incomplete request");bytes.Add(one[0]);if(bytes.Count>=4&&bytes[^4]==13&&bytes[^3]==10&&bytes[^2]==13&&bytes[^1]==10)break;}
            var lines=Encoding.ASCII.GetString(bytes.ToArray()).Split("\r\n",StringSplitOptions.RemoveEmptyEntries);var start=lines[0].Split(' ');
            var headers=lines.Skip(1).Select(line=>line.Split(':',2)).ToDictionary(p=>p[0],p=>p[1].Trim(),StringComparer.OrdinalIgnoreCase);
            var length=int.Parse(headers["Content-Length"],System.Globalization.CultureInfo.InvariantCulture);var body=new byte[length];await stream.ReadExactlyAsync(body,stop.Token);
            Requests.Enqueue(new Seen(start[0],start[1],headers["Host"],Encoding.UTF8.GetString(body),headers.GetValueOrDefault("X-Revision")));
            var response=Responses.TryDequeue(out var reply)?reply:(200,(string?)null);var payload=response.Item2 is null?body:Encoding.UTF8.GetBytes(response.Item2);
            var head=Encoding.ASCII.GetBytes($"HTTP/1.1 {response.Item1} Result\r\nContent-Type: application/json\r\nContent-Length: {payload.Length}\r\nConnection: close\r\n\r\n");
            await stream.WriteAsync(head,stop.Token);await stream.WriteAsync(payload,stop.Token);
        }}catch(OperationCanceledException)when(stop.IsCancellationRequested){}catch(SocketException)when(stop.IsCancellationRequested){}
    }
    public async ValueTask DisposeAsync(){stop.Cancel();listener.Stop();await worker;stop.Dispose();}
}
