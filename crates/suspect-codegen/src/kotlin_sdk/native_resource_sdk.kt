package consumer

import example.resources.sdk.*
import com.sun.net.httpserver.HttpServer
import java.net.InetSocketAddress
import java.net.URI
import java.util.Collections
import java.util.concurrent.Executors
import java.util.concurrent.TimeUnit
import kotlinx.coroutines.runBlocking

private suspend inline fun <reified T: Throwable> fails(crossinline call: suspend () -> Unit): T {
    try { call() } catch (error: Throwable) { check(error is T) { "expected ${T::class.simpleName}: $error" }; return error }
    error("expected ${T::class.simpleName}")
}
fun main() = runBlocking {
    check(dynamicStrings().single().value == JsonString("dynamic guide"))
    val bodies = Collections.synchronizedList(mutableListOf<String>())
    val server = HttpServer.create(InetSocketAddress("127.0.0.1", 0), 0)
    val threads = Executors.newVirtualThreadPerTaskExecutor(); server.executor = threads
    server.createContext("/") { exchange ->
        try {
            check(exchange.requestMethod == "POST" && exchange.requestHeaders.getFirst("Content-Type") == "application/json")
            val body = exchange.requestBody.readAllBytes(); bodies.add(body.toString(Charsets.UTF_8))
            exchange.responseHeaders.add("Content-Type", "application/json")
            exchange.sendResponseHeaders(200, body.size.toLong()); exchange.responseBody.write(body)
        } finally { exchange.close() }
    }
    server.start()
    try {
        Client(options = ClientOptions(serverUrl = URI("http://127.0.0.1:${server.address.port}/api"))).use { client ->
            val member = linkedMapOf<String, JsonValue>("data" to JsonString("child"))
            val tree = Tree(data = Presence.Present(JsonString("root")), children = Presence.Present(listOf(TreeChildrenItem(JsonObject(member)))))
            val strict = client.strict(StrictInput(tree)).data
            check((strict.children as Presence.Present).value.single().value == JsonObject(mapOf("data" to JsonString("child"))))
            member["unexpected"] = JsonBoolean(true)
            check(((strict.children as Presence.Present).value.single().value as JsonObject).values.keys == setOf("data"))
            check(fails<SdkException> { client.strict(StrictInput(tree)) }.kind == FailureKind.REQUEST_VALIDATION)
            check((client.tree(TreeInput(tree)).data.children as Presence.Present).value.single().value == JsonObject(member))
            val numbers = listOf(NumbersItem(JsonNumber.parse("9007199254740993")))
            check(client.numbers(NumbersInput(numbers)).data.single().value == JsonNumber.parse("9007199254740993"))
            val strings = listOf(NumbersItem(JsonString("dynamic string")))
            check(client.strings(StringsInput(strings)).data.single().value == JsonString("dynamic string"))
            check(fails<SdkException> { client.strings(StringsInput(numbers)) }.kind == FailureKind.REQUEST_VALIDATION)
            check(fails<SdkException> { client.numbers(NumbersInput(strings)) }.kind == FailureKind.REQUEST_VALIDATION)
            val precise = JsonNumber.parse("9007199254740993.0000000000000000001")
            check(fails<SdkException> { client.counter(CounterInput(precise)) }.kind == FailureKind.REQUEST_VALIDATION)
            check(client.counter(CounterInput(JsonNumber.parse("9007199254740993"))).data.token == "9007199254740993")
            check(client.poly(PolyInput(Poly(JsonString("resource carrier")))).data.value == JsonString("resource carrier"))
            check(client.poly(PolyInput(Poly(JsonNumber.of(3)))).data.value == JsonNumber.of(3))
            fails<SdkException> { client.poly(PolyInput(Poly(JsonBoolean(false)))) }
            check(Quickstart.firstRequest(client).response.status == 200)
        }
        val invalid = "{\"children\":[{\"unexpected\":1}]}".toByteArray()
        Client(transport = Transport { HttpResponse(200, mapOf("Content-Type" to listOf("application/json")), invalid) }).use { client ->
            val error = fails<SdkException> { client.strict(StrictInput(Tree())) }
            check(error.kind == FailureKind.RESPONSE_VALIDATION)
            val findings = (error.cause as ValidationException).findings
            check(findings.any { it.source.document.startsWith("file:") && it.source.pointer.endsWith("/Strict/unevaluatedProperties") && it.instancePath == "/children/0/unexpected" })
        }
        println("KOTLIN_RESOURCE_SDK_WIRE=${bodies.size}")
        println("KOTLIN_RESOURCE_SDK_NATIVE_PASSED")
    } finally { server.stop(0); threads.shutdownNow(); check(threads.awaitTermination(5, TimeUnit.SECONDS)) }
}
