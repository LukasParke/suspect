using Protocol.Csharp;
using System.Collections.Concurrent;
using System.Net;
using System.Net.Sockets;
using System.Text;
using System.Text.Json;
using System.Runtime.InteropServices;
using System.IO.Compression;
using System.Xml.Linq;

static class Probe
{
    private static int checks;
    private static void Check(bool value,string message){if(!value)throw new Exception(message);checks++;}
    private static async Task<SdkException> Failure(Func<Task> action,SdkErrorKind kind)
    {try{await action();throw new Exception("Expected "+kind);}catch(SdkException error){Check(error.Kind==kind,"Kind "+error.Kind+" expected "+kind);return error;}}
    public static async Task Main(string[] args)
    {
        await using var server=new Server();
        using var client=new Client(new Credentials(),new ClientOptions{ServerUrl=server.Url+"/api"});
        void CheckLast(int index,string location,string expected)
        {
            var request=server.Requests.Last();var suffix=location switch{"path"=>"/vector/"+index+"/"+expected,"query" or "querystring"=>"/vector/"+index+"?"+expected,_=>"/vector/"+index};
            Check(request.Target.EndsWith(suffix,StringComparison.Ordinal),"Normative target "+request.Target+" expected "+suffix);
            if(location=="header")Check(request.Headers.Values.Contains(expected),"Unencoded header");
            if(location=="cookie")Check(request.Headers["Cookie"]==expected,"Cookie style");
        }
        __VECTORS__
        Check(server.Requests.Count==33,"All shared parameter/querystring vectors");
        await client.MethodGetAsync();await client.MethodPutAsync();await client.MethodPostAsync();await client.MethodDeleteAsync();await client.MethodOptionsAsync();await client.MethodHeadAsync();await client.MethodPatchAsync();await client.MethodTraceAsync();await client.MethodQueryAsync();
        Check(server.Requests.TakeLast(9).Select(r=>r.Method).SequenceEqual(new[]{"GET","PUT","POST","DELETE","OPTIONS","HEAD","PATCH","TRACE","QUERY"}),"Standard method tokens");
        await client.CopyAsync();await client.PingAsync();
        Check(server.Requests.TakeLast(2).Select(r=>r.Method).SequenceEqual(new[]{"COPY","x-PING"}),"Custom method tokens are preserved");
        await client.GetUnnamedAsync();Check(server.Requests.Last().Target=="/api/unnamed","Unnamed operation presentation is source-derived");
        var count=server.Requests.Count;
        await Failure(()=>client.AuthorizeAsync(new AuthorizeInput(),new RequestOptions{SecurityAlternative=0}),SdkErrorKind.Authentication);
        Check(server.Requests.Count==count,"Missing AND credentials fail before send");
        using(var authenticated=new Client(new Credentials{Bearer="token",HeaderKey="h",QueryKey="q/x",CookieKey="c x"},new ClientOptions{ServerUrl=server.Url+"/api"}))
        {await authenticated.AuthorizeAsync(new AuthorizeInput(),new RequestOptions{SecurityAlternative=0});var request=server.Requests.Last();Check(request.Headers["Authorization"]=="Bearer token"&&request.Headers["X-Api-Key"]=="h"&&request.Headers["Cookie"]=="session=c%20x"&&request.Target.EndsWith("?api_key=q%2Fx"),"Conjunctive auth attachments");}
        using(var basic=new Client(new Credentials{Basic=new BasicCredential("user","pass")},new ClientOptions{ServerUrl=server.Url+"/api"}))
        {await basic.AuthorizeAsync(new AuthorizeInput(),new RequestOptions{SecurityAlternative=1});Check(server.Requests.Last().Headers["Authorization"]=="Basic dXNlcjpwYXNz","Basic bytes");}
        var oauthCalls=0;var oidcCalls=0;
        using(var hooks=new Client(new Credentials{
            Oauth=(context,token)=>{oauthCalls++;Check(context.PermissionKind=="scopes"&&context.Permissions.SequenceEqual(new[]{"read:items"})&&context.Flows[0].TokenUrl=="https://auth.example/token"&&context.SchemeSource.Contains("/securitySchemes/oauth"),"OAuth source metadata");return ValueTask.FromResult(new AuthorizationValue("DPoP","supplied"));},
            Oidc=(context,token)=>{oidcCalls++;Check(context.DiscoveryUrl=="https://auth.example/.well-known/openid-configuration"&&context.Permissions[0]=="openid","OIDC source metadata");return ValueTask.FromResult(new AuthorizationValue("Custom","supplied"));}
        },new ClientOptions{ServerUrl=server.Url+"/api"}))
        {await hooks.AuthorizeAsync(new AuthorizeInput(),new RequestOptions{SecurityAlternative=2});Check(server.Requests.Last().Headers["Authorization"]=="DPoP supplied","No inferred bearer type");await hooks.AuthorizeAsync(new AuthorizeInput(),new RequestOptions{SecurityAlternative=3});Check(server.Requests.Last().Headers["Authorization"]=="Custom supplied","OIDC explicit attachment");}
        Check(oauthCalls==1&&oidcCalls==1,"No implicit credential acquisition or retries");
        await client.AuthorizeAsync(new AuthorizeInput(),new RequestOptions{SecurityAlternative=4});Check(!server.Requests.Last().Headers.ContainsKey("Authorization"),"Explicit anonymous alternative");
        using(var relative=new Client(new Credentials(),new ClientOptions{ServerIndex=1,ServerVariables=new Dictionary<string,string>{{"base","v3"}},DocumentUrl=server.Url+"/specs/openapi.json"}))
        {Check((await relative.AnonymousAsync()).Data=="ok","Relative server call");Check(server.Requests.Last().Target=="/v3/anonymous","Relative variable resolution");await Failure(()=>relative.AnonymousAsync(new AnonymousInput(),new RequestOptions{ServerVariables=new Dictionary<string,string>{{"base","bad"}}}),SdkErrorKind.RequestRepresentation);}
        using(var noBase=new Client(new Credentials(),new ClientOptions{ServerIndex=1}))await Failure(()=>noBase.AnonymousAsync(),SdkErrorKind.RequestRepresentation);
        server.Mode="json";var selected=await client.SelectResponseAsync();
        Check(selected is SelectResponseResult.Status200{Data:SelectResponseStatus200Body.Json},"Exact JSON media arm");
        var json=(SelectResponseResult.Status200)selected;
        Check(json.Headers.XLimit.Token=="1.0"&&json.Headers.XTags.Value.SequenceEqual(new[]{"a","b"})&&json.Headers.XMeta.Value.Extra["n"].Token=="2.00","Typed response scalar/array/object headers");
        Check(json.Metadata.Links[0].Name=="next"&&json.Metadata.Links[0].Parameters["id"].GetString()=="$response.body#/value"&&json.Metadata.Links[0].RequestBody.Value.GetProperty("$ref").GetString()=="literal","Inert Link metadata");
        server.Mode="text";Check(await client.SelectResponseAsync() is SelectResponseResult.Status200{Data:SelectResponseStatus200Body.Text{Value:"plain"}},"Typed text media");
        server.Mode="application";Check(await client.SelectResponseAsync() is SelectResponseResult.Status200{Data:__APP_BYTES_CASE__},"Type wildcard media");
        server.Mode="any";Check(await client.SelectResponseAsync() is SelectResponseResult.Status200{Data:__ANY_BYTES_CASE__},"Any wildcard media");
        server.Mode="profile";Check(await client.SelectResponseAsync() is SelectResponseResult.Status200{Data:__PROFILE_JSON_CASE__},"Declared media parameter specificity");
        server.Mode="profile-case";Check(await client.SelectResponseAsync() is SelectResponseResult.Status200{Data:SelectResponseStatus200Body.Json},"Non-charset media parameter values are case sensitive");
        server.Mode="range";var range=await client.SelectResponseAsync();Check(range.Status==201&&range is SelectResponseResult.Status2xx{Data:SelectResponseStatus2xxBody.UndeclaredBytes{Value:var bytes}}&&bytes.SequenceEqual(new byte[]{0,255,42}),"Range actual status and unspecified bytes");
        server.Mode="empty";Check(await client.SelectResponseAsync() is SelectResponseResult.Status2xx{Data:SelectResponseStatus2xxBody.NoContent},"204 has a separate bodyless arm");
        server.Mode="default-success";Check((await client.SelectResponseAsync()).Status==299,"Range success uses actual status");
        server.Mode="default-error";var error=await Failure(()=>client.SelectResponseAsync(),SdkErrorKind.Api);Check(error is SelectResponseApiException.Default&&error.Response?.Status==404,"Default actual error");
        server.Mode="bad-header";await Failure(()=>client.SelectResponseAsync(),SdkErrorKind.ResponseDecode);
        server.Mode="missing-media";await Failure(()=>client.SelectResponseAsync(),SdkErrorKind.UnexpectedResponse);
        server.Mode="json";
        using(var document=JsonDocument.Parse("{\"value\":9007199254740993.000000000000000001}")){var free=await client.FreeJsonAsync(new FreeJsonInput{Body=document.RootElement.Clone()});Check(free.Data.GetProperty("value").GetRawText()=="9007199254740993.000000000000000001","Schema-free JSON remains exact JSON");}
        await Failure(()=>client.UnspecifiedAsync(),SdkErrorKind.UnexpectedResponse);
        var defaultSuccess=await client.DefaultResponseAsync();Check(defaultSuccess.Status==202,"A default match can succeed");
        server.Mode="default-error";Check(await Failure(()=>client.DefaultResponseAsync(),SdkErrorKind.Api) is DefaultResponseApiException.Default,"Default match failure uses actual status");server.Mode="json";
        var text=await client.PostTextAsync(new PostTextInput{Body=new JsonInteger("10e-0001")});Check(text.Data.Token=="1.00e+0"&&Encoding.UTF8.GetString(server.Requests.Last().Body)=="10e-0001","Exact mathematical text scalar");
        var binary=await client.PutBytesAsync(new PutBytesInput{Body=new byte[]{0,255,42}});Check(binary.Data.SequenceEqual(new byte[]{0,255,42})&&server.Requests.Last().Body.SequenceEqual(binary.Data),"Binary bytes never become JSON null");
        count=server.Requests.Count;await Failure(()=>client.PutBytesAsync(new PutBytesInput{Body=new byte[4]}),SdkErrorKind.ResourceLimit);Check(server.Requests.Count==count,"Byte policy before transport");
        await client.ChooseAsync(new ChooseInput{Body=new ChooseBody.Json(new Value{Value2="native"})});
        await client.ChooseAsync(new ChooseInput{Body=new ChooseBody.Bytes(new byte[]{0,255},"application/pdf")});Check(server.Requests.Last().Headers["Content-Type"]=="application/pdf"&&server.Requests.Last().Body.SequenceEqual(new byte[]{0,255}),"Explicit concrete wildcard request media");
        await Failure(()=>client.ChooseAsync(new ChooseInput{Body=new ChooseBody.Bytes(new byte[]{0},"application/json")}),SdkErrorKind.RequestRepresentation);
        Check((await client.HeadValueAsync()).Data==default(HttpNoContent),"HEAD ignores declared body schema");
        await client.PostFormAsync(__FORM_INPUT__);
        Check(Encoding.UTF8.GetString(server.Requests.Last().Body)=="codes=1&codes=2&metadata=%7B%22value%22%3A%22v%22%7D&name=a+%2B+b&tags=x&tags=y","OAS 3.2 per-item form style, JSON and repeated encodings: "+Encoding.UTF8.GetString(server.Requests.Last().Body));
        var form=await client.GetFormAsync();Check(form.Data.Name=="a + b"&&form.Data.Tags.Value.Select(v=>v.Token).SequenceEqual(new[]{"1.0","2"}),"Typed form response");
        var uploadInput=__UPLOAD_INPUT__;
        await client.UploadAsync(uploadInput);
        var uploaded=server.Requests.Last();Check(uploaded.Headers["Content-Type"].StartsWith("multipart/form-data; boundary="),"Multipart concrete boundary");
        var uploadText=Encoding.Latin1.GetString(uploaded.Body);Check(uploadText.Contains("name=\"file\"; filename=\"x.bin\"")&&uploadText.Contains("Content-Type: image/png")&&uploadText.Contains("X-Part-Id: part-1")&&uploaded.Body.AsSpan().IndexOf(new byte[]{0,255,42})>=0,"Multipart exact bytes, disposition and typed headers");
        Check(uploadText.Split("name=\"files\"").Length==3&&uploaded.Body.AsSpan().IndexOf(new byte[]{1,255})>=0&&uploaded.Body.AsSpan().IndexOf(new byte[]{2,255})>=0,"Repeated byte parts and order");
        await client.UploadAsync(uploadInput,new RequestOptions{ContentType="multipart/form-data; boundary=explicit-fixture"});Check(server.Requests.Last().Headers["Content-Type"].Contains("boundary=explicit-fixture")&&Encoding.ASCII.GetString(server.Requests.Last().Body).StartsWith("--explicit-fixture\r\n"),"Explicit multipart boundary parameter is preserved");
        count=server.Requests.Count;uploadInput.Body.File.Value=new byte[1024*1024+1];await Failure(()=>client.UploadAsync(uploadInput),SdkErrorKind.ResourceLimit);Check(server.Requests.Count==count,"Part ceiling before transport");
        uploadInput.Body.File.Value=new byte[]{0};uploadInput.Body.Files.Value.Add(uploadInput.Body.Files.Value[0]);await Failure(()=>client.UploadAsync(uploadInput),SdkErrorKind.RequestValidation);Check(server.Requests.Count==count,"Repeated part structural cardinality before transport");
        var downloaded=await client.DownloadPartsAsync();Check(downloaded.Data.File.Value.SequenceEqual(new byte[]{0,255,42})&&downloaded.Data.File.Headers.XPartId=="part-1","Typed binary multipart response");
        await using(var events=await client.EventsAsync())
        {var list=new List<Event>();await foreach(var item in events.Data)list.Add(item);Check(list.Count==3&&list[0].Data=="one\ntwo"&&list[0].Id.Value=="e1"&&list[0].Retry.Value.Token=="25"&&list[1].Data=="[DONE]"&&list[1].Id.Value=="e1"&&list[2].Data=="\ufffd","SSE envelope, persistent id, retry, sentinel and HTML UTF-8 replacement");}
        await using(var lines=await client.LinesAsync()){var items=new List<string>();await foreach(var item in lines.Data)items.Add(item.Token);Check(items.SequenceEqual(new[]{"9007199254740993.000000000000000001","1e999999999999"}),"JSON-lines exact item tokens");}
        await StreamLifetimes();
        using(var archive=ZipFile.OpenRead(args[0]))
        {
            string Read(string path){using var reader=new StreamReader(archive.GetEntry(path)!.Open());return reader.ReadToEnd();}
            var xml=XDocument.Parse(Read("lib/net8.0/Protocol.Csharp.xml")).Descendants("member").Select(v=>(string)v.Attribute("name")!).ToHashSet();
            using var reference=JsonDocument.Parse(Read("docs/reference.json"));var html=Read("docs/index.html");
            foreach(var symbol in reference.RootElement.GetProperty("symbols").EnumerateArray()){Check(xml.Contains(symbol.GetProperty("xmlId").GetString()!),"Actual native XML symbol "+symbol.GetProperty("xmlId"));Check(html.Contains("id=\""+symbol.GetProperty("id").GetString()+"\""),"Browsable native reference anchor");}
            Check(archive.GetEntry("examples/native-fixtures.json") is not null,"Explicit byte fixture provenance");
        }
        Console.WriteLine($"EXPANDED CSHARP PASS: {checks} checks; {RuntimeInformation.FrameworkDescription}; actual loopback requests {server.Requests.Count}");
    }
    private static async Task StreamLifetimes()
    {
        var stream=new ControlledStream(Encoding.UTF8.GetBytes("data: first\n\ndata: second\n\n"));
        using(var http=new HttpClient(new Handler((_,_)=>Task.FromResult(Response(stream,"text/event-stream")))))
        using(var client=new Client(new Credentials(),httpClient:http))
        {var result=await client.EventsAsync();await foreach(var item in result.Data){Check(item.Data=="first","Early stream item");break;}Check(stream.Closed,"Early-break iterator disposes response");}
        stream=new ControlledStream(Array.Empty<byte>(),block:true,failClose:true);
        using(var http=new HttpClient(new Handler((_,_)=>Task.FromResult(Response(stream,"text/event-stream")))))
        using(var client=new Client(new Credentials(),httpClient:http))
        using(var cts=new CancellationTokenSource())
        {var result=await client.EventsAsync();var iterator=result.Data.GetAsyncEnumerator(cts.Token);var move=iterator.MoveNextAsync().AsTask();await stream.Entered.Task.WaitAsync(TimeSpan.FromSeconds(2));cts.Cancel();try{await move;throw new Exception("Expected cancellation");}catch(OperationCanceledException){Check(stream.Closed,"Cancellation survives failing close");}}
        stream=new ControlledStream(Encoding.UTF8.GetBytes("data: orphan\n"));
        using(var http=new HttpClient(new Handler((_,_)=>Task.FromResult(Response(stream,"text/event-stream")))))
        using(var client=new Client(new Credentials(),httpClient:http))
        {await using var result=await client.EventsAsync();var count=0;await foreach(var _ in result.Data)count++;Check(count==0,"EOF does not dispatch unterminated SSE");}
        stream=new ControlledStream(Encoding.UTF8.GetBytes("true\n"));
        using(var http=new HttpClient(new Handler((_,_)=>Task.FromResult(Response(stream,"application/x-ndjson")))))
        using(var client=new Client(new Credentials(),httpClient:http))
        {await using var result=await client.LinesAsync();await Failure(async()=>{await foreach(var _ in result.Data){}},SdkErrorKind.ResponseDecode);Check(stream.Closed,"Item codec error closes stream");}
        stream=new ControlledStream(Encoding.UTF8.GetBytes("data: x\n\n"));
        using(var http=new HttpClient(new Handler((_,_)=>Task.FromResult(Response(stream,"text/event-stream")))))
        using(var client=new Client(new Credentials(),new ClientOptions{MaxResponseBytes=4,MaxCaptureBytes=2},http))
        {await using var result=await client.EventsAsync();var error=await Failure(async()=>{await foreach(var _ in result.Data){}},SdkErrorKind.ResourceLimit);Check(error.BodyCapture.Length<=2&&stream.Closed,"Total stream ceiling and capture");}
        stream=new ControlledStream(Array.Empty<byte>(),block:true);
        using(var http=new HttpClient(new Handler((_,_)=>Task.FromResult(Response(stream,"text/event-stream")))))
        using(var client=new Client(new Credentials(),new ClientOptions{Timeout=TimeSpan.FromMilliseconds(30)},http))
        {await using var result=await client.EventsAsync();await Failure(async()=>{await foreach(var _ in result.Data){}},SdkErrorKind.Timeout);Check(stream.Closed,"Whole-stream deadline");}
    }
    private static HttpResponseMessage Response(Stream stream,string media){var result=new HttpResponseMessage(HttpStatusCode.OK){Content=new StreamContent(stream)};result.Content.Headers.TryAddWithoutValidation("Content-Type",media);return result;}
}
sealed class Handler(Func<HttpRequestMessage,CancellationToken,Task<HttpResponseMessage>> callback):HttpMessageHandler
{protected override Task<HttpResponseMessage> SendAsync(HttpRequestMessage request,CancellationToken token)=>callback(request,token);}
sealed class ControlledStream(byte[] bytes,bool block=false,bool failClose=false):Stream
{
    private int position;public bool Closed;public TaskCompletionSource Entered=new(TaskCreationOptions.RunContinuationsAsynchronously);
    public override bool CanRead=>true;public override bool CanSeek=>false;public override bool CanWrite=>false;public override long Length=>throw new NotSupportedException();public override long Position{get=>position;set=>throw new NotSupportedException();}
    public override int Read(byte[] buffer,int offset,int count)=>ReadAsync(buffer.AsMemory(offset,count)).AsTask().GetAwaiter().GetResult();
    public override async ValueTask<int> ReadAsync(Memory<byte> target,CancellationToken token=default){Entered.TrySetResult();if(block)await Task.Delay(Timeout.Infinite,token);var count=Math.Min(1,Math.Min(target.Length,bytes.Length-position));bytes.AsMemory(position,count).CopyTo(target);position+=count;return count;}
    protected override void Dispose(bool disposing){Closed=true;base.Dispose(disposing);if(failClose)throw new IOException("private-close");}
    public override void Flush(){}public override long Seek(long offset,SeekOrigin origin)=>throw new NotSupportedException();public override void SetLength(long length)=>throw new NotSupportedException();public override void Write(byte[] data,int offset,int count)=>throw new NotSupportedException();
}
sealed record Seen(string Method,string Target,Dictionary<string,string> Headers,byte[] Body);
sealed class Server:IAsyncDisposable
{
    private readonly TcpListener listener=new(IPAddress.Loopback,0);private readonly CancellationTokenSource stop=new();private readonly Task worker;
    public readonly ConcurrentQueue<Seen> Requests=new();public string Mode="json";public string Url{get;}
    public Server(){listener.Start();Url="http://127.0.0.1:"+((IPEndPoint)listener.LocalEndpoint).Port;worker=Run();}
    private async Task Run()
    {
        try{while(!stop.IsCancellationRequested){using var connection=await listener.AcceptTcpClientAsync(stop.Token);using var stream=connection.GetStream();var header=new List<byte>();var one=new byte[1];while(header.Count<65_536){if(await stream.ReadAsync(one,stop.Token)==0)throw new Exception("Incomplete request");header.Add(one[0]);if(header.Count>=4&&header[^4]==13&&header[^3]==10&&header[^2]==13&&header[^1]==10)break;}
            var lines=Encoding.ASCII.GetString(header.ToArray()).Split("\r\n",StringSplitOptions.RemoveEmptyEntries);var first=lines[0].Split(' ');var headers=lines.Skip(1).Select(l=>l.Split(':',2)).ToDictionary(p=>p[0],p=>p[1].Trim(),StringComparer.OrdinalIgnoreCase);var body=new byte[headers.TryGetValue("Content-Length",out var n)?int.Parse(n):0];await stream.ReadExactlyAsync(body,stop.Token);var request=new Seen(first[0],first[1],headers,body);Requests.Enqueue(request);
            var(status,media,data,extra)=Reply(request);var response=$"HTTP/1.1 {status} Response\r\nContent-Length: {data.Length}\r\nConnection: close\r\n"+(media is null?"":"Content-Type: "+media+"\r\n")+extra+"\r\n";await stream.WriteAsync(Encoding.ASCII.GetBytes(response),stop.Token);if(request.Method!="HEAD")foreach(var b in data)await stream.WriteAsync(new[]{b},stop.Token);
        }}catch(OperationCanceledException)when(stop.IsCancellationRequested){}catch(IOException)when(stop.IsCancellationRequested){}catch(SocketException)when(stop.IsCancellationRequested){}
    }
    private (int,string?,byte[],string) Reply(Seen request)
    {
        var path=request.Target.Split('?')[0];byte[] Text(string v)=>Encoding.UTF8.GetBytes(v);
        if(path.Contains("/vector/")||path.Contains("/method/")||path.EndsWith("/choose")||path.EndsWith("/form")||path.EndsWith("/multipart")||path.EndsWith("/unnamed")||path=="/api/custom"&&request.Method!="head")return(204,null,Array.Empty<byte>(),"");
        if(path.EndsWith("/custom"))return(200,"application/json",Text("\"lower-head-body\""),"");
        if(path.EndsWith("/anonymous")||path.EndsWith("/auth"))return(200,"application/json",Text("\"ok\""),"");
        if(path.EndsWith("/default"))return(Mode=="default-error"?404:202,"application/json",Text("\"default\""),"");
        if(path.EndsWith("/free-json"))return(200,"application/json",request.Body,"");
        if(path.EndsWith("/unspecified"))return(200,"application/octet-stream",new byte[]{0,255,42},"");
        if(path.EndsWith("/text"))return(200,"text/plain; charset=UTF-8",Text("1.00e+0"),"");
        if(path.EndsWith("/bytes"))return(200,"application/octet-stream",new byte[]{0,255,42},"");
        if(path.EndsWith("/head"))return(200,null,Array.Empty<byte>(),"");
        if(path.EndsWith("/form-output"))return(200,"application/x-www-form-urlencoded",Text("name=a+%2B+b&tags=1.0&tags=2"),"");
        if(path.EndsWith("/multipart-output"))return(200,"multipart/form-data; boundary=witness",Text("--witness\r\nContent-Disposition: form-data; name=\"file\"; filename=\"x.bin\"\r\nContent-Type: image/png\r\nX-Part-Id: part-1\r\n\r\n").Concat(new byte[]{0,255,42}).Concat(Text("\r\n--witness\r\nContent-Disposition: form-data; name=\"metadata\"\r\nContent-Type: application/json\r\n\r\n{\"title\":\"native\"}\r\n--witness--\r\n")).ToArray(),"");
        if(path.EndsWith("/events"))return(200,"text/event-stream",new byte[]{239,187,191}.Concat(Text(":comment\r\nid: e1\revent: update\rretry: 00025\rdata: one\rdata: two\r\rdata: [DONE]\n\ndata: ")).Concat(new byte[]{255}).Concat(Text("\n\n")).ToArray(),"");
        if(path.EndsWith("/lines"))return(200,"application/x-ndjson",Text("9007199254740993.000000000000000001\r\n1e999999999999"),"");
        if(path.EndsWith("/response"))
        {
            var extra=Mode=="bad-header"?"X-Limit: false\r\n":"X-Limit: 1.0\r\nX-Tags: a, b\r\nX-Meta: n=2.00\r\n";
            return Mode switch{"text"=>(200,"text/plain",Text("plain"),extra),"profile"=>(200,"application/json;profile=rich",Text("{\"value\":\"x\"}"),extra),"profile-case"=>(200,"application/json;profile=RICH",Text("{\"value\":\"x\"}"),extra),"application"=>(200,"application/pdf",new byte[]{0,255,42},extra),"any"=>(200,"image/png",new byte[]{0,255,42},extra),"range"=>(201,null,new byte[]{0,255,42},""),"empty"=>(204,null,Array.Empty<byte>(),""),"default-success"=>(299,null,new byte[]{0},""),"default-error"=>(404,null,new byte[]{0,255,42},""),"missing-media"=>(200,null,Text("{\"value\":\"x\"}"),extra),_=>(200,"Application/JSON; charset=UTF-8",Text("{\"value\":\"x\"}"),extra)};
        }
        throw new Exception("No independent response for "+request.Target);
    }
    public async ValueTask DisposeAsync(){stop.Cancel();listener.Stop();await worker;stop.Dispose();}
}
