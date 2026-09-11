// Independent MIME expectations, source-native constructors and installed CLR metadata.
using Positional.Csharp;
using System.Collections.Concurrent;
using System.IO.Compression;
using System.Net;
using System.Net.Sockets;
using System.Reflection;
using System.Text;
using System.Text.Json;
using System.Xml.Linq;
using Sequence = Positional.Csharp.__SEQUENCE__;
using TextPart = Positional.Csharp.__SEQUENCE_PART1__;
using TextHeaders = Positional.Csharp.__SEQUENCE_HEADER1__;
using JsonPart = Positional.Csharp.__SEQUENCE_PART2__;
using JsonHeaders = Positional.Csharp.__SEQUENCE_HEADER2__;
using BytePart = Positional.Csharp.__SEQUENCE_ITEM__;
using ByteHeaders = Positional.Csharp.__SEQUENCE_ITEM_HEADERS__;

static class Probe
{
    private static int checks;
    internal static readonly byte[] Binary = new byte[]{0,255}.Concat(Encoding.ASCII.GetBytes("\r\n--boundZ")).ToArray();
    internal static void Check(bool value,string reason){if(!value)throw new Exception(reason);checks++;}
    private static async Task<SdkException> Failure(Func<Task> action,SdkErrorKind kind)
    {try{await action();throw new Exception("Expected "+kind);}catch(SdkException error){Check(error.Kind==kind,"Expected "+kind+", got "+error.Kind);return error;}}
    private static Sequence Value()=>new()
    {
        Item1=new TextPart {Value="native",Headers=new TextHeaders{XOrdinal=new JsonInteger("1.00")}},
        Item2=new JsonPart {Value=new Payload{Amount=new JsonNumber("9007199254740993.000000000000000001")},Headers=new JsonHeaders{XTrace="trace"}},
        Items=new List<BytePart>{new(){Value=new byte[]{0,255,42},Headers=new ByteHeaders{XIndex=2}}}
    };
    public static async Task Main(string[] args)
    {
        await using var server=new MimeServer();
        using var client=new Client(new Credentials(),new ClientOptions{ServerUrl=server.Url});
        var options=new RequestOptions{ContentType="multipart/mixed; boundary=witness"};
        var result=await client.SendSequenceAsync(new SendSequenceInput{Body=Value()},options);
        var request=server.Requests.Last();
        var expected=Encoding.UTF8.GetBytes("--witness\r\nContent-Type: text/plain\r\nX-Ordinal: 1.00\r\n\r\nnative\r\n--witness\r\nContent-Type: application/json\r\nX-Trace: trace\r\n\r\n{\"amount\":9007199254740993.000000000000000001}\r\n--witness\r\nContent-Type: application/octet-stream\r\nX-Index: 2\r\n\r\n").Concat(new byte[]{0,255,42}).Concat(Encoding.ASCII.GetBytes("\r\n--witness--\r\n")).ToArray();
        Check(request.Method=="POST"&&request.Target=="/sequence","Source operation");
        Check(request.Body.SequenceEqual(expected),"Independent positional request MIME bytes");
        Check(!Encoding.Latin1.GetString(request.Body).Contains("name="),"No invented positional part names");
        Check(result.Data.Item1.Value=="first"&&result.Data.Item1.Headers.XOrdinal.Token=="1.0","Prefix text and typed header codec");
        Check(result.Data.Item1.ContentType is null,"Absent Content-Type remains absent metadata");
        Check(result.Data.Item2.HasValue&&result.Data.Item2.Value.Value.Amount.Token=="9007199254740993.000000000000000001","Exact JSON prefix codec");
        Check(result.Data.Item2.Value.Headers.XTrace.Value=="line one","MIME header continuation");
        Check(result.Data.Items.Count==1&&result.Data.Items[0].Value.SequenceEqual(Binary)&&result.Data.Items[0].Headers.XIndex.Token=="2e0","Typed repeating bytes and false-boundary substring preserved");

        var count=server.Requests.Count;
        var gap=Value();gap.Item2=default;await Failure(()=>client.SendSequenceAsync(new SendSequenceInput{Body=gap}),SdkErrorKind.RequestRepresentation);Check(server.Requests.Count==count,"Tail cannot skip an absent prefix");
        var tooMany=Value();tooMany.Items.Add(tooMany.Items[0]);tooMany.Items.Add(tooMany.Items[0]);await Failure(()=>client.SendSequenceAsync(new SendSequenceInput{Body=tooMany}),SdkErrorKind.RequestValidation);Check(server.Requests.Count==count,"maxItems before transport");
        var bad=Value();bad.Item2.Value.Value.Amount=default;var invalid=await Failure(()=>client.SendSequenceAsync(new SendSequenceInput{Body=bad}),SdkErrorKind.RequestValidation);Check(invalid.CodecError?.Kind==CodecErrorKind.Conversion&&server.Requests.Count==count,"Mutated prefix JSON validates before send");
        bad=Value();bad.Item1.Headers.XOrdinal=default;await Failure(()=>client.SendSequenceAsync(new SendSequenceInput{Body=bad}),SdkErrorKind.RequestValidation);Check(server.Requests.Count==count,"Invalid source-typed part header before send");
        var minimal=new Sequence{Item1=new TextPart{Value="minimum",Headers=new TextHeaders{XOrdinal=1}}};
        await client.SendSequenceAsync(new SendSequenceInput{Body=minimal},options);Check(Encoding.UTF8.GetString(server.Requests.Last().Body).Split("--witness").Length==3,"Optional suffix absent with an empty tail");

        server.Mode="empty";var optional=await client.SendOptionalAsync();Check(!optional.Data.Item1.HasValue,"Empty positional response is a zero-length sequence");
        Check(server.Requests.Last().Body.Length==0&&!server.Requests.Last().Headers.ContainsKey("Content-Type"),"Optional body absence");
        await client.SendOptionalAsync(new SendOptionalInput{Body=new __OPTIONAL__()},options);Check(Encoding.ASCII.GetString(server.Requests.Last().Body)=="--witness--\r\n","Present empty multipart differs from an absent body");
        server.Mode="headerless";optional=await client.SendOptionalAsync(new SendOptionalInput{Body=new __OPTIONAL__{Item1=new __OPTIONAL_PART1__{Value=""}}},options);
        Check(optional.Data.Item1.HasValue&&optional.Data.Item1.Value.Value==""&&optional.Data.Item1.Value.ContentType is null,"Headerless empty text part is not absence");
        Check(Encoding.ASCII.GetString(server.Requests.Last().Body)=="--witness\r\nContent-Type: text/plain\r\n\r\n\r\n--witness--\r\n","Zero-byte text part framing");

        server.Mode="barrier";var barrier=await client.SendBarrierAsync(new SendBarrierInput{Body=new __BARRIER__{Item1=new __BARRIER_PART1__{Value="only"}}},options);
        Check(barrier.Data.Item1.Value.Value=="one"&&typeof(__BARRIER__).GetProperty("Item2") is null&&typeof(__BARRIER__).GetProperty("Items") is null,"False prefix truncates native slots and closes the tail");
        server.Mode="barrier-extra";await Failure(()=>client.SendBarrierAsync(new SendBarrierInput{Body=new __BARRIER__()}),SdkErrorKind.ResponseDecode);
        server.Mode="empty";await client.SendEmptyAsync(new SendEmptyInput{Body=new __EMPTY__()},options);Check(Encoding.ASCII.GetString(server.Requests.Last().Body)=="--witness--\r\n","False first prefix permits only zero parts");

        server.Mode="repeat";
        var repeated=await client.SendRepeatedAsync(new SendRepeatedInput{Body=new __REPEATED__{Items=new List<__REPEATED_ITEM__>{new(){Value=new Payload{Amount=new JsonNumber("1.0")}},new(){Value=new Payload{Amount=new JsonNumber("2e0")}}}}},options);
        Check(repeated.Data.Items.Select(p=>p.Value.Amount.Token).SequenceEqual(new[]{"1.0","2e0"}),"Prefix-free typed repeating JSON items");
        count=server.Requests.Count;await Failure(()=>client.SendRepeatedAsync(new SendRepeatedInput{Body=new __REPEATED__()}),SdkErrorKind.RequestValidation);Check(server.Requests.Count==count,"minItems before transport");

        server.Mode="fallback";
        var fallback=await client.SendFallbackAsync(new SendFallbackInput{Body=new __FALLBACK__{Item1=new __FALLBACK_PART1__{Value="first"},Item2=new __FALLBACK_PART2__{Value=new JsonInteger("2.00"),Headers=new __FALLBACK_HEADER2__{XNumber=2}},Items=new List<__FALLBACK_ITEM__>{new(){Value=new JsonInteger("3e0")}}}},options);
        Check(fallback.Data.Item2.Value.Value.Token=="2.00"&&fallback.Data.Item2.Value.Headers.XNumber.Token=="2"&&fallback.Data.Items[0].Value.Token=="3e0","prefixEncoding beyond prefixItems uses actual items codec, then itemEncoding");
        count=server.Requests.Count;await Failure(()=>client.SendFallbackAsync(new SendFallbackInput{Body=new __FALLBACK__{Item2=new __FALLBACK_PART2__{Value=2,Headers=new __FALLBACK_HEADER2__{XNumber=2}}}}),SdkErrorKind.RequestRepresentation);Check(server.Requests.Count==count,"Optional prefix holes cannot renumber later positions");

        server.Mode="disposition";var dispositionValue="form-data; name=\"first\"; filename=\"a.txt\"";
        var disposition=await client.SendDispositionAsync(new SendDispositionInput{Body=new __DISPOSITION__{Item1=new __DISPOSITION_PART1__{Value="text",Headers=new __DISPOSITION_HEADER1__{ContentDisposition=dispositionValue}}}},new RequestOptions{ContentType="multipart/form-data; boundary=witness"});
        Check(disposition.Data.Item1.Headers.ContentDisposition==dispositionValue&&disposition.Data.Item1.FileName=="a.txt","Source-typed positional form-data disposition");
        Check(Encoding.UTF8.GetString(server.Requests.Last().Body).Contains("Content-Disposition: "+dispositionValue),"Disposition supplied by the source header is not synthesized or replaced");
        await client.SendDispositionAsync(new SendDispositionInput{Body=new __DISPOSITION__{Item1=new __DISPOSITION_PART1__{Value=disposition.Data.Item1.Value,FileName=disposition.Data.Item1.FileName,Headers=new __DISPOSITION_HEADER1__{ContentDisposition=disposition.Data.Item1.Headers.ContentDisposition}}}},new RequestOptions{ContentType="multipart/form-data; boundary=witness"});
        Check(Encoding.UTF8.GetString(server.Requests.Last().Body).Split("Content-Disposition:").Length==2,"Decoded matching filename metadata does not duplicate source disposition");

        foreach(var mode in new[]{"missing-header","invalid-payload","wrong-media","too-few","too-many","duplicate-header","incomplete"})
        {server.Mode=mode;var failure=await Failure(()=>client.SendSequenceAsync(new SendSequenceInput{Body=Value()}),SdkErrorKind.ResponseDecode);Check(failure.OperationSource.EndsWith("/paths/~1sequence/post"),"Located response boundary "+mode);}
        server.Mode="tiny";var tiny=await client.SendTinyAsync(new SendTinyInput{Body=new __TINY__{Item1=new __TINY_PART1__{Value=new byte[]{0,255,42}}}},options);Check(tiny.Data.Item1.Value.SequenceEqual(new byte[]{0,255,42}),"Positional byte policy exact boundary");
        count=server.Requests.Count;await Failure(()=>client.SendTinyAsync(new SendTinyInput{Body=new __TINY__{Item1=new __TINY_PART1__{Value=new byte[4]}}}),SdkErrorKind.ResourceLimit);Check(server.Requests.Count==count,"Positional source byte limit before send");
        server.Mode="too-large";await Failure(()=>client.SendTinyAsync(new SendTinyInput{Body=new __TINY__{Item1=new __TINY_PART1__{Value=new byte[0]}}}),SdkErrorKind.ResourceLimit);
        await Cleanup();

        // Independent transport witness for the documented source refusal.
        using(var direct=new HttpClient(new SocketsHttpHandler{AllowAutoRedirect=false,UseCookies=false,UseProxy=false}))
        {using var response=await direct.SendAsync(new HttpRequestMessage(new HttpMethod("hEaD"),server.Url+"/method-probe"));var data=await response.Content.ReadAsByteArrayAsync();var seen=server.Requests.Last().Method;Check(seen!="hEaD"||data.Length==0,"HttpClient cannot preserve both custom known-method bytes and body semantics");Console.WriteLine($"HttpClient case witness: source=hEaD, wire={seen}, bodyBytes={data.Length}");}

        using(var zip=ZipFile.OpenRead(args[0]))
        {
            string Read(string path){using var reader=new StreamReader(zip.GetEntry(path)!.Open());return reader.ReadToEnd();}
            var xml=XDocument.Parse(Read("lib/net8.0/Positional.Csharp.xml")).Descendants("member").Select(m=>(string)m.Attribute("name")!).ToHashSet();using var reference=JsonDocument.Parse(Read("docs/reference.json"));var html=Read("docs/index.html");
            foreach(var symbol in reference.RootElement.GetProperty("symbols").EnumerateArray()){Check(xml.Contains(symbol.GetProperty("xmlId").GetString()!),"Native XML member "+symbol.GetProperty("xmlId"));Check(html.Contains("id=\""+symbol.GetProperty("id").GetString()+"\""),"Browsable native symbol anchor");}
            Check(Read("README.md").Contains("Finite positional multipart"),"Positional native guide");
        }
        Console.WriteLine($"POSITIONAL CSHARP PASS: {checks} checks; {System.Runtime.InteropServices.RuntimeInformation.FrameworkDescription}; loopbackRequests={server.Requests.Count}");
    }
    private static async Task Cleanup()
    {
        var stream=new ControlledStream(MimeServer.Sequence("invalid-payload"),failClose:true);
        using(var http=new HttpClient(new Handler((_,_)=>Task.FromResult(Response(stream)))))using(var client=new Client(new Credentials(),httpClient:http))
        {var error=await Failure(()=>client.SendSequenceAsync(new SendSequenceInput{Body=Value()}),SdkErrorKind.ResponseDecode);Check(stream.Closed&&!error.ToString().Contains("private-close"),"Cleanup cannot replace payload codec failure");}
        stream=new ControlledStream(MimeServer.Sequence("normal"),failClose:true);
        using(var http=new HttpClient(new Handler((_,_)=>Task.FromResult(Response(stream)))))using(var client=new Client(new Credentials(),httpClient:http))
        {await Failure(()=>client.SendSequenceAsync(new SendSequenceInput{Body=Value()}),SdkErrorKind.Transport);Check(stream.Closed,"Standalone cleanup failure is typed");}
        stream=new ControlledStream(Array.Empty<byte>(),block:true,failClose:true);
        using(var http=new HttpClient(new Handler((_,_)=>Task.FromResult(Response(stream)))))using(var client=new Client(new Credentials(),httpClient:http))using(var cancel=new CancellationTokenSource())
        {var task=client.SendSequenceAsync(new SendSequenceInput{Body=Value()},cancellationToken:cancel.Token);await stream.Entered.Task.WaitAsync(TimeSpan.FromSeconds(2));cancel.Cancel();try{await task;throw new Exception("Expected cancellation");}catch(OperationCanceledException error){Check(error.CancellationToken==cancel.Token&&stream.Closed,"Caller cancellation and close failure");}}
        stream=new ControlledStream(MimeServer.Sequence("normal"),failClose:true);
        using(var http=new HttpClient(new Handler((_,_)=>Task.FromResult(Response(stream)))))using(var client=new Client(new Credentials(),new ClientOptions{MaxResponseBytes=4,MaxCaptureBytes=2},http))
        {var error=await Failure(()=>client.SendSequenceAsync(new SendSequenceInput{Body=Value()}),SdkErrorKind.ResourceLimit);Check(error.BodyCapture.Length<=2&&stream.Closed,"Bounded body capture before positional parse");}
    }
    private static HttpResponseMessage Response(Stream value){var result=new HttpResponseMessage(HttpStatusCode.OK){Content=new StreamContent(value)};result.Content.Headers.TryAddWithoutValidation("Content-Type","multipart/mixed; boundary=bound");return result;}
}
sealed class Handler(Func<HttpRequestMessage,CancellationToken,Task<HttpResponseMessage>> callback):HttpMessageHandler
{protected override Task<HttpResponseMessage> SendAsync(HttpRequestMessage request,CancellationToken token)=>callback(request,token);}
sealed class ControlledStream(byte[] bytes,bool block=false,bool failClose=false):Stream
{
    private int position;public bool Closed;public TaskCompletionSource Entered=new(TaskCreationOptions.RunContinuationsAsynchronously);
    public override bool CanRead=>true;public override bool CanSeek=>false;public override bool CanWrite=>false;public override long Length=>throw new NotSupportedException();public override long Position{get=>position;set=>throw new NotSupportedException();}
    public override int Read(byte[] buffer,int offset,int count)=>ReadAsync(buffer.AsMemory(offset,count)).AsTask().GetAwaiter().GetResult();
    public override async ValueTask<int> ReadAsync(Memory<byte> target,CancellationToken token=default){Entered.TrySetResult();if(block)await Task.Delay(Timeout.Infinite,token);var count=Math.Min(target.Length,bytes.Length-position);bytes.AsMemory(position,count).CopyTo(target);position+=count;return count;}
    protected override void Dispose(bool disposing){Closed=true;base.Dispose(disposing);if(failClose)throw new IOException("private-close");}
    public override void Flush(){}public override long Seek(long offset,SeekOrigin origin)=>throw new NotSupportedException();public override void SetLength(long length)=>throw new NotSupportedException();public override void Write(byte[] buffer,int offset,int count)=>throw new NotSupportedException();
}
sealed record Seen(string Method,string Target,Dictionary<string,string> Headers,byte[] Body);
sealed class MimeServer:IAsyncDisposable
{
    private readonly TcpListener listener=new(IPAddress.Loopback,0);private readonly CancellationTokenSource stop=new();private readonly Task worker;
    public readonly ConcurrentQueue<Seen> Requests=new();public string Mode="normal";public string Url{get;}
    public MimeServer(){listener.Start();Url="http://127.0.0.1:"+((IPEndPoint)listener.LocalEndpoint).Port;worker=Run();}
    internal static byte[] Sequence(string mode)
    {
        var first=mode=="wrong-media"?"Content-Type: application/octet-stream\r\n":"";
        if(mode!="missing-header")first+="X-Ordinal: "+(mode=="duplicate-header"?"1\r\nX-Ordinal: 2":"1.0")+"\r\n";
        var json=mode=="invalid-payload"?"{\"amount\":true}":"{\"amount\":9007199254740993.000000000000000001}";
        if(mode=="too-few")return Encoding.ASCII.GetBytes("--bound--\r\n");
        var body=Encoding.UTF8.GetBytes("bounded preamble\r\n--bound \t\r\n"+first+"\r\nfirst\r\n--bound\r\nContent-Type: application/json\r\nX-Trace: line\r\n one\r\n\r\n"+json+"\r\n--bound\r\nContent-Type: application/octet-stream\r\nX-Index: 2e0\r\n\r\n").Concat(Probe.Binary).ToArray();
        if(mode=="incomplete")return body;
        if(mode=="too-many")body=body.Concat(Encoding.ASCII.GetBytes("\r\n--bound\r\nContent-Type: application/octet-stream\r\nX-Index: 3\r\n\r\nx\r\n--bound\r\nContent-Type: application/octet-stream\r\nX-Index: 4\r\n\r\ny")).ToArray();
        return body.Concat(Encoding.ASCII.GetBytes("\r\n--bound--\t\r\nbounded epilogue")).ToArray();
    }
    private async Task Run()
    {
        try{while(!stop.IsCancellationRequested){using var connection=await listener.AcceptTcpClientAsync(stop.Token);using var stream=connection.GetStream();var header=new List<byte>();var one=new byte[1];while(header.Count<65_536){if(await stream.ReadAsync(one,stop.Token)==0)throw new Exception("Incomplete request");header.Add(one[0]);if(header.Count>=4&&header[^4]==13&&header[^3]==10&&header[^2]==13&&header[^1]==10)break;}
            var lines=Encoding.ASCII.GetString(header.ToArray()).Split("\r\n",StringSplitOptions.RemoveEmptyEntries);var start=lines[0].Split(' ');var headers=lines.Skip(1).Select(l=>l.Split(':',2)).ToDictionary(p=>p[0],p=>p[1].Trim(),StringComparer.OrdinalIgnoreCase);var body=new byte[headers.TryGetValue("Content-Length",out var n)?int.Parse(n):0];await stream.ReadExactlyAsync(body,stop.Token);Requests.Enqueue(new Seen(start[0],start[1],headers,body));
            var response=Reply(start[1]);var media=start[1]=="/disposition"?"multipart/form-data; boundary=bound":start[1]=="/method-probe"?"application/octet-stream":"multipart/mixed; boundary=bound";
            var prefix=Encoding.ASCII.GetBytes($"HTTP/1.1 200 OK\r\nContent-Type: {media}\r\nContent-Length: {response.Length}\r\nConnection: close\r\n\r\n");await stream.WriteAsync(prefix.Concat(response).ToArray(),stop.Token);
        }}catch(OperationCanceledException)when(stop.IsCancellationRequested){}catch(IOException)when(stop.IsCancellationRequested){}catch(SocketException)when(stop.IsCancellationRequested){}
    }
    private byte[] Reply(string path)
    {
        byte[] Text(string text)=>Encoding.UTF8.GetBytes(text);
        if(path=="/method-probe")return Text("actual-response-body");
        if(Mode=="empty")return Text("--bound--\r\n");
        if(path=="/optional")return Text("--bound\r\n\r\n\r\n--bound--\r\n");
        if(path=="/barrier")return Text("--bound\r\n\r\none"+(Mode=="barrier-extra"?"\r\n--bound\r\n\r\nsecond":"")+"\r\n--bound--\r\n");
        if(path=="/repeat")return Text("--bound\r\nContent-Type: application/json\r\n\r\n{\"amount\":1.0}\r\n--bound\r\nContent-Type: application/json\r\n\r\n{\"amount\":2e0}\r\n--bound--\r\n");
        if(path=="/fallback")return Text("--bound\r\n\r\nfirst\r\n--bound\r\nX-Number: 2\r\n\r\n2.00\r\n--bound\r\n\r\n3e0\r\n--bound--\r\n");
        if(path=="/disposition")return Text("--bound\r\nContent-Disposition: form-data; name=\"first\"; filename=\"a.txt\"\r\n\r\ntext\r\n--bound--\r\n");
        if(path=="/tiny")return Text("--bound\r\nContent-Type: application/octet-stream\r\n\r\n").Concat(Mode=="too-large"?new byte[]{0,255,42,43}:new byte[]{0,255,42}).Concat(Text("\r\n--bound--\r\n")).ToArray();
        return Sequence(Mode);
    }
    public async ValueTask DisposeAsync(){stop.Cancel();listener.Stop();await worker;stop.Dispose();}
}
