package consumer

import example.sdk.*
import com.sun.net.httpserver.HttpServer
import java.net.InetSocketAddress
import java.net.URI
import java.util.Collections
import java.util.concurrent.Executors
import java.util.concurrent.TimeUnit
import kotlinx.coroutines.runBlocking

private val creditsJson = __CREDITS_JSON__
private val createJson = __CREATE_JSON__
private val updateJson = __UPDATE_JSON__
private val fileJson = __FILE_JSON__
private val listJson = __LIST_JSON__

private data class Request(val method: String, val path: String, val auth: String?, val accept: String?, val contentType: String?, val body: String)

private inline fun <reified T : Throwable> rejected(block: () -> Unit): T {
    try { block() } catch (error: Throwable) { check(error is T) { "expected ${T::class.simpleName}, got $error" }; return error }
    error("expected rejection")
}

fun main() = runBlocking {
    val records = Collections.synchronizedList(mutableListOf<Request>())
    val server = HttpServer.create(InetSocketAddress("127.0.0.1", 0), 0)
    val executor = Executors.newVirtualThreadPerTaskExecutor()
    server.executor = executor
    var creditsCalls = 0
    server.createContext("/") { exchange ->
        try {
            val path = exchange.requestURI.toASCIIString()
            val body = exchange.requestBody.readAllBytes().toString(Charsets.UTF_8)
            records.add(Request(exchange.requestMethod, path, exchange.requestHeaders.getFirst("Authorization"), exchange.requestHeaders.getFirst("Accept"), exchange.requestHeaders.getFirst("Content-Type"), body))
            val response = when {
                path == "/api/v1/credits" && creditsCalls++ == 0 -> 200 to creditsJson
                path == "/api/v1/credits" -> 401 to "{\"error\":{\"code\":401,\"message\":\"Missing Authentication header\"}}"
                path == "/api/v1/keys" -> 201 to createJson
                path.startsWith("/api/v1/keys/") -> 200 to updateJson
                path.startsWith("/api/v1/containers/sess_abc123/files?") -> 200 to listJson
                else -> 200 to fileJson
            }
            val bytes = response.second.toByteArray()
            exchange.responseHeaders.add("Content-Type", "application/json")
            exchange.sendResponseHeaders(response.first, bytes.size.toLong())
            exchange.responseBody.write(bytes)
        } finally { exchange.close() }
    }
    server.start()
    try {
        val base = URI("http://127.0.0.1:${server.address.port}/api/v1")
        Client(Credentials(apiKey = "test-management-token"), options = ClientOptions(serverUrl = base)).use { client ->
            val credits = client.getCredits() as GetCreditsResult.Status200
            check(credits.data.data.totalCredits.token == "100.50000000000000001")
            try { client.getCredits(); error("declared failure became success") }
            catch (error: GetCreditsApiException.Status401) {
                check(error.data.error.message == "Missing Authentication header")
                check(error.response.status == 401 && error.operationId == "getCredits")
                check(!error.toString().contains("Missing Authentication") && !error.toString().contains("test-management-token"))
            }
            val created = client.createKeys(CreateKeysInput(body = __CREATE_BODY__(name = "Native Test Key", limit = Presence.Present(JsonNumber.parse("50.250")), limitReset = Presence.Present(null)))) as CreateKeysResult.Status201
            check(created.data.key == "fixture-secret" && created.data.data.updatedAt == null)
            check(created.data.data.limit!!.token == "50.250")
            check(created.data.data.expiresAt === Presence.Absent)
            val source = (Codecs.__CREATE_CODEC__.encodeJson(__CREATE_BODY__(name = "optional")) as JsonObject).values
            check(!source.containsKey("limit") && !source.containsKey("limit_reset") && !source.containsKey("workspace_id"))
            check(Codecs.__CREATE_CODEC__.encode(__CREATE_BODY__(name = "enum", limitReset = Presence.Present(__RESET_ENUM__.MONTHLY))).contains("monthly"))
            rejected<ValidationException> { Codecs.__CREATE_CODEC__.decode("{\"name\":\"x\",\"limit_reset\":\"yearly\"}") }
            val updated = client.updateKeys(UpdateKeysInput(hash = "fixture/hash 雪", body = __UPDATE_BODY__(name = Presence.Present("Updated Native Key"), limit = Presence.Present(JsonNumber.parse("75.50")), limitReset = Presence.Present(null), disabled = Presence.Present(true)))) as UpdateKeysResult.Status200
            check(updated.data.data.disabled && updated.data.data.limit!!.token == "75.50")
            check(Codecs.__UPDATE_CODEC__.encode(__UPDATE_BODY__()) == "{}")
            val file = client.getContainerFile(GetContainerFileInput(containerId = "sess_abc123", fileId = "cfile_a/b 雪!'()*")) as GetContainerFileResult.Status200
            check(file.data.bytes.toLongExact() == 123L && file.data.objectValue.wireValue == "container.file")
            val page = client.listContainerFiles(ListContainerFilesInput(containerId = "sess_abc123", limit = Presence.Present(JsonNumber.of(2)), after = Presence.Present("a/b +雪"))) as ListContainerFilesResult.Status200
            check(page.data.data.single().path == "out/report.csv" && !page.data.hasMore)
            check(page.data.firstId == "cfile-1")
            check(records.size == 6)
            check(records[0].method == "GET" && records[0].path == "/api/v1/credits")
            check(records[2].method == "POST" && records[2].path == "/api/v1/keys")
            check(records[2].body == "{\"limit\":50.250,\"limit_reset\":null,\"name\":\"Native Test Key\"}")
            check(records[3].method == "PATCH" && records[3].path == "/api/v1/keys/fixture%2Fhash%20%E9%9B%AA")
            check(records[3].body == "{\"disabled\":true,\"limit\":75.50,\"limit_reset\":null,\"name\":\"Updated Native Key\"}")
            check(records[4].path == "/api/v1/containers/sess_abc123/files/cfile_a%2Fb%20%E9%9B%AA%21%27%28%29%2A")
            check(records[5].path == "/api/v1/containers/sess_abc123/files?limit=2&after=a%2Fb%20%2B%E9%9B%AA")
            check(records.all { it.auth == "Bearer test-management-token" && it.accept == "application/json" })
            check(records[2].contentType == "application/json" && records[3].contentType == "application/json")
            check(records[0].contentType == null)
            val count = records.size
            try { client.createKeys(CreateKeysInput(body = __CREATE_BODY__(name = ""))); error("invalid request sent") }
            catch (error: SdkException) { check(error.kind == FailureKind.REQUEST_VALIDATION) }
            check(records.size == count)
        }
        val integer = Codecs.containerFile.decode(fileJson.replace("\"bytes\":123", "\"bytes\":10e-" + "0".repeat(40) + "1"))
        check(integer.bytes.toLongExact() == 1L)
        val large = Codecs.containerFile.decode(fileJson.replace("\"bytes\":123", "\"bytes\":9007199254740993"))
        check(large.bytes.toBigIntegerExact().toString() == "9007199254740993")
        rejected<ValidationException> { Codecs.containerFile.decode(fileJson.replace("\"bytes\":123", "\"bytes\":1.1")) }
        val createResponse = SchemaValidation.sources().single { it.pointer == "/paths/~1keys/post/responses/201/content/application~1json/schema" }
        rejected<ValidationException> { SchemaValidation.validate(createResponse, Json.parse(createJson.replace("\"updated_at\":null,", ""))) }
        check(Json.stringify(Json.parse(createJson)).contains("\"updated_at\":null"))
        println("KOTLIN_OPENROUTER_OPERATIONS=5 WIRE_EXCHANGES=6")
    } finally {
        server.stop(0); executor.shutdownNow(); check(executor.awaitTermination(5, TimeUnit.SECONDS))
    }
}
