package consumer

import example.sdk.*
import com.sun.net.httpserver.HttpServer
import java.net.InetSocketAddress
import java.net.URI
import java.time.Duration
import java.util.Collections
import java.util.concurrent.CountDownLatch
import java.util.concurrent.Executors
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicInteger
import kotlinx.coroutines.*

private const val WIDGET = """{"id":"w1","amount":9007199254740993.000000000000000001,"meta":null,"payload":{"kind":"standard","text":"plain"},"child":{"label":"root"},"é":"accent","é":"decomposed","雪/~/😀":{"big":1e999999999999999999999999}}"""
private const val PAGE = """{"items":[{"id":"w2","amount":1e-400,"payload":{"kind":"secure","vault":"v1"}}]}"""

private inline fun <reified T : Throwable> bad(block: () -> Unit): T {
    try { block() } catch (failure: Throwable) {
        check(failure is T) { "expected ${T::class.simpleName}, got $failure" }
        return failure
    }
    error("invalid value accepted")
}

private suspend inline fun <reified T : Throwable> fails(crossinline block: suspend () -> Unit): T {
    try { block() } catch (failure: Throwable) {
        check(failure is T) { "expected ${T::class.simpleName}, got $failure" }
        return failure
    }
    error("invalid operation accepted")
}

private fun models() {
    for (token in listOf("-", "01", "1.", "1e", "1e+", "+1", "NaN", "--1", "١")) bad<JsonException> { JsonNumber.parse(token) }
    for (text in listOf("{}true", "[1,]", "{\"a\":1,\"\\u0061\":2}", "\"\\ud800\"", "\"\n\"", "true false", "\"\\uＦＦＦＦ\"")) bad<JsonException> { Json.parse(text) }
    bad<JsonException> { Json.parse(byteArrayOf(0xC0.toByte(), 0xAF.toByte())) }
    bad<JsonException> { Json.stringify(JsonString("\udc00")) }
    check(Json.stringify(JsonNumber.of(8), JsonLimits(maxBytes = 1)) == "8")
    check(Json.stringify(JsonNumber.of(999), JsonLimits(maxBytes = 3)) == "999")
    check(Json.parse("\"😀\"", JsonLimits(maxBytes = 6)) == JsonString("😀"))
    check(bad<JsonException> { Json.stringify(JsonString("😀"), JsonLimits(maxBytes = 5)) }.kind == JsonErrorKind.RESOURCE_LIMIT)
    val number = JsonNumber.parse("9007199254740993.000000000000000001")
    check(!number.isInteger())
    val huge = JsonNumber.parse("1e9999999999999999999999999999")
    check(huge.isInteger() && huge > JsonNumber.parse("9e9999999999999999999999999998"))
    check(JsonNumber.parse("100e-2") == JsonNumber.parse("1.00"))
    check(JsonNumber.parse("100e-2").hashCode() == JsonNumber.parse("1.00").hashCode())
    check(JsonNumber.parse("-0").hashCode() == JsonNumber.parse("0e9999999999999").hashCode())
    check(JsonNumber.parse("1e" + "0".repeat(41)).toLongExact() == 1L)
    check(!JsonNumber.parse("0.1e" + "0".repeat(41)).isInteger())
    check(JsonNumber.parse("10e-" + "0".repeat(40) + "1").toLongExact() == 1L)
    check(JsonNumber.parse("9223372036854775807").toLongExact() == Long.MAX_VALUE)
    bad<ArithmeticException> { JsonNumber.parse("9223372036854775808").toLongExact() }
    bad<ArithmeticException> { number.toBigIntegerExact() }
    check(bad<JsonException> { huge.toBigIntegerExact() }.kind == JsonErrorKind.RESOURCE_LIMIT)
    val input = WidgetInput(name = "alpha")
    check(Codecs.widgetInput.encode(input) == "{\"name\":\"alpha\"}")
    val branch: WidgetPayload = WidgetPayload.AsStandardPayload(StandardPayload(text = "plain"))
    check(Codecs.widgetPayload.encode(branch) == "{\"kind\":\"standard\",\"text\":\"plain\"}")
    val widget = Codecs.widget.decode(WIDGET)
    check(widget.amount == number)
    check(widget.meta == Presence.Present(null))
    check((widget.payload as WidgetPayload.AsStandardPayload).value.kind == StandardPayloadKind.STANDARD)
    check((widget.child as Presence.Present).value.child === Presence.Absent)
    check(widget.additionalProperties["é"] == JsonString("accent"))
    check(widget.additionalProperties["é"] == JsonString("decomposed"))
    check(Json.parse(Codecs.widget.encode(widget)) == Json.parse(WIDGET))
    val absent = Codecs.widget.decode(WIDGET.replace("\"meta\":null,", ""))
    check(absent.meta === Presence.Absent)
    check(!Codecs.widget.encode(absent).contains("\"meta\""))
    check(Codecs.widget.decode(Codecs.widget.encode(widget.copy(meta = Presence.Present("changed")))).meta == Presence.Present("changed"))
    bad<ValidationException> { Codecs.widget.decode(WIDGET.replace("standard", "unknown")) }
    bad<ValidationException> { Codecs.widget.decode(WIDGET.replace("\"amount\":9007199254740993.000000000000000001,", "")) }
    bad<ValidationException> { Codecs.widgetInput.encode(input.copy(name = "")) }
    bad<ValidationException> { Codecs.widgetInput.encode(input.copy(additionalProperties = mapOf("name" to JsonString("collision")))) }
    val mutable = mutableListOf(widget)
    val page = WidgetList(items = mutable)
    mutable.add(widget.copy(payload = WidgetPayload.AsSecurePayload(SecurePayload(vault = "other"))))
    check(Codecs.widgetList.decode(Codecs.widgetList.encode(page)).items.size == 2)
    mutable.add(widget.copy(additionalProperties = mapOf("id" to JsonString("wrong"))))
    bad<ValidationException> { Codecs.widgetList.encode(page) }
    val extraArray = mutableListOf<JsonValue>(JsonString("before"))
    val extras = mutableMapOf<String, JsonValue>("nested" to JsonArray(extraArray))
    val open = input.copy(additionalProperties = extras)
    val snapshot = Codecs.widgetInput.encodeJson(open)
    extraArray[0] = JsonString("after")
    check(Json.stringify(snapshot).contains("before"))
    check(Codecs.widgetInput.encode(open).contains("after"))
    bad<EvaluationException> { Codecs.widgetInput.encode(input.copy(additionalProperties = mapOf("huge" to JsonString("x".repeat(65536)))), CodecLimits(maxModelBytes = 32)) }
    bad<EvaluationException> { Codecs.widgetInput.encode(input.copy(name = "x".repeat(65536)), CodecLimits(maxModelBytes = 32)) }
    bad<EvaluationException> { Codecs.widgetPayload.encode(branch, CodecLimits(maxEqualitySteps = 1)) }
    bad<EvaluationException> { Codecs.widget.decode(WIDGET, CodecLimits(maxEvaluationSteps = 4)) }
    val cycle = mutableMapOf<String, JsonValue>()
    cycle["self"] = JsonObject(cycle)
    bad<EvaluationException> { Codecs.widgetInput.encode(input.copy(additionalProperties = cycle), CodecLimits(json = JsonLimits(maxDepth = 8))) }
    val largeKey = "\nFORGED\t" + "a".repeat(10000)
    val error = bad<ValidationException> { SchemaValidation.validate(Codecs.widgetInput.source, JsonObject(mapOf("name" to JsonString(""), largeKey to JsonNull))) }
    check(!error.toString().contains("\nFORGED") && error.toString().length < 2000)
}

private data class Recorded(val method: String, val target: String, val authorization: String?, val accept: String?, val contentType: String?, val cookie: String?, val body: String)

private suspend fun wire() {
    val records = Collections.synchronizedList(mutableListOf<Recorded>())
    val entered = CompletableDeferred<Unit>()
    val cancelledBody = CompletableDeferred<Unit>()
    val bodyClosed = CountDownLatch(1)
    val release = CountDownLatch(1)
    val oversizedClosed = CountDownLatch(1)
    val server = HttpServer.create(InetSocketAddress("127.0.0.1", 0), 0)
    val executor = Executors.newVirtualThreadPerTaskExecutor()
    server.executor = executor
    server.createContext("/") { exchange ->
        val target = exchange.requestURI.toASCIIString()
        try {
            val body = exchange.requestBody.readAllBytes().toString(Charsets.UTF_8)
            records.add(Recorded(exchange.requestMethod, target, exchange.requestHeaders.getFirst("Authorization"), exchange.requestHeaders.getFirst("Accept"), exchange.requestHeaders.getFirst("Content-Type"), exchange.requestHeaders.getFirst("Cookie"), body))
            if (target.endsWith("/cancelled")) { entered.complete(Unit); release.await(3, TimeUnit.SECONDS) }
            if (target.endsWith("/timeout")) Thread.sleep(300)
            exchange.responseHeaders.add("Content-Type", when {
                target.endsWith("/badmedia") -> "text/plain"
                target.endsWith("/badcharset") -> "application/json; charset=iso-8859-1"
                else -> "Application/JSON; charset=\"utf-8\""
            })
            exchange.responseHeaders.add("Set-Cookie", "implicit=must-not-replay; Path=/")
            if (target.endsWith("/duplicate-media")) exchange.responseHeaders.add("Content-Type", "application/json")
            if (target.endsWith("/encoding")) exchange.responseHeaders.add("Content-Encoding", "gzip")
            if (target.endsWith("/large-headers")) exchange.responseHeaders.add("X-Large", "x".repeat(40000))
            if (target.endsWith("/redirect")) exchange.responseHeaders.add("Location", "/leak")
            if (target.endsWith("/cancel-body") || target.endsWith("/oversized")) {
                exchange.sendResponseHeaders(200, 0)
                exchange.responseBody.write("{".toByteArray()); exchange.responseBody.flush()
                if (target.endsWith("/cancel-body")) cancelledBody.complete(Unit)
                try {
                    repeat(1024) { exchange.responseBody.write(ByteArray(8192) { 32 }); exchange.responseBody.flush(); Thread.sleep(5) }
                } catch (_: java.io.IOException) {
                    if (target.endsWith("/cancel-body")) bodyClosed.countDown() else oversizedClosed.countDown()
                }
                return@createContext
            }
            val status = when {
                body == "{\"name\":\"deny\"}" -> 422
                target.endsWith("/missing") -> 404
                target.endsWith("/redirect") -> 307
                target.endsWith("/unknown") -> 599
                else -> 200
            }
            val response = when {
                status == 422 || status == 404 -> "{\"message\":\"private rejection\"}"
                target.endsWith("/invalid-error") -> "{\"message\":true}"
                target.endsWith("/badjson") -> "{private"
                target.endsWith("/badmodel") -> "{}"
                target.contains('?') -> PAGE
                else -> WIDGET
            }
            val bytes = response.toByteArray()
            exchange.sendResponseHeaders(if (target.endsWith("/invalid-error")) 404 else status, bytes.size.toLong())
            exchange.responseBody.write(bytes)
        } catch (_: java.io.IOException) { /* The client may cancel a body or deadline. */ }
        finally { exchange.close() }
    }
    server.start()
    val base = URI("http://127.0.0.1:${server.address.port}/api/v1")
    try {
        Client(Credentials(apiKey = "test-key"), options = ClientOptions(serverUrl = base)).use { client ->
            val created = client.createWidget(CreateWidgetInput(body = WidgetInput(name = "alpha"))) as CreateWidgetResult.Status200
            check(created.data.amount.token == "9007199254740993.000000000000000001")
            val listed = client.listWidgets(ListWidgetsInput(tag = Presence.Present("a"), tags = Presence.Present(listOf("x", "y")), labels = Presence.Present(listOf("a,b", "c")), limit = Presence.Present(JsonNumber.of(2)))) as ListWidgetsResult.Status200
            check(listed.data.items.single().amount.token == "1e-400")
            client.getWidget(GetWidgetInput(widgetId = "a/b 雪!'()*"))
            client.updateWidget(UpdateWidgetInput(widgetId = "w1", body = WidgetPatch()))
            check(records.take(4).map { it.method + " " + it.target + " " + it.body } == listOf(
                "POST /api/v1/widgets {\"name\":\"alpha\"}",
                "GET /api/v1/widgets?tag=a&tags=x&tags=y&labels=a%2Cb,c&limit=2 ",
                "GET /api/v1/widgets/a%2Fb%20%E9%9B%AA%21%27%28%29%2A ",
                "PATCH /api/v1/widgets/w1 {}"
            )) { records.take(4).toString() }
            val count = records.size
            check(fails<SdkException> { client.listWidgets(ListWidgetsInput(limit = Presence.Present(JsonNumber.of(0)))) }.kind == FailureKind.REQUEST_VALIDATION)
            check(fails<SdkException> { client.createWidget(CreateWidgetInput(WidgetInput(name = ""))) }.kind == FailureKind.REQUEST_VALIDATION)
            check(records.size == count)
            val denied = fails<CreateWidgetApiException.Status422> { client.createWidget(CreateWidgetInput(WidgetInput(name = "deny"))) }
            check(denied.data.message == "private rejection" && denied.response.status == 422 && denied.operationId == "createWidget")
            check(!denied.toString().contains("private rejection") && !denied.toString().contains("test-key"))
            check(fails<GetWidgetApiException.Status404> { client.getWidget(GetWidgetInput("missing")) }.data.message == "private rejection")
            for (path in listOf("badmedia", "badcharset", "duplicate-media", "encoding")) {
                check(fails<SdkException> { client.getWidget(GetWidgetInput(path)) }.kind == FailureKind.CONTENT_TYPE)
            }
            for (path in listOf("badjson", "badmodel", "invalid-error")) {
                val error = fails<SdkException> { client.getWidget(GetWidgetInput(path)) }
                check(error.kind == FailureKind.RESPONSE_VALIDATION && error.response != null && error.operationId == "getWidget")
                check(error.source!!.pointer.endsWith("/get"))
            }
            for (path in listOf("redirect", "unknown")) check(fails<SdkException> { client.getWidget(GetWidgetInput(path)) }.kind == FailureKind.UNEXPECTED_STATUS)
            check(fails<SdkException> { client.getWidget(GetWidgetInput("large-headers")) }.let { it.kind == FailureKind.RESPONSE_LIMIT && it.response!!.headersTruncated })
            check(fails<SdkException> { client.getWidget(GetWidgetInput("timeout"), RequestOptions(timeout = Duration.ofMillis(50))) }.kind == FailureKind.TIMEOUT)
            coroutineScope {
                var sawCancellation = false
                val pending = launch {
                    try { client.getWidget(GetWidgetInput("cancelled")); error("cancellation ignored") }
                    catch (cancelled: CancellationException) { sawCancellation = true; throw cancelled }
                }
                withTimeout(3000) { entered.await() }
                pending.cancelAndJoin()
                check(pending.isCancelled && sawCancellation)
                release.countDown()
            }
            coroutineScope {
                val pending = launch { client.getWidget(GetWidgetInput("cancel-body")) }
                withTimeout(3000) { cancelledBody.await() }
                pending.cancelAndJoin()
                check(pending.isCancelled)
            }
            check(bodyClosed.await(3, TimeUnit.SECONDS)) { "JDK body subscription outlived caller cancellation" }
        }
        Client(Credentials(apiKey = "test-key"), options = ClientOptions(serverUrl = base, maxResponseBytes = 64, captureBytes = 4)).use { bounded ->
            val failure = fails<SdkException> { bounded.getWidget(GetWidgetInput("oversized")) }
            check(failure.kind == FailureKind.RESPONSE_LIMIT && failure.response!!.bodyPreview.size == 4 && failure.response!!.truncated)
        }
        check(oversizedClosed.await(3, TimeUnit.SECONDS)) { "oversized body subscription was not cancelled" }
        check(records.none { it.target.contains("/leak") })
        check(records.all { it.authorization == "Bearer test-key" && it.accept == "application/json" && it.cookie == null })
        check(records.first().contentType == "application/json")
        check(records[1].contentType == null)
        println("KOTLIN_M2_WIRE_EXCHANGES=${records.size}")
    } finally {
        release.countDown()
        server.stop(0)
        executor.shutdownNow()
        check(executor.awaitTermination(5, TimeUnit.SECONDS))
    }
}

private class ClosingTransport(private val action: suspend () -> HttpResponse) : Transport, AutoCloseable {
    var closed = false
    override suspend fun execute(request: HttpRequest): HttpResponse = action()
    override fun close() { closed = true; throw IllegalStateException("private closer failure") }
}

private suspend fun injected() {
    fun reply(headers: Map<String, List<String>> = mapOf("Content-Type" to listOf("application/json")), body: ByteArray = WIDGET.toByteArray(), status: Int = 200): HttpResponse = HttpResponse(status, headers, body)
    val calls = AtomicInteger()
    val transport = Transport { calls.incrementAndGet(); reply() }
    for (token in listOf<String?>(null, "", "====", "x\r\ny", "private token")) {
        Client(Credentials(apiKey = token), transport).use { client -> check(fails<SdkException> { client.getWidget(GetWidgetInput("x")) }.kind == FailureKind.AUTHENTICATION) }
    }
    check(calls.get() == 0)
    // A caller's single-thread timeout must remain schedulable during encoding.
    // This collection intentionally performs a slow, caller-owned getter.
    val slowValues = object : AbstractList<JsonValue>() {
        override val size: Int get() = 1
        override fun get(index: Int): JsonValue { Thread.sleep(75); return JsonString("late") }
    }
    Client(Credentials(apiKey = "test-key"), transport).use { client ->
        fails<TimeoutCancellationException> {
            withTimeout(15) { client.createWidget(CreateWidgetInput(WidgetInput(name = "x", additionalProperties = mapOf("slow" to JsonArray(slowValues))))) }
        }
    }
    check(calls.get() == 0) { "caller timeout was starved by native encoding" }
    Client(Credentials(apiKey = "test-key"), transport, ClientOptions(maxRequestBytes = 16)).use { client ->
        check(fails<SdkException> { client.getWidget(GetWidgetInput("x")) }.kind == FailureKind.REQUEST_LIMIT)
    }
    check(calls.get() == 0)
    Client(Credentials(apiKey = "test-key"), Transport { reply(body = "x".repeat(1024).toByteArray()) }, ClientOptions(maxResponseBytes = 32, captureBytes = 3)).use { client ->
        val error = fails<SdkException> { client.getWidget(GetWidgetInput("x")) }
        check(error.kind == FailureKind.RESPONSE_LIMIT && error.response!!.bodyPreview.size == 3 && error.response!!.truncated)
    }
    val tooManyHeaders = (0..128).associate { "X-$it" to emptyList<String>() }
    Client(Credentials(apiKey = "test-key"), Transport { reply(headers = tooManyHeaders) }).use { client ->
        val error = fails<SdkException> { client.getWidget(GetWidgetInput("x")) }
        check(error.kind == FailureKind.RESPONSE_LIMIT && error.response!!.headers.size <= 128 && error.response!!.headersTruncated)
    }
    Client(Credentials(apiKey = "test-key"), Transport { reply(headers = mapOf("Content-Type" to listOf("application/json"), "X-Unsafe" to listOf("x\nforged"))) }).use { client ->
        check(fails<SdkException> { client.getWidget(GetWidgetInput("x")) }.kind == FailureKind.RESPONSE_METADATA)
    }
    Client(Credentials(apiKey = "test-key"), Transport { reply(body = byteArrayOf(0xc0.toByte(), 0xaf.toByte())) }).use { client ->
        check(fails<SdkException> { client.getWidget(GetWidgetInput("x")) }.kind == FailureKind.RESPONSE_VALIDATION)
    }
    val original = IllegalStateException("private custom transport cause")
    Client(Credentials(apiKey = "test-key"), Transport { throw original }).use { client ->
        val error = fails<SdkException> { client.getWidget(GetWidgetInput("x")) }
        check(error.kind == FailureKind.TRANSPORT && error.cause === original && !error.toString().contains("private"))
    }
    var cleaned = false
    Client(Credentials(apiKey = "test-key"), Transport {
        try { delay(1000); reply() } finally { cleaned = true; throw IllegalStateException("private finally failure") }
    }, ClientOptions(timeout = Duration.ofMillis(25))).use { client ->
        check(fails<SdkException> { client.getWidget(GetWidgetInput("x")) }.kind == FailureKind.TIMEOUT)
        check(cleaned)
    }
    cleaned = false
    val entered = CompletableDeferred<Unit>()
    Client(Credentials(apiKey = "test-key"), Transport {
        try { entered.complete(Unit); awaitCancellation() } finally { cleaned = true; throw IllegalStateException("private finally failure") }
    }).use { client ->
        coroutineScope {
            var cancelled = false
            val job = launch {
                try { client.getWidget(GetWidgetInput("x")) }
                catch (error: CancellationException) { cancelled = true; throw error }
            }
            entered.await(); job.cancelAndJoin()
            check(cancelled && cleaned)
        }
    }
    val closing = ClosingTransport { reply(body = byteArrayOf()) }
    val primary = fails<SdkException> {
        Client(Credentials(apiKey = "test-key"), closing).use { it.getWidget(GetWidgetInput("x")) }
    }
    check(primary.kind == FailureKind.RESPONSE_VALIDATION && primary.suppressed.size == 1 && closing.closed)
    check(primary.suppressed.single() is SdkException && !primary.suppressed.single().toString().contains("private"))
    for (url in listOf("https://user:pass@example.test", "https://example.test/api/../private", "https://example.test/api/%2e%2e%2fprivate", "https://example.test:70000", "http://example.test", "https://example.test?query")) {
        bad<IllegalArgumentException> { ClientOptions(serverUrl = URI(url)) }
    }
    println("KOTLIN_INJECTED_TRANSPORT_AND_CLEANUP_PASSED")
}

fun main() = runBlocking {
    withTimeout(30000) { models(); wire(); injected() }
    println("KOTLIN_M2_NATIVE_PASSED")
}
