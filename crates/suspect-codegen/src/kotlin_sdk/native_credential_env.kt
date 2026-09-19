package consumer

import example.credentialenv.*
import java.net.URI
import kotlinx.coroutines.runBlocking

private suspend fun missing(call:suspend()->Unit):SdkException {
    try {call()}catch(error:SdkException){
        check(error.kind==FailureKind.AUTHENTICATION && error.response==null)
        check(error.toString().length<1024 && listOf("system-token","explicit-token","before-token","after-token","private-reader-value").none{error.toString().contains(it)})
        return error
    }
    error("expected authentication failure before transport")
}

fun main()=runBlocking {
    val requests=mutableListOf<HttpRequest>()
    val transport=Transport{request->
        check(request.url.scheme=="https"&&request.url.host=="source.example"&&request.url.rawPath.startsWith("/api/v1/"))
        requests.add(request)
        HttpResponse(200,mapOf("Content-Type" to listOf("application/json")),if(request.url.path.endsWith("/reserved"))"{\"label\":\"model\"}".toByteArray()else"\"ok\"".toByteArray())
    }
    val environment=mutableMapOf<String,String?>("KOTLIN_ENV_A" to "before-token","KOTLIN_ENV_B" to "header-value","KOTLIN_ENV_QUERY" to "a/b +雪","KOTLIN_ENV_COOKIE" to "cookie-value","KOTLIN_ENV_ALIAS_B" to "alias-token")
    val reads=mutableMapOf<String,Int>()
    val reader=CredentialEnvironment{name->reads[name]=(reads[name]?:0)+1;environment[name]}
    Class.forName("example.credentialenv.Client")
    check(reads.isEmpty())
    Client.fromEnv(transport=transport,environment=reader).use{client->
        check(reads.values.sum()==5 && reads.values.all{it==1})
        environment["KOTLIN_ENV_A"]="after-token"
        check(client.protectedCall().data=="ok")
        check(requests.last().headers["Authorization"]=="Bearer before-token")
        client.conjunction();check(requests.last().headers["Authorization"]=="Bearer before-token"&&requests.last().headers["X-Key"]=="header-value")
        client.aliasA();check(requests.last().headers["Authorization"]=="Bearer before-token")
        client.aliasB();check(requests.last().headers["Authorization"]=="Bearer alias-token")
        client.query();check(requests.last().url.rawQuery=="key=a%2Fb%20%2B%E9%9B%AA")
        client.cookie();check(requests.last().headers["Cookie"]=="session=cookie-value")
        client.alternatives(requestOptions=RequestOptions(securityAlternative=2));check(requests.last().headers["Authorization"]==null&&requests.last().headers["X-Key"]==null)
        client.fromEnv2();check(client.reserved().data.label=="model")
        val model=CredentialEnvironment2("native model");check(model.label=="native model")
        check(reads.values.sum()==5)
    }
    Client.fromEnv(transport=transport,environment=reader).use{client->client.protectedCall();check(requests.last().headers["Authorization"]=="Bearer after-token")}
    check(reads.values.sum()==10)
    for(readerValue in listOf<CredentialEnvironment>(CredentialEnvironment{null},CredentialEnvironment{""},CredentialEnvironment{throw SecurityException("private-reader-value")})) {
        Client.fromEnv(transport=transport,environment=readerValue).use{client->
            client.anonymous();check(requests.last().headers["Authorization"]==null)
            client.alternatives();check(requests.last().headers["Authorization"]==null&&requests.last().headers["X-Key"]==null)
            val before=requests.size;missing{client.protectedCall()};missing{client.conjunction()};check(requests.size==before)
        }
    }
    Client.fromEnv(transport=transport,environment=CredentialEnvironment{if(it=="KOTLIN_ENV_B")"only-header"else null}).use{client->
        client.alternatives();check(requests.last().headers["X-Key"]=="only-header"&&requests.last().headers["Authorization"]==null)
        val before=requests.size;missing{client.alternatives(requestOptions=RequestOptions(securityAlternative=0))};missing{client.conjunction()};check(requests.size==before)
        client.alternatives(requestOptions=RequestOptions(securityAlternative=1));check(requests.last().headers["X-Key"]=="only-header")
    }
    Client.fromEnv(transport=transport,environment=CredentialEnvironment{if(it=="KOTLIN_ENV_A")"primary-only"else null}).use{client->
        client.protectedCall();check(requests.last().headers["Authorization"]=="Bearer primary-only")
        val before=requests.size;missing{client.aliasB()};check(requests.size==before)
    }
    Client.fromEnv(transport=transport,environment=CredentialEnvironment{"invalid\r\nprivate-reader-value"}).use{client->
        val before=requests.size;missing{client.protectedCall()};missing{client.cookie()};check(requests.size==before);client.anonymous()
    }
    check(System.getenv("KOTLIN_ENV_A")=="system-token")
    Client(transport=transport).use{client->client.protectedCall();check(requests.last().headers["Authorization"]=="Bearer system-token")}
    Client(Credentials(__PRIMARY_MEMBER__="explicit-token"),transport).use{client->
        client.protectedCall();check(requests.last().headers["Authorization"]=="Bearer explicit-token")
        client.aliasB();check(requests.last().headers["Authorization"]=="Bearer explicit-token")
    }
    for(credentials in listOf(Credentials(),Credentials(__PRIMARY_MEMBER__=null),Credentials(__PRIMARY_MEMBER__=""),Credentials(headerKey="explicit-header"))) {
        Client(credentials,transport).use{client->
            val before=requests.size;missing{client.protectedCall()};missing{client.conjunction()};check(requests.size==before)
            client.anonymous();check(requests.last().headers["Authorization"]==null)
        }
    }
    Client(Credentials(headerKey="explicit-header"),transport).use{client->client.alternatives();check(requests.last().headers["X-Key"]=="explicit-header"&&requests.last().headers["Authorization"]==null)}
    println("KOTLIN_CREDENTIAL_ENV_CONTROLS=${requests.size}")
    println("KOTLIN_CREDENTIAL_ENV_NATIVE_PASSED")
}
