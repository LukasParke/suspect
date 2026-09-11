package consumer

import example.documents.*
import java.net.URI
import kotlinx.coroutines.runBlocking

fun main() = runBlocking {
    val expected = listOf(
        "__A__/inherited", "__B__/empty", "__B__/storage/fragments/%2e%2e/Api%2Fv1/relative",
        "__B__/root/root", "__A__/absolute/absolute", "__A__/from-default/variable",
        "__B__/storage/variable/variable", "__A__/override/%2e%2e/Api%2Fv1/relative",
        "__A__/local/v1/local", "__B__/storage/api/oauth", "__A__/same//variable"
    )
    var calls = 0
    var hooks = 0
    val jdk = JdkTransport()
    val transport = Transport { request ->
        check(request.url.toASCIIString() == expected[calls]) { "${request.url} != ${expected[calls]}" }
        check(!request.url.toString().contains("logical.example") && !request.url.toString().contains("requested"))
        calls++
        jdk.execute(request)
    }
    val credentials = Credentials(oauth = CredentialProvider { context ->
        hooks++
        check(context.effectiveServer == URI("__B__/storage/api/"))
        check(context.scopes == listOf("read"))
        check(Json.stringify(context.metadata).contains("../authorize") && Json.stringify(context.metadata).contains("token"))
        "Bearer supplied"
    })
    try {
        Client(credentials, transport).use { client ->
            val bytes = byteArrayOf(111, 107, 0)
            check(client.inheritedDoc().data.contentEquals(bytes))
            check(client.emptyDoc().data.contentEquals(bytes))
            check(client.relativeDoc().data.contentEquals(bytes))
            check(client.rootDoc().data.contentEquals(bytes))
            check(client.absoluteDoc().data.contentEquals(bytes))
            check(client.variableDoc().data.contentEquals(bytes))
            check(client.variableDoc(requestOptions = RequestOptions(serverVariables = mapOf("server" to "../variable"))).data.contentEquals(bytes))
            check(client.relativeDoc(requestOptions = RequestOptions(documentUrl = URI("__A__/override/spec.json"))).data.contentEquals(bytes))
            val before = calls
            try { client.localDoc(); error("local file base was guessed") } catch (error: SdkException) { check(error.kind == FailureKind.REQUEST_REPRESENTATION) }
            check(calls == before)
            check(client.localDoc(requestOptions = RequestOptions(documentUrl = URI("__A__/local/docs/spec.json"))).data.contentEquals(bytes))
            check(client.oauthDoc().data.contentEquals(bytes))
            check(client.variableDoc(requestOptions = RequestOptions(serverVariables = mapOf("server" to "__A__/same//"))).data.contentEquals(bytes))
        }
        check(calls == expected.size && hooks == 1)
        println("KOTLIN_DOCUMENT_SERVERS_WIRE=$calls")
        println("KOTLIN_DOCUMENT_SERVERS_PASSED")
    } finally { jdk.close() }
}
