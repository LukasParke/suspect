import example.credentialenv.*;
import java.util.*;
import java.util.concurrent.atomic.AtomicInteger;
import java.util.function.Function;

/** Public factory/constructor controls; transport never connects to an account. */
public final class NativeCredentialEnv {
    private NativeCredentialEnv() {}
    private static void check(boolean value,String message){if(!value)throw new AssertionError(message);}
    private static NativeSupport.Mock transport(){return new NativeSupport.Mock(new NativeSupport.Script(200,"application/json","true"));}
    private static String header(NativeSupport.Mock transport,String name){return transport.requests.getLast().headers().entrySet().stream().filter(e->e.getKey().equalsIgnoreCase(name)).map(e->e.getValue().getFirst()).findFirst().orElse("");}
    private static void missing(NativeSupport.Mock transport,Runnable action){int before=transport.requests.size();try{action.run();}catch(SdkException error){check(error.kind().equals("missing-credential")&&error.status()==0&&error.capture().length==0&&error.getCause()==null,"missing credential outcome");check(!error.toString().contains("secret")&&transport.requests.size()==before,"credential failure leaked data or reached HTTP");return;}throw new AssertionError("missing credential accepted");}
    private static void invalid(Runnable action){try{action.run();}catch(IllegalArgumentException|NullPointerException error){check(error.getMessage()==null||!error.getMessage().contains("secret"),"explicit invalid credential leaked data");return;}throw new AssertionError("explicit invalid argument was replaced");}
    private static void anonymous(Client client,NativeSupport.Mock transport){check(client.publicValue().data(),"anonymous response");check(header(transport,"Authorization").isEmpty()&&header(transport,"X-Key").isEmpty()&&header(transport,"Cookie").isEmpty(),"anonymous operation attached unused credentials");check(client.fromEnv2().data(),"source collision method was lost");}
    private static void explicitWins(){
        var transport=transport();
        try(var client=new Client(HttpRuntime.Options.builder().httpClient(transport).credential("bearer","explicit-token").build())){
            client.bearerValue();check(header(transport,"Authorization").equals("Bearer explicit-token"),"explicit token lost precedence");missing(transport,client::both);anonymous(client,transport);
        }
        try(var client=new Client(HttpRuntime.Options.builder().httpClient(transport).apiKey("header","explicit-key").build())){
            client.either();check(header(transport,"Authorization").isEmpty()&&header(transport,"X-Key").equals("explicit-key"),"environment filled explicit missing member");missing(transport,client::both);
        }
        try(var client=new Client(HttpRuntime.Options.builder().httpClient(transport).build())){anonymous(client,transport);missing(transport,client::bearerValue);missing(transport,client::either);}
        invalid(()->HttpRuntime.Options.builder().credential("bearer", ""));invalid(()->HttpRuntime.Options.builder().credential("bearer",null));invalid(()->HttpRuntime.Options.builder().apiKey("header", ""));invalid(()->HttpRuntime.Options.builder().apiKey("header",null));invalid(()->new Client(null));
    }
    private static void snapshot(){
        var values=new HashMap<String,String>();values.put("CRED_ENV_BEARER","first-token");values.put("CRED_ENV_HEADER","first-key");var calls=new AtomicInteger();
        Function<String,String> lookup=name->{calls.incrementAndGet();return values.get(name);};var first=transport();
        Client.operationMetadata();check(calls.get()==0,"class metadata consulted environment");
        try(var client=Client.fromEnv(first,lookup)){
            check(calls.get()==4,"mapped variables not snapshotted exactly once");values.put("CRED_ENV_BEARER","second-token");values.put("CRED_ENV_HEADER","second-key");
            client.bearerValue();check(header(first,"Authorization").equals("Bearer first-token"),"old client reread environment");client.aliasValue();check(header(first,"Authorization").equals("Bearer first-token"),"same variable bindings were not one snapshot");client.both();check(header(first,"X-Key").equals("first-key")&&calls.get()==4,"per-operation environment lookup");
            var second=transport();try(var newer=Client.fromEnv(second,lookup)){newer.both();check(header(second,"Authorization").equals("Bearer second-token")&&header(second,"X-Key").equals("second-key")&&calls.get()==8,"new client did not resnapshot");}
        }
        var unavailable=transport();try(var client=Client.fromEnv(unavailable,name->{throw new SecurityException("secret-unavailable");})){anonymous(client,unavailable);missing(unavailable,client::either);}
        try(var client=Client.fromEnv(unavailable,null)){anonymous(client,unavailable);missing(unavailable,client::either);}
        var alternative=transport();try(var client=Client.fromEnv(alternative,name->{if(name.equals("CRED_ENV_HEADER"))return "usable-key";throw new IllegalStateException("secret-unavailable");})){client.either();check(header(alternative,"X-Key").equals("usable-key"),"unused unavailable alternative blocked auth");}
        var oversized=transport();try(var client=Client.fromEnv(oversized,name->"x".repeat(16385))){anonymous(client,oversized);missing(oversized,client::both);}
        var called=new AtomicInteger();invalid(()->Client.fromEnv(null,name->{called.incrementAndGet();return "secret";}));check(called.get()==0,"explicit null transport triggered fallback");
    }
    private static void explicitAccessor(){
        var transport=transport();
        try(var client=Client.fromEnv(transport,null)){anonymous(client,transport);missing(transport,client::bearerValue);}
        try(var client=Client.fromEnv(transport,name->null)){missing(transport,client::either);}
        try(var client=Client.fromEnv(transport,name->name.equals("CRED_ENV_HEADER")?"accessor-key":null)){
            client.either();check(header(transport,"Authorization").isEmpty()&&header(transport,"X-Key").equals("accessor-key"),"partial explicit accessor fell back to process environment");missing(transport,client::both);
        }
        try(var client=Client.fromEnv(transport,name->{throw new SecurityException("secret-accessor-error");})){anonymous(client,transport);missing(transport,client::either);}
    }
    public static void main(String[] args){
        String mode=args[0];if(mode.equals("snapshot")){snapshot();System.out.println("JAVA_CREDENTIAL_ENV_SNAPSHOT_OK");return;}
        if(mode.equals("accessor")){explicitAccessor();System.out.println("JAVA_CREDENTIAL_ENV_EXPLICIT_ACCESSOR_OK");return;}
        var transport=transport();
        try(var client=Client.fromEnv(transport)){
            anonymous(client,transport);
            if(mode.equals("positive")){
                client.bearerValue();check(header(transport,"Authorization").equals("Bearer env-bearer-secret"),"runtime bearer binding");check(transport.requests.getLast().uri().equals("https://sdk-env.example.test/v1/bearer"),"source-default HTTPS server changed");
                client.aliasValue();check(header(transport,"Authorization").equals("Bearer env-bearer-secret"),"shared variable binding");
                client.keys();check(header(transport,"X-Key").equals("env-header-secret")&&header(transport,"Cookie").equals("session=cookie-secret")&&transport.requests.getLast().uri().equals("https://sdk-env.example.test/v1/keys?key=query%20%2B%2F%E9%9B%AA"),"source API-key attachments");
                client.either();check(header(transport,"Authorization").equals("Bearer env-bearer-secret"),"OR order changed");client.both();check(header(transport,"X-Key").equals("env-header-secret"),"AND incomplete");
                client.optionalAuth(RequestOptions.builder().securityAlternative(1).build());check(header(transport,"Authorization").isEmpty(),"explicit anonymous alternative ignored");explicitWins();
            }else if(mode.equals("alternative")){
                client.either();check(header(transport,"X-Key").equals("env-header-secret")&&header(transport,"Authorization").isEmpty(),"satisfiable OR alternative blocked");missing(transport,client::both);missing(transport,()->client.either(RequestOptions.builder().securityAlternative(0).build()));client.optionalAuth();check(header(transport,"Authorization").isEmpty(),"anonymous fallback blocked");
            }else{
                missing(transport,client::bearerValue);missing(transport,client::aliasValue);missing(transport,client::keys);missing(transport,client::both);missing(transport,client::either);client.optionalAuth();check(header(transport,"Authorization").isEmpty(),"optional anonymous call failed");
            }
        }
        check(transport.shutdowns.get()==0,"caller-owned transport closed");
        try(var client=Client.fromEnv()){check(client!=null,"default fromEnv factory");}
        System.out.println("JAVA_CREDENTIAL_ENV_"+mode.toUpperCase(Locale.ROOT)+"_OK");
    }
}
