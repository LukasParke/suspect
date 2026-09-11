using OpenRouter;
using System.Reflection;
using System.Security;
using System.Text;
using System.Text.Json;
using System.Net;

internal static class Probe
{
    private static int checks;
    private static void Check(bool value,string message){if(!value)throw new Exception(message);checks++;}
    private static readonly string[] Variables={"CSHARP_ENV_BEARER","CSHARP_ENV_HEADER","CSHARP_ENV_QUERY","CSHARP_ENV_COOKIE","CSHARP_ENV_ALIAS"};
    private static void Set(params string?[] values){for(var i=0;i<Variables.Length;i++)Environment.SetEnvironmentVariable(Variables[i],i<values.Length?values[i]:null);}
    private static async Task Missing(Func<Task> action,Recorder recorder)
    {
        var before=recorder.Requests.Count;
        try{await action();throw new Exception("Expected missing credentials");}
        catch(SdkException error)
        {
            Check(error.Kind==SdkErrorKind.Authentication,"Authentication failure category");
            Check(error.OperationSource.StartsWith("https://source.csharp.test/credential-env.json#/paths/",StringComparison.Ordinal),"Physical operation finding");
            Check(error.Message.Length<128&&!error.ToString().Contains("test-alpha",StringComparison.Ordinal)&&!error.ToString().Contains("test-beta",StringComparison.Ordinal)&&!error.ToString().Contains("Authorization:",StringComparison.Ordinal),"Bounded secret-free failure");
        }
        Check(recorder.Requests.Count==before,"Missing credentials fail before HttpClient transport");
    }
    internal static async Task Main()
    {
        using var recorder=new Recorder();using var http=new HttpClient(recorder);
        Set("import-value","header-a","query a/b","cookie a/b","alias-a");
        _=typeof(Client).Assembly.GetName(); _=Client.Operations.Count;
        Set("test-alpha","header-a","query a/b","cookie a/b","alias-a");
        using var client=Client.FromEnvironment(httpClient:http);
        Set("test-beta","header-b","changed","changed","alias-b");
        await using(var result=await client.GetCurrentKeyAsync())Check(result.Status==204,"Environment-positive source operation");
        Check(recorder.Requests.Last().Authorization=="Bearer test-alpha","Creation-time snapshot, not import or per-request lookup");
        Check(recorder.Requests.Last().Url=="https://openrouter.ai/api/v1/key","Source-default HTTPS server");
        await client.HeaderOnlyAsync();Check(recorder.Requests.Last().Header=="header-a","Source header API-key attachment");
        await client.QueryOnlyAsync();Check(recorder.Requests.Last().Url.EndsWith("/query?api_key=query%20a%2Fb",StringComparison.Ordinal),"Source query API-key attachment");
        await client.CookieOnlyAsync();Check(recorder.Requests.Last().Cookie=="api_key=cookie%20a%2Fb","Source cookie API-key attachment");
        await client.AliasAuthAsync();Check(recorder.Requests.Last().Authorization=="Bearer alias-a","Aliases retain separate mapped declaration identities");
        using(var newer=Client.FromEnvironment(httpClient:http)){await newer.GetCreditsAsync();Check(recorder.Requests.Last().Authorization=="Bearer test-beta","New client observes environment change");}
        using(var explicitValue=new Client(new Credentials{__apiKey__="explicit-value"},httpClient:http)){await explicitValue.GetCurrentKeyAsync();Check(recorder.Requests.Last().Authorization=="Bearer explicit-value","Whole explicit argument wins");await Missing(()=>explicitValue.HeaderOnlyAsync(),recorder);}
        using(var explicitEmpty=new Client(new Credentials{__apiKey__=""},httpClient:http))await Missing(()=>explicitEmpty.GetCurrentKeyAsync(),recorder);
        using(var explicitNull=new Client(new Credentials{__apiKey__=null},httpClient:http))await Missing(()=>explicitNull.GetCurrentKeyAsync(),recorder);
        using(var explicitMissing=new Client(new Credentials(),httpClient:http))await Missing(()=>explicitMissing.GetCurrentKeyAsync(),recorder);
        using(var explicitPartial=new Client(new Credentials{__headerKey__="explicit-header"},httpClient:http))
        {await explicitPartial.AlternativeAuthAsync();Check(recorder.Requests.Last().Authorization is null&&recorder.Requests.Last().Header=="explicit-header","Missing explicit member is not filled from env");await Missing(()=>explicitPartial.ConjunctiveAuthAsync(),recorder);}
        var before=recorder.Requests.Count;
        try{using var invalid=new Client(null!,httpClient:http);throw new Exception("Expected explicit null argument refusal");}
        catch(SdkException error){Check(error.Kind==SdkErrorKind.RequestRepresentation&&error.Message.Length<128,"Explicit null preserves existing constructor refusal");}
        Check(recorder.Requests.Count==before,"Explicit null never performs transport or env fallback");
        Set();using(var absent=Client.FromEnvironment(httpClient:http))
        {await Missing(()=>absent.GetCurrentKeyAsync(),recorder);await absent.AnonymousAsync();Check(recorder.Requests.Last().Authorization is null,"Anonymous operation works without environment");await absent.OptionalAuthAsync();Check(recorder.Requests.Last().Authorization is null,"Anonymous OR alternative survives missing env");}
        Set("","","","","");using(var empty=Client.FromEnvironment(httpClient:http))
        {await Missing(()=>empty.GetCurrentKeyAsync(),recorder);await Missing(()=>empty.HeaderOnlyAsync(),recorder);await Missing(()=>empty.QueryOnlyAsync(),recorder);await Missing(()=>empty.CookieOnlyAsync(),recorder);}
        Set(null,"usable-header");using(var alternative=Client.FromEnvironment(httpClient:http))
        {await alternative.AlternativeAuthAsync();Check(recorder.Requests.Last().Header=="usable-header"&&recorder.Requests.Last().Authorization is null,"Missing first alternative does not block second");await Missing(()=>alternative.AlternativeAuthAsync(new AlternativeAuthInput(),new RequestOptions{SecurityAlternative=0}),recorder);await alternative.AlternativeAuthAsync(new AlternativeAuthInput(),new RequestOptions{SecurityAlternative=1});Check(recorder.Requests.Last().Header=="usable-header","Explicit alternative selection");await Missing(()=>alternative.ConjunctiveAuthAsync(),recorder);}
        Set("and-value","and-header");using(var conjunction=Client.FromEnvironment(httpClient:http))
        {await conjunction.ConjunctiveAuthAsync();Check(recorder.Requests.Last().Authorization=="Bearer and-value"&&recorder.Requests.Last().Header=="and-header","Conjunctive requirements attach together");await conjunction.OptionalAuthAsync(new OptionalAuthInput(),new RequestOptions{SecurityAlternative=1});Check(recorder.Requests.Last().Authorization is null,"Explicit anonymous alternative");}
        Set("not a token","fallback-header");using(var invalidEnvironment=Client.FromEnvironment(httpClient:http))
        {await invalidEnvironment.AlternativeAuthAsync();Check(recorder.Requests.Last().Header=="fallback-header"&&recorder.Requests.Last().Authorization is null,"Unusable environment bearer does not mask a valid alternative");}
        Set(new string('x',8193),"bad\r\nheader");using(var bounded=Client.FromEnvironment(httpClient:http))
        {await Missing(()=>bounded.GetCurrentKeyAsync(),recorder);await Missing(()=>bounded.HeaderOnlyAsync(),recorder);await bounded.AnonymousAsync();}
        Set(null,null,null,null,"alias-only");using(var aliases=Client.FromEnvironment(httpClient:http))
        {await aliases.AliasAuthAsync();Check(recorder.Requests.Last().Authorization=="Bearer alias-only","Mapped alias remains usable");await Missing(()=>aliases.GetCurrentKeyAsync(),recorder);}

        // Exercise the same native environment-reader boundary with unavailable
        // access; platform/security failures become missing values without text leakage.
        var runtime=typeof(Client).Assembly.GetType("OpenRouter.CredentialEnvironment")!;
        var read=runtime.GetMethods(BindingFlags.NonPublic|BindingFlags.Static).Single(m=>m.Name=="Read"&&m.GetParameters().Length==3);
        var validated=false;Action<string> validate=_=>validated=true;
        foreach(Func<string,string?> unavailable in new Func<string,string?>[]{_=>throw new SecurityException("test-alpha"),_=>throw new PlatformNotSupportedException("test-beta"),_=>null,_=>""})
        {Check(read.Invoke(null,new object[]{"CSHARP_ENV_BEARER",unavailable,validate}) is null&&!validated,"Unavailable native environment access stays missing and secret-free");}
        before=recorder.Requests.Count;using(var cancelled=new CancellationTokenSource())
        {cancelled.Cancel();try{await client.GetCurrentKeyAsync(cancelled.Token);throw new Exception("Expected cancellation");}catch(OperationCanceledException error){Check(error.CancellationToken==cancelled.Token&&recorder.Requests.Count==before,"Task caller cancellation is preserved");}}
        using(var ownedBoundary=Client.FromEnvironment(httpClient:http)){}Check(!recorder.Disposed,"Factory preserves caller-owned HttpClient/handler");
        await client.AnonymousAsync();Check(recorder.Requests.Last().Authorization is null,"Explicit anonymous operation never attaches ambient credentials");
        var method=typeof(Client).GetMethod("FromEnvironment",BindingFlags.Public|BindingFlags.Static)!;
        Check(method.ReturnType==typeof(Client)&&method.GetParameters().Select(p=>p.Name).SequenceEqual(new[]{"options","httpClient"}),"Installed factory names and argument order");
        Set();
        File.WriteAllText("env-results.json",JsonSerializer.Serialize(new{checks,requests=recorder.Requests.Count,urls=recorder.Requests.Select(r=>r.Url).ToArray(),framework=System.Runtime.InteropServices.RuntimeInformation.FrameworkDescription},new JsonSerializerOptions{WriteIndented=true}));
        Console.WriteLine($"ENVIRONMENT CSHARP PASS: {checks} checks; {recorder.Requests.Count} controlled requests; {System.Runtime.InteropServices.RuntimeInformation.FrameworkDescription}");
    }
}
internal sealed record Seen(string Url,string? Authorization,string? Header,string? Cookie);
internal sealed class Recorder:HttpMessageHandler
{
    internal readonly List<Seen> Requests=new();internal bool Disposed;
    protected override Task<HttpResponseMessage> SendAsync(HttpRequestMessage request,CancellationToken token)
    {
        token.ThrowIfCancellationRequested();
        if(request.RequestUri!.Scheme!="https"||request.RequestUri.Host!="openrouter.ai"||request.Method!=HttpMethod.Get)throw new Exception("Unexpected source target");
        string? Header(string name)=>request.Headers.TryGetValues(name,out var values)?values.Single():null;
        Requests.Add(new Seen(request.RequestUri.OriginalString,Header("Authorization"),Header("X-Api-Key"),Header("Cookie")));
        return Task.FromResult(new HttpResponseMessage(HttpStatusCode.NoContent));
    }
    protected override void Dispose(bool disposing){Disposed=true;base.Dispose(disposing);}
}
