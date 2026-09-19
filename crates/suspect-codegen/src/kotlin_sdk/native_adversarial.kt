package consumer

import example.sdk.*
import java.util.concurrent.atomic.AtomicInteger
import kotlinx.coroutines.runBlocking

private const val hostileKey = "\$payload */ \u000c 😀"
private const val hostileText = "\$message */ \u000c \u2028 <script>alert('x')</script>"

private inline fun <reified T : Throwable> rejected(block: () -> Unit): T {
    try { block() } catch (error: Throwable) { check(error is T) { "expected ${T::class.simpleName}, got $error" }; return error }
    error("expected rejection")
}

fun main() = runBlocking {
    val value = Value(requiredNullable = null, __HOSTILE_MEMBER__ = hostileText)
    val wire = Codecs.value.encode(value)
    val raw = (Json.parse(wire) as JsonObject).values
    check(raw["required_nullable"] === JsonNull && raw[hostileKey] == JsonString(hostileText))
    check(!raw.containsKey("flag") && !raw.containsKey("token"))
    val parsed = Codecs.value.decode(wire)
    check(parsed.requiredNullable == null && parsed.__HOSTILE_MEMBER__ == hostileText)
    rejected<ValidationException> { Codecs.value.decode("{}") }
    rejected<ValidationException> { Codecs.value.encode(value.copy(token = Presence.Present("a1\n"))) }
    check(Codecs.value.encode(value.copy(token = Presence.Present("a123"))).contains("a123"))
    val selected = rejected<ValidationException> { Codecs.split.encode(Split.AsVariant1("x")) }
    check(selected.findings.first().source.pointer.endsWith("/oneOf/0/minLength"))
    check(Codecs.split.decode("\"x\"") is Split.AsVariant2)
    check(Codecs.split.encode(Split.AsVariant1("long")) == "\"long\"")
    val parent = rejected<ValidationException> { Codecs.parent.encode(Parent.AsVariant1(JsonNumber.of(3))) }
    check(parent.findings.any { it.source.pointer == "/components/schemas/Parent/minimum" })
    rejected<ValidationException> { Codecs.ambiguous.encode(Ambiguous.AsVariant1("overlap")) }
    rejected<ValidationException> { Codecs.ambiguous.decode("\"overlap\"") }
    val mixed = Codecs.mixed.decode("{\"name\":\"ok\",\"extra\":9007199254740993}")
    check(mixed is Mixed.AsConfiguration && mixed.value.name == "ok")
    check(Codecs.mixed.encode(mixed) == "{\"extra\":9007199254740993,\"name\":\"ok\"}")
    check(Codecs.mixed.encode(Codecs.mixed.decode("\"auto\"")) == "\"auto\"")
    val typed = TypedMap(additionalProperties = mapOf("a/b~😀" to null, "é" to JsonNumber.parse("1e-400"), "é" to JsonNumber.parse("1e99999999999999999999999999999")))
    val decoded = Codecs.typedMap.decode(Codecs.typedMap.encode(typed))
    check(decoded.additionalProperties.containsKey("a/b~😀") && decoded.additionalProperties["a/b~😀"] == null)
    check(decoded.additionalProperties["é"]!!.token == "1e-400")
    check(decoded.additionalProperties.size == 3)
    rejected<ValidationException> { Codecs.typedMap.decode("{\"extra\":false}") }
    val choices = mutableListOf<Split>(Split.AsVariant1("long"))
    val mutable = value.copy(choices = Presence.Present(choices))
    Codecs.value.encode(mutable)
    choices[0] = Split.AsVariant1("x")
    rejected<ValidationException> { Codecs.value.encode(mutable) }
    SchemaValidation.validate(Codecs.split.source, JsonString("long"), CodecLimits(maxEvaluationSteps = 10))
    rejected<EvaluationException> { Codecs.split.decode("\"long\"", CodecLimits(maxEvaluationSteps = 10)) }
    rejected<EvaluationException> { Codecs.split.encode(Split.AsVariant1("long"), CodecLimits(maxEvaluationSteps = 10)) }
    val huge = object : AbstractList<Split>() {
        override val size: Int get() = Int.MAX_VALUE
        override fun get(index: Int): Split = error("oversized native collection was traversed")
    }
    rejected<EvaluationException> { Codecs.value.encode(value.copy(choices = Presence.Present(huge))) }
    val hugeJson = object : AbstractList<JsonValue>() {
        override val size: Int get() = Int.MAX_VALUE
        override fun get(index: Int): JsonValue = error("oversized JSON collection was traversed")
    }
    rejected<EvaluationException> { Codecs.value.decodeJson(JsonObject(mapOf("required_nullable" to JsonNull, hostileKey to JsonArray(hugeJson)))) }
    val count = AtomicInteger()
    val recorded = mutableListOf<HttpRequest>()
    val transport = Transport { request ->
        count.incrementAndGet(); recorded.add(request)
        HttpResponse(200, mapOf("Content-Type" to listOf("application/json")), wire.toByteArray())
    }
    Client(Credentials(apiKey = "native-token"), transport).use { client ->
        val result = client.close2(__INPUT__(client = "a/b +😀", body2 = Presence.Present("query-body"), input = Presence.Present("query-input"), __COMPOSED__ = Presence.Present("one"), __DECOMPOSED__ = Presence.Present("two"), body = value))
        check(result.data.__HOSTILE_MEMBER__ == hostileText)
        check(recorded.single().url.toASCIIString() == "https://example.test/api/v1/echo/a%2Fb%20%2B%F0%9F%98%80?body=query-body&input=query-input&%C3%A9=one&e%CC%81=two")
        check(recorded.single().body!!.toString(Charsets.UTF_8) == wire)
        check(client.__HOSTILE_METHOD__().data.requiredNullable == null)
        val before = count.get()
        try { client.close2(__INPUT__(client = "x", body = value.copy(token = Presence.Present("wrong")))); error("invalid body sent") }
        catch (error: SdkException) { check(error.kind == FailureKind.REQUEST_VALIDATION && error.operationId == "close") }
        check(count.get() == before)
    }
    println("KOTLIN_ADVERSARIAL_UNIONS_PRESENCE_NAMES_AND_BUDGETS_PASSED")
}
