using Documents.Csharp;
using System.Collections.Concurrent;
using System.Net;
using System.Net.Sockets;
using System.Text;
using System.Text.Json;
using System.IO.Compression;
using System.Xml.Linq;

internal static class Probe
{
    private static int checks;
    private static void Check(bool value,string message){if(!value)throw new Exception(message);checks++;}
    internal static async Task Main(string[] args)
    {
        await using var server=new Server();
        using var handler=new SocketsHttpHandler { AllowAutoRedirect=false,UseCookies=false,UseProxy=false,
            ConnectCallback=async (_,token)=> {var socket=new Socket(SocketType.Stream,ProtocolType.Tcp);try {await socket.ConnectAsync(IPAddress.Loopback,server.Port,token);return new NetworkStream(socket,ownsSocket:true);}catch {socket.Dispose();throw;}} };
        using var http=new HttpClient(handler);
        var authBase="";var authCalls=0;
        AuthorizationProvider hook=(context,token)=>
        {
            token.ThrowIfCancellationRequested();authCalls++;
            Check(context.ServerUrl==authBase&&context.UrlBase==ApiUrlBase.EffectiveServer,"Hook receives the actual effective server");
            Check(context.SchemeSource.StartsWith("http://storage.example/releases/specs/parts.json#",StringComparison.Ordinal),"Scheme physical ownership");
            Check(context.SchemeResource!.BaseUri=="https://logical.example/catalog/parts.json","Logical scheme context remains separate");
            if(context.SchemeName=="OAuth") {Check(context.MetadataUrl=="../oauth/metadata","OAuth raw metadata URL retained");Check(context.Flows.Single().TokenUrl=="./token"&&context.Flows.Single().UrlBase==ApiUrlBase.EffectiveServer,"OAuth flow base classification");}
            else Check(context.DiscoveryUrl=="../openid/config","OIDC raw discovery URL retained");
            return ValueTask.FromResult(new AuthorizationValue("Bearer","source-token"));
        };
        var credentials=new Credentials { __OAuth__=hook,__Oidc__=hook };
        using var client=new Client(credentials,httpClient:http);
        async Task Send<T>(Func<Task<T>> action,string host,string target) where T:IAsyncDisposable
        {
            var before=server.Requests.Count;await using var result=await action();var observed=server.Requests.Last();
            Check(server.Requests.Count==before+1,"Exactly one native request");
            Check(observed.Method=="GET"&&observed.Host==host&&observed.Target==target,$"Physical server/encoded request mismatch: {observed}, expected {host}{target}");
        }
        await Send(()=>client.DefaultServerAsync(),"entry.example","/default");
        await Send(()=>client.EmptyServerAsync(),"storage.example","/empty");
        await Send(()=>client.DeclaredServerAsync(),"storage.example","/releases/Api%2Fv1/%2e%2E/North/declared");
        await Send(()=>client.DeclaredServerAsync(new DeclaredServerInput(),new RequestOptions {ServerVariables=new Dictionary<string,string>{{"tenant","South"}}}),"storage.example","/releases/Api%2Fv1/%2e%2E/South/declared");
        await Send(()=>client.DeclaredServerAsync(new DeclaredServerInput(),new RequestOptions {DocumentUrl="http://override.example/ui/spec/root.json?ticket=1#unused"}),"override.example","/ui/Api%2Fv1/%2e%2E/North/declared");
        await Send(()=>client.VariableServerAsync(),"absolute.example","/v1/variable");
        await Send(()=>client.VariableServerAsync(new VariableServerInput(),new RequestOptions {DocumentUrl="invalid ignored base"}),"absolute.example","/v1/variable");
        await Send(()=>client.VariableServerAsync(new VariableServerInput(),new RequestOptions {ServerVariables=new Dictionary<string,string>{{"base","../relative/%2E%2e/Case%2f"}}}),"storage.example","/releases/relative/%2E%2e/Case%2f/variable");
        await Send(()=>client.AbsoluteServerAsync(),"fixed.example","/Keep%2f/%2e%2e/Case/absolute");
        await Send(()=>client.AbsoluteServerAsync(new AbsoluteServerInput(),new RequestOptions {DocumentUrl="file:///does-not-rebase-absolute.json"}),"fixed.example","/Keep%2f/%2e%2e/Case/absolute");
        await Send(()=>client.NetworkServerAsync(),"network.example","/Top//Case/network");
        await Send(()=>client.DeclaredServerAsync(new DeclaredServerInput(),new RequestOptions {ServerUrl="http://direct.example/a/../Keep%2F/%2e%2e"}),"direct.example","/Keep%2F/%2e%2e/declared");
        using(var overridden=new Client(credentials,new ClientOptions {DocumentUrl="http://client.example/one/two/root.json"},http))
        {
            await Send(()=>overridden.DeclaredServerAsync(),"client.example","/one/Api%2Fv1/%2e%2E/North/declared");
            await Send(()=>overridden.DeclaredServerAsync(new DeclaredServerInput(),new RequestOptions {DocumentUrl="http://request.example/new/root.json"}),"request.example","/Api%2Fv1/%2e%2E/North/declared");
        }
        async Task Reject(Func<Task> action,string operation)
        {
            var before=server.Requests.Count;
            try{await action();throw new Exception("Invalid server accepted");}
            catch(SdkException error){Check(error.Kind==SdkErrorKind.RequestRepresentation&&error.OperationSource=="http://entry.example/root/spec/api.json#/paths/~1"+operation,"Physical mount finding: "+error.OperationSource);Check(server.Requests.Count==before,"Invalid resolution fails before native transport");}
        }
        await Reject(()=>client.LocalServerAsync(),"local");
        await Send(()=>client.LocalServerAsync(new LocalServerInput(),new RequestOptions {DocumentUrl="http://local-override.example/docs/openapi.json"}),"local-override.example","/Local%2Fapi/local");
        await Reject(()=>client.DeclaredServerAsync(new DeclaredServerInput(),new RequestOptions {ServerVariables=new Dictionary<string,string>{{"tenant","not-enum"}}}),"declared");
        foreach(var invalid in new[]{"http:/missing-host","http://user:pass@example.test/api","http://example.test/bad%2","http://example.test/path?query","../raw\\path","../bad path","../{base}","../[bad]"})
            await Reject(()=>client.VariableServerAsync(new VariableServerInput(),new RequestOptions {ServerVariables=new Dictionary<string,string>{{"base",invalid}}}),"variable");
        authBase="http://storage.example/releases/auth-api";await Send(()=>client.OauthServerAsync(),"storage.example","/releases/auth-api/oauth");
        Check(server.Requests.Last().Authorization=="Bearer source-token","OAuth attachment observed on native wire");
        authBase="http://override.example/Api%2Fauth";await Send(()=>client.OidcServerAsync(new OidcServerInput(),new RequestOptions {ServerUrl=authBase}),"override.example","/Api%2Fauth/oidc");
        Check(authCalls==2,"Metadata URLs caused no acquisition calls");

        var declared=Client.Operations["DeclaredServerAsync"];var metadata=declared.Servers[0];
        Check(declared.Source=="http://entry.example/root/spec/api.json#/paths/~1declared","Use-site physical mount source");
        Check(declared.TerminalSource=="http://storage.example/releases/specs/parts.json#/components/pathItems/Declared/get","Redirected terminal physical source");
        Check(declared.UseSiteResource!.BaseUri=="https://logical.example/catalog/api.json"&&declared.TerminalResource!.CanonicalUri=="https://logical.example/catalog/parts.json#definition","Logical metadata is read directly");
        Check(declared.UseSiteResource.Aliases.Contains("http://requested.example/bootstrap/api.json"),"Requested redirect alias is metadata");
        Check(declared.References.Count==declared.ReferenceResources.Count,"Reference-hop contexts stay aligned");
        Check(metadata.DocumentUrl=="http://storage.example/releases/specs/parts.json"&&metadata.DocumentSource==metadata.DocumentUrl+"#","Explicit physical server document root");
        Check(metadata.Resource!.BaseSource==metadata.DocumentUrl+"#/$self"&&metadata.Resource.BaseUri==declared.TerminalResource!.BaseUri,"Logical declaration keeps physical source ownership");
        Check(metadata.UrlBase==ApiUrlBase.ServerDocument&&metadata.ResolveUrl()=="http://storage.example/releases/Api%2Fv1/%2e%2E/North","Metadata resolution equals native request base");
        Check(Client.Operations["DefaultServerAsync"].Servers[0].DocumentUrl=="http://entry.example/root/spec/api.json","Implicit default belongs to entry document");
        Check(Client.Operations["EmptyServerAsync"].Servers[0].DocumentUrl==metadata.DocumentUrl,"Explicit empty belongs to overriding declaration");
        Check(Client.Operations["LocalServerAsync"].Servers[0].DocumentUrl=="file:///recorded/local/source.json","Local source is never an invented network base");
        var variable=Client.Operations["VariableServerAsync"].Servers[0];
        foreach(var (reference,expected) in new[]{(".","https://base.example/a/b/"),("..","https://base.example/a/"),("../../..","https://base.example/"),("./x/../Y","https://base.example/a/b/Y"),("%2e%2e/%2F","https://base.example/a/b/%2e%2e/%2F"),("/A//./B/../C","https://base.example/A//C"),("//other.example/%2e/X%2f","https://other.example/%2e/X%2f")})
            Check(variable.ResolveUrl(new Dictionary<string,string>{{"base",reference}},"https://base.example/a/b/spec.json?version=1")==expected,"Independent RFC3986 literal-dot/encoded-data resolution");
        using(var zip=ZipFile.OpenRead(args[0]))
        {
            string Read(string path){using var reader=new StreamReader(zip.GetEntry(path)!.Open());return reader.ReadToEnd();}
            var xml=XDocument.Parse(Read("lib/net8.0/Documents.Csharp.xml")).Descendants("member").Select(m=>(string)m.Attribute("name")!).ToHashSet();
            foreach(var name in new[]{"P:Documents.Csharp.ServerInfo.DocumentUrl","P:Documents.Csharp.ServerInfo.DocumentSource","P:Documents.Csharp.ResourceInfo.CanonicalUri","P:Documents.Csharp.CredentialContext.ServerUrl","P:Documents.Csharp.CredentialContext.UrlBase"})Check(xml.Contains(name),"Installed physical/logical/base XML docs");
            Check(Read("README.md").Contains("Physical document server bases",StringComparison.Ordinal),"Native server resolution guide");
        }
        File.WriteAllText("document-server-results.json",JsonSerializer.Serialize(new{checks,requests=server.Requests.ToArray(),framework=System.Runtime.InteropServices.RuntimeInformation.FrameworkDescription},new JsonSerializerOptions{WriteIndented=true}));
        Console.WriteLine($"PHYSICAL SERVER PASS: {checks} checks; {server.Requests.Count} loopback requests; {System.Runtime.InteropServices.RuntimeInformation.FrameworkDescription}");
    }
}
internal sealed record Seen(string Method,string Target,string Host,string? Authorization);
internal sealed class Server:IAsyncDisposable
{
    private readonly TcpListener listener=new(IPAddress.Loopback,0);
    private readonly CancellationTokenSource stop=new();private readonly Task worker;
    internal readonly ConcurrentQueue<Seen> Requests=new();internal int Port {get;}
    internal Server(){listener.Start();Port=((IPEndPoint)listener.LocalEndpoint).Port;worker=Run();}
    private async Task Run()
    {
        try {while(!stop.IsCancellationRequested)
        {
            using var socket=await listener.AcceptTcpClientAsync(stop.Token);using var stream=socket.GetStream();var bytes=new List<byte>();var one=new byte[1];
            while(bytes.Count<65_536) {if(await stream.ReadAsync(one,stop.Token)==0)throw new Exception("Incomplete request");bytes.Add(one[0]);if(bytes.Count>=4&&bytes[^4]==13&&bytes[^3]==10&&bytes[^2]==13&&bytes[^1]==10)break;}
            var lines=Encoding.ASCII.GetString(bytes.ToArray()).Split("\r\n",StringSplitOptions.RemoveEmptyEntries);var start=lines[0].Split(' ');
            var headers=lines.Skip(1).Select(line=>line.Split(':',2)).ToDictionary(p=>p[0],p=>p[1].Trim(),StringComparer.OrdinalIgnoreCase);
            Requests.Enqueue(new Seen(start[0],start[1],headers["Host"],headers.GetValueOrDefault("Authorization")));
            await stream.WriteAsync("HTTP/1.1 204 No Content\r\nConnection: close\r\n\r\n"u8.ToArray(),stop.Token);
        }}catch(OperationCanceledException)when(stop.IsCancellationRequested){}catch(SocketException)when(stop.IsCancellationRequested){}
    }
    public async ValueTask DisposeAsync(){stop.Cancel();listener.Stop();await worker;stop.Dispose();}
}
