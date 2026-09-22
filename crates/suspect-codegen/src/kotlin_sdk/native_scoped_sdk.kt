package consumer

import example.scoped.sdk.*
import com.sun.net.httpserver.HttpServer
import java.net.InetSocketAddress
import java.net.URI
import java.time.Duration
import java.util.Collections
import java.util.concurrent.Executors
import java.util.concurrent.TimeUnit
import kotlinx.coroutines.*

private suspend inline fun <reified T: Throwable> fails(crossinline action: suspend () -> Unit): T {
    try { action() } catch (failure: Throwable) { check(failure is T) { "expected ${T::class.simpleName}: $failure" }; return failure }
    error("expected ${T::class.simpleName}")
}

fun main() = runBlocking {
    check(scopedPattern().additionalProperties["n_even"] == JsonNumber.of(4))
    check(scopedCarrier().value == JsonString("scoped guide"))
    val requests = Collections.synchronizedList(mutableListOf<String>())
    val server = HttpServer.create(InetSocketAddress("127.0.0.1", 0), 0)
    val threads = Executors.newVirtualThreadPerTaskExecutor()
    server.executor = threads
    server.createContext("/") { exchange ->
        try {
            check(exchange.requestMethod == "POST")
            check(exchange.requestHeaders.getFirst("Content-Type") == "application/json")
            val body = exchange.requestBody.readAllBytes()
            val text = body.toString(Charsets.UTF_8)
            requests.add(text)
            val denied = exchange.requestURI.path.endsWith("/dependencies") && text.contains("denied")
            val response = if (denied) "{\"message\":\"denied\"}".toByteArray() else body
            exchange.responseHeaders.add("Content-Type", "application/json")
            exchange.sendResponseHeaders(if (denied) 422 else 200, response.size.toLong())
            exchange.responseBody.write(response)
        } finally { exchange.close() }
    }
    server.start()
    try {
        Client(options = ClientOptions(serverUrl = URI("http://127.0.0.1:${server.address.port}/api"))).use { client ->
            val extra = linkedMapOf<String, JsonValue>("n_even" to JsonNumber.parse("9007199254740994"), "n_é" to JsonNumber.of(2), "n_é" to JsonNumber.of(3))
            val pattern = PatternRecord(label = "native", remark = Presence.Present(null), additionalProperties = extra)
            val echoed = client.roundtripPatterns(RoundtripPatternsInput(pattern)).data
            check(echoed.kind == PatternRecordKind.RECORD && echoed.remark == Presence.Present(null))
            check(echoed.additionalProperties.keys == extra.keys)
            check((echoed.additionalProperties.getValue("n_even") as JsonNumber).token == "9007199254740994")
            val raw = Json.parse(requests.last()) as JsonObject
            check(raw.values["kind"] == JsonString("record") && raw.values["remark"] === JsonNull)
            extra["n_even"] = JsonNumber.of(3)
            check((echoed.additionalProperties.getValue("n_even") as JsonNumber).token == "9007199254740994")
            check(fails<SdkException> { client.roundtripPatterns(RoundtripPatternsInput(pattern)) }.kind == FailureKind.REQUEST_VALIDATION)
            extra["n_even"] = JsonNumber.of(4)
            val before = requests.size
            fails<SdkException> { client.roundtripPatterns(RoundtripPatternsInput(pattern.copy(additionalProperties = mapOf("not_pattern" to JsonNumber.of(1))))) }
            fails<SdkException> { client.roundtripPatterns(RoundtripPatternsInput(pattern.copy(additionalProperties = mapOf("label" to JsonString("collision"))))) }
            check(requests.size == before)

            val conditional = ConditionalRecord(ConditionalRecordMode.TEXT, JsonString("text"))
            check(client.roundtripConditional(RoundtripConditionalInput(conditional)).data.value == JsonString("text"))
            fails<SdkException> { client.roundtripConditional(RoundtripConditionalInput(conditional.copy(value = JsonNumber.of(1)))) }
            val counted = conditional.copy(mode = ConditionalRecordMode.COUNT, value = JsonNumber.parse("100.000"))
            check(client.roundtripConditional(RoundtripConditionalInput(counted)).data.value == JsonNumber.of(100))

            check(client.roundtripCarrier(RoundtripCarrierInput(ConditionalCarrier(JsonString("string carrier")))).data.value == JsonString("string carrier"))
            check(client.roundtripCarrier(RoundtripCarrierInput(ConditionalCarrier(JsonNumber.of(12)))).data.value == JsonNumber.of(12))
            fails<SdkException> { client.roundtripCarrier(RoundtripCarrierInput(ConditionalCarrier(JsonBoolean(true)))) }
            val values = mutableListOf<JsonValue>(JsonString("head"), JsonNumber.of(2))
            val array = ScopedArray(JsonArray(values))
            check((client.roundtripArray(RoundtripArrayInput(array)).data.value as JsonArray).values == values)
            values.add(JsonBoolean(false))
            fails<SdkException> { client.roundtripArray(RoundtripArrayInput(array)) }

            val members = linkedMapOf<String, JsonValue>("a" to JsonString("one"), "b" to JsonNumber.of(2))
            val composition = ScopedComposition(JsonObject(members))
            val composed = client.roundtripComposition(RoundtripCompositionInput(composition)).data
            members["other"] = JsonBoolean(true)
            check((composed.value as JsonObject).values.keys == setOf("a", "b"))
            fails<SdkException> { client.roundtripComposition(RoundtripCompositionInput(composition)) }

            val absent = DependenciesRecord()
            check(client.roundtripDependencies(RoundtripDependenciesInput(absent)).data.credit === Presence.Absent)
            check(requests.last() == "{}")
            val nullable = absent.copy(credit = Presence.Present(null), billing = Presence.Present("street"))
            check(client.roundtripDependencies(RoundtripDependenciesInput(nullable)).data.credit == Presence.Present(null))
            fails<SdkException> { client.roundtripDependencies(RoundtripDependenciesInput(nullable.copy(billing = Presence.Absent))) }
            val denied = fails<RoundtripDependenciesApiException.Status422> { client.roundtripDependencies(RoundtripDependenciesInput(absent.copy(billing = Presence.Present("denied")))) }
            check(denied.data.message == "denied")
            check(Quickstart.firstRequest(client).response.status == 200)
        }

        val invalidResponse = "{\"kind\":\"record\",\"label\":\"x\",\"n_even\":3}".toByteArray()
        Client(transport = Transport { HttpResponse(200, mapOf("Content-Type" to listOf("application/json")), invalidResponse) }).use { client ->
            val error = fails<SdkException> { client.roundtripPatterns(RoundtripPatternsInput(PatternRecord("valid"))) }
            check(error.kind == FailureKind.RESPONSE_VALIDATION && error.response!!.bodyPreview.contentEquals(invalidResponse))
        }
        var sent = 0
        val transport = Transport { sent++; error("invalid preflight reached transport") }
        Client(transport = transport, options = ClientOptions(codecLimits = CodecLimits(maxEvaluationSteps = 8))).use { client ->
            check(fails<SdkException> { client.roundtripComposition(RoundtripCompositionInput(ScopedComposition(JsonObject(mapOf("a" to JsonString("x")))))) }.kind == FailureKind.EVALUATION)
        }
        val entered = CompletableDeferred<Unit>()
        val paused = object: AbstractMap<String, JsonValue>() {
            override val entries: Set<Map.Entry<String, JsonValue>> get() { entered.complete(Unit); Thread.sleep(50); return mapOf("a" to JsonString("x")).entries }
        }
        Client(transport = transport, options = ClientOptions(timeout = Duration.ofSeconds(5))).use { client ->
            coroutineScope {
                val pending = launch { client.roundtripComposition(RoundtripCompositionInput(ScopedComposition(JsonObject(paused)))) }
                entered.await(); pending.cancelAndJoin()
                check(pending.isCancelled)
            }
        }
        check(sent == 0)
        println("KOTLIN_SCOPED_SDK_WIRE=${requests.size}")
        println("KOTLIN_SCOPED_SDK_NATIVE_PASSED")
    } finally { server.stop(0); threads.shutdownNow(); check(threads.awaitTermination(5, TimeUnit.SECONDS)) }
}
