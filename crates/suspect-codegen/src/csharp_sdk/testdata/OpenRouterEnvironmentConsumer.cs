using OpenRouter;
using System.Net;
using System.Text;
using System.Text.Json;

internal static class Program
{
    internal static async Task Main()
    {
        using var examples=JsonDocument.Parse(File.ReadAllBytes("responses.json"));
        using var handler=new SourceHandler(examples.RootElement.Clone());using var http=new HttpClient(handler);
        Environment.SetEnvironmentVariable("OPENROUTER_API_KEY","controlled-initial-key");
        using var client=Client.FromEnvironment(new ClientOptions{Timeout=TimeSpan.FromSeconds(10)},http);
        Environment.SetEnvironmentVariable("OPENROUTER_API_KEY","controlled-new-key");
        var checks=0;
        void Check(bool condition,string message){if(!condition)throw new Exception(message);checks++;}
        await using(var response=await client.GetCurrentKeyAsync())
        {
            Check(response.Status==200,"Actual getCurrentKey status");
            Check(response.Data.Data.Usage.Token=="25.5"&&!response.Data.Data.IsFreeTier&&!response.Data.Data.IsManagementKey,"Actual source-declared current-key model");
            Check(handler.LastUrl=="https://openrouter.ai/api/v1/key"&&handler.LastAuthorization=="Bearer controlled-initial-key","Actual source server and creation-time environment snapshot");
        }
        await using(var response=await client.GetCreditsAsync())
        {Check(response.Status==200&&response.Data.Data.TotalCredits.Token=="100.5"&&response.Data.Data.TotalUsage.Token=="25.75","Actual management-operation credit model");Check(handler.LastUrl=="https://openrouter.ai/api/v1/credits","Management operation remains an explicit separate call");}
        using(var second=Client.FromEnvironment(httpClient:http))
        {await second.GetCurrentKeyAsync();Check(handler.LastAuthorization=="Bearer controlled-new-key","New real-source client reads changed env");}
        handler.Unauthorized=true;
        try{await client.GetCurrentKeyAsync();throw new Exception("Expected source 401");}
        catch(GetCurrentKeyApiException.Status401 error){await using(error){Check(error.Response!.Status==401,"Actual typed source authentication response");}}
        var before=handler.Requests;
        Environment.SetEnvironmentVariable("OPENROUTER_API_KEY",null);
        using(var missing=Client.FromEnvironment(httpClient:http))
        {try{await missing.GetCurrentKeyAsync();throw new Exception("Expected missing environment credentials");}catch(SdkException error){Check(error.Kind==SdkErrorKind.Authentication&&handler.Requests==before,"Missing real-source credentials fail before HTTP");}}
        using(var explicitClient=new Client(new Credentials{ApiKey="explicit-controlled-key"},httpClient:http))
        {handler.Unauthorized=false;await explicitClient.GetCurrentKeyAsync();Check(handler.LastAuthorization=="Bearer explicit-controlled-key","Existing explicit credentials constructor remains usable");}
        Check(!handler.Disposed,"Injected HttpClient remains caller-owned");
        File.WriteAllText("openrouter-env-results.json",JsonSerializer.Serialize(new{checks,requests=handler.Requests,sourceServer="https://openrouter.ai/api/v1",operations=new[]{"getCurrentKey","getCredits"},framework=System.Runtime.InteropServices.RuntimeInformation.FrameworkDescription,mode="controlled-handler-no-live-account-call"},new JsonSerializerOptions{WriteIndented=true}));
        Console.WriteLine($"OPENROUTER ENVIRONMENT PASS: {checks} checks; {handler.Requests} controlled requests; {System.Runtime.InteropServices.RuntimeInformation.FrameworkDescription}");
    }
}
internal sealed class SourceHandler(JsonElement examples):HttpMessageHandler
{
    internal bool Unauthorized;internal int Requests;internal string? LastUrl;internal string? LastAuthorization;internal bool Disposed;
    protected override Task<HttpResponseMessage> SendAsync(HttpRequestMessage request,CancellationToken token)
    {
        token.ThrowIfCancellationRequested();
        if(request.Method!=HttpMethod.Get||request.RequestUri!.Scheme!="https"||request.RequestUri.Host!="openrouter.ai")throw new Exception("Unexpected actual-source request");
        Requests++;LastUrl=request.RequestUri.OriginalString;LastAuthorization=request.Headers.Authorization?.ToString();
        var name=request.RequestUri.AbsolutePath.EndsWith("/credits",StringComparison.Ordinal)?"getCredits":"getCurrentKey";
        var code=Unauthorized?"401":"200";
        var body=examples.GetProperty(name).GetProperty(code).GetRawText();
        return Task.FromResult(new HttpResponseMessage(Unauthorized?HttpStatusCode.Unauthorized:HttpStatusCode.OK){Content=new StringContent(body,Encoding.UTF8,"application/json")});
    }
    protected override void Dispose(bool disposing){Disposed=true;base.Dispose(disposing);}
}
