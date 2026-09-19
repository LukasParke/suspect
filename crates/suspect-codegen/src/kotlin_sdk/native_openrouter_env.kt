package consumer

import ai.openrouter.kotlin.*
import java.net.URI
import kotlinx.coroutines.runBlocking

private const val BODY = """{"data":{"label":"controlled key","limit":null,"limit_remaining":null,"limit_reset":null,"usage":1.0,"usage_daily":0,"usage_weekly":1,"usage_monthly":1,"byok_usage":0,"byok_usage_daily":0,"byok_usage_weekly":0,"byok_usage_monthly":0,"is_free_tier":false,"is_management_key":false,"is_provisioning_key":false,"include_byok_in_limit":false,"creator_user_id":null,"rate_limit":{"requests":-1,"interval":"1h","note":"controlled"}}}"""

fun main(args:Array<String>)=runBlocking {
    var calls=0
    val expected=if(args.single()=="positive")"runtime-env-token"else null
    val transport=Transport {request->
        check(request.method=="GET"&&request.url==URI("https://openrouter.ai/api/v1/key"))
        check(expected!=null&&request.headers["Authorization"]=="Bearer $expected")
        calls++
        HttpResponse(200,mapOf("Content-Type" to listOf("application/json")),BODY.toByteArray())
    }
    suspend fun checkClient(client:Client) {
        client.use {
            if(expected!=null){val result=it.getCurrentKey();check(result.response.status==200&&result.data.data.label=="controlled key"&&result.data.data.limit==null)}
            else{try{it.getCurrentKey();error("missing runtime credential was accepted")}catch(error:SdkException){check(error.kind==FailureKind.AUTHENTICATION&&error.response==null&&error.toString().length<1024&&!error.toString().contains("runtime-env-token"))}}
        }
    }
    Class.forName("ai.openrouter.kotlin.Client")
    checkClient(Client(transport=transport))
    checkClient(Client.fromEnv(transport=transport))
    if(expected!=null)Client.fromEnv(transport=transport).use{client->check(Quickstart.firstRequest(client).data.data.label=="controlled key")}
    check(calls==if(expected==null)0 else 3)
    println("KOTLIN_OPENROUTER_ENV_${args.single().uppercase()}=$calls")
}
