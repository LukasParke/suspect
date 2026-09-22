package consumer

import example.protocol.*
import com.sun.net.httpserver.HttpServer
import java.net.InetSocketAddress
import java.net.URI
import java.util.Collections
import java.util.concurrent.Executors
import java.util.concurrent.TimeUnit
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout

fun main() = runBlocking {
    val bytes = byteArrayOf(0, -1, -2, 13, 10, 65)
    val seen = Collections.synchronizedList(mutableListOf<Pair<String, ByteArray>>())
    val server = HttpServer.create(InetSocketAddress("127.0.0.1", 0), 0)
    val executor = Executors.newVirtualThreadPerTaskExecutor()
    server.executor = executor
    server.createContext("/") { exchange ->
        try {
            val body = exchange.requestBody.readAllBytes()
            seen.add(exchange.requestHeaders.getFirst("Content-Type") to body)
            val response = if (exchange.requestURI.path == "/api/parts") "\"ok\"".toByteArray() else body
            exchange.responseHeaders.add("Content-Type", if (exchange.requestURI.path == "/api/raw") "application/octet-stream" else "application/json")
            exchange.sendResponseHeaders(200, response.size.toLong())
            exchange.responseBody.write(response)
        } finally { exchange.close() }
    }
    server.start()
    try {
        withTimeout(5000) {
            Client(options = ClientOptions(serverUrl = URI("http://127.0.0.1:${server.address.port}/api"))).use { client ->
                check(client.rawData(RawDataInput(bytes)).data.contentEquals(bytes))
                check(seen.last().first == "application/octet-stream" && seen.last().second.contentEquals(bytes))
                val text = "ordinary JSON string \u0000 😀"
                check(client.jsonMarker(JsonMarkerInput(JsonMarker(text))).data.content == text)
                check(seen.last().first == "application/json")
                check(Json.parse(seen.last().second) == JsonObject(mapOf("content" to JsonString(text))))
                check(client.parts(PartsInput(PartsRequestMultipartBody(Upload(bytes, "raw.bin")))).data == "ok")
                check(seen.last().first.startsWith("multipart/form-data; boundary="))
                val multipart = seen.last().second.toString(Charsets.ISO_8859_1)
                check(multipart.contains("filename=\"raw.bin\"") && multipart.contains(bytes.toString(Charsets.ISO_8859_1)))
                check(Quickstart.firstRequest(client).response.status == 200)
                val count = seen.size
                try { client.jsonMarker(JsonMarkerInput(JsonMarker(""))); error("missing JSON string validation") }
                catch (failure: SdkException) { check(failure.kind == FailureKind.REQUEST_VALIDATION) }
                check(seen.size == count)
            }
        }
        println("KOTLIN_LEGACY_PROFILE_WIRE=${seen.size}")
        println("KOTLIN_LEGACY_PROFILE_NATIVE_PASSED")
    } finally { server.stop(0); executor.shutdownNow(); check(executor.awaitTermination(5, TimeUnit.SECONDS)) }
}
