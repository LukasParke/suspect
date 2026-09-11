import ai.openrouter.sdk.*;
import ai.openrouter.sdk.Client.*;
import java.util.*;

/** Actual OpenRouter schemas; JDK transport capture, never real account requests. */
public final class NativeCredentialEnvOpenRouter {
    private NativeCredentialEnvOpenRouter() {}
    private static final String KEY="{\"data\":{\"label\":\"controlled\",\"limit\":null,\"limit_remaining\":null,\"limit_reset\":null,\"usage\":1e-400,\"usage_daily\":0,\"usage_weekly\":0,\"usage_monthly\":0,\"byok_usage\":0,\"byok_usage_daily\":0,\"byok_usage_weekly\":0,\"byok_usage_monthly\":0,\"is_free_tier\":false,\"is_management_key\":false,\"is_provisioning_key\":false,\"include_byok_in_limit\":false,\"creator_user_id\":null,\"rate_limit\":{\"requests\":-1,\"interval\":\"none\",\"note\":\"controlled\"}}}";
    private static void check(boolean value,String message){if(!value)throw new AssertionError(message);}
    private static String authorization(NativeSupport.Mock transport){return transport.requests.getLast().headers().entrySet().stream().filter(e->e.getKey().equalsIgnoreCase("Authorization")).map(e->e.getValue().getFirst()).findFirst().orElse("");}
    public static void main(String[] args)throws Exception{
        var key=new NativeSupport.Mock(new NativeSupport.Script(200,"application/json",KEY));
        try(var client=Client.fromEnv(key)){
            var result=client.getCurrentKey();check(result.status()==200&&!result.data().data().isManagementKey()&&result.data().data().limit()==null&&result.data().data().usage().token().equals("1e-400"),"actual current-key schema decoding");
            check(key.requests.getLast().method().equals("GET")&&key.requests.getLast().uri().equals("https://openrouter.ai/api/v1/key"),"source-default HTTPS GET /key");check(authorization(key).equals("Bearer controlled-openrouter-token"),"source apiKey is bearer");
        }
        var credits=new NativeSupport.Mock(new NativeSupport.Script(200,"application/json","{\"data\":{\"total_credits\":100.25,\"total_usage\":1.5}}"));
        try(var client=Client.fromEnv(credits)){
            var result=client.getCreditsAsync().get();check(result.status()==200&&result.data().data().totalCredits().token().equals("100.25"),"actual optional credits schema");check(credits.requests.getLast().uri().equals("https://openrouter.ai/api/v1/credits"),"source-default optional management path");
        }
        var missing=new NativeSupport.Mock(new NativeSupport.Script(200,"application/json",KEY));
        try(var client=new Client(HttpRuntime.Options.builder().httpClient(missing).build())){try{client.getCurrentKey();throw new AssertionError("explicit empty options used environment");}catch(SdkException error){check(error.kind().equals("missing-credential")&&missing.requests.isEmpty()&&error.capture().length==0&&error.getCause()==null,"missing key did not fail before HTTPS");}}
        var denied=new NativeSupport.Mock(new NativeSupport.Script(401,"application/json","{\"error\":{\"code\":401,\"message\":\"denied\"}}"));
        try(var client=Client.fromEnv(denied)){try{client.getCurrentKey();throw new AssertionError("401 became success");}catch(GetCurrentKeyStatus401 error){check(error.status()==401&&error.data().error().message().equals("denied")&&!error.toString().contains("controlled-openrouter-token"),"typed current-key failure leaked credential");}}
        check(key.shutdowns.get()==0&&credits.shutdowns.get()==0&&denied.shutdowns.get()==0,"caller transport ownership");
        System.out.println("JAVA_CREDENTIAL_ENV_OPENROUTER_OK: compiled ai.openrouter.sdk.Client.fromEnv; source-default GET /key and optional /credits; typed decode and secret-free pre-HTTP missing credentials");
    }
}
