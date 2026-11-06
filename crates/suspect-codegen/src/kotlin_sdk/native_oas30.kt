package consumer

// Independent native consumer of the OpenAPI 3.0 petstore fixture
// (tests/fixtures/oas30/petstore-nullable.openapi.yaml). Expectations come from
// OAS 3.0.3: same-object nullable, Boolean exclusive bounds, and ignored
// Reference Object siblings. Wire bytes are hand-authored.

import example.sdk.*
import kotlinx.coroutines.runBlocking

private const val PET = """{"id":1,"name":"Rex","tag":null,"owner":{"name":"Ada"},"nickname":null,"metadata":null,"status":null,"weight":49.5,"anything":null}"""

private var checks = 0
private fun expect(condition: Boolean, message: String) {
    if (!condition) throw AssertionError(message)
    checks++
}

private inline fun <reified T : Throwable> rejected(block: () -> Unit): T {
    try { block() } catch (error: Throwable) {
        if (error !is T) throw AssertionError("expected ${T::class.simpleName}, got $error", error)
        return error
    }
    error("expected rejection")
}

private fun invalid(json: String): ValidationFinding =
    rejected<ValidationException> { Codecs.pet.decode(json) }.findings.first()

private fun nullable() {
    val pet = Codecs.pet.decode(PET)
    expect(pet.tag == null, "scalar nullable decodes null")
    expect(pet.nickname == Presence.Present(null), "nullable \$ref target decodes present null")
    expect(pet.metadata == Presence.Present(null), "object nullable decodes present null")
    expect(pet.status == Presence.Present(null), "enum listing null decodes null")
    expect(pet.anything == Presence.Present(JsonNull), "untyped nullable keeps JSON null")
    val encoded = Codecs.pet.encode(pet)
    expect(encoded.contains("\"tag\":null") && encoded.contains("\"metadata\":null") && encoded.contains("\"nickname\":null"), "present nulls re-encode: $encoded")
    val full = Codecs.pet.decode(PET.replace("\"tag\":null", "\"tag\":\"good\"").replace("\"metadata\":null", "\"metadata\":{\"note\":\"n\"}").replace("\"status\":null", "\"status\":\"sold\"").replace("\"nickname\":null", "\"nickname\":\"Rexy\""))
    expect(full.tag == "good" && full.metadata == Presence.Present(PetMetadata(note = Presence.Present("n"))) && full.status == Presence.Present(PetStatus.SOLD) && full.nickname == Presence.Present("Rexy"), "non-null nullable values")
    // nullable adds null only to the same-object type; other assertions still apply.
    expect(invalid(PET.replace("\"nickname\":null", "\"nickname\":\"\"")).source.pointer.endsWith("/components/schemas/Nickname/minLength"), "nullable target keeps minLength")
    invalid(PET.replace("\"status\":null", "\"status\":\"lost\""))
    invalid(PET.replace("\"name\":\"Rex\"", "\"name\":null"))
    expect(Codecs.nickname.decode("null") == null && Codecs.nickname.encode(null) == "null", "nullable component root")
}

private fun exclusiveBounds() {
    expect(invalid(PET.replace("\"weight\":49.5", "\"weight\":50")).source.pointer.endsWith("/components/schemas/Pet/properties/weight/maximum"), "exclusiveMaximum: true excludes 50")
    expect(Codecs.pet.decode(PET.replace("\"weight\":49.5", "\"weight\":49.999")).weight == Presence.Present(JsonNumber.parse("49.999")), "just below the exclusive maximum")
    expect(invalid(PET.replace("\"id\":1", "\"id\":0")).source.pointer.endsWith("/components/schemas/Pet/properties/id/minimum"), "exclusiveMinimum: true excludes 0")
    rejected<ValidationException> { Codecs.pet.encode(Codecs.pet.decode(PET).copy(weight = Presence.Present(JsonNumber.of(50)))) }
}

private fun referenceSiblings() {
    // owner: {$ref: Owner, nullable: true, type: string} applies only Owner.
    expect(invalid(PET.replace("\"owner\":{\"name\":\"Ada\"}", "\"owner\":null")).instancePath == "/owner", "ignored nullable sibling")
    invalid(PET.replace("\"owner\":{\"name\":\"Ada\"}", "\"owner\":\"Ada\""))
    expect(Codecs.pet.decode(PET).owner == Owner(name = "Ada"), "owner object")
}

private fun petstore() = runBlocking {
    val seen = mutableListOf<String>()
    val transport = Transport { request ->
        val body = request.body?.decodeToString() ?: ""
        seen += "${request.method} ${request.url} $body"
        val (status, payload) = when {
            request.url.path == "/v1/pets" && request.method == "GET" -> 200 to "[$PET]"
            request.url.path == "/v1/pets" -> 201 to body
            request.url.path == "/v1/pets/bad" -> 200 to PET.replace("\"weight\":49.5", "\"weight\":50")
            else -> 200 to PET
        }
        HttpResponse(status, mapOf("Content-Type" to listOf("application/json")), payload.encodeToByteArray())
    }
    Client(Credentials(), transport).use { client ->
        val page = client.listPets(ListPetsInput(limit = Presence.Present(JsonNumber.of(100))))
        expect(page is ListPetsResult.Status200 && page.data.single().tag == null && page.data.single().owner.name == "Ada" && seen.last().contains("?limit=100"), "typed list; inclusive maximum")
        client.listPets(ListPetsInput(limit = Presence.Present(JsonNumber.of(1))))
        expect(seen.last().contains("?limit=1"), "just above the exclusive minimum")
        val before = seen.size
        // exclusiveMinimum: true on the limit parameter rejects 0 before transport.
        expect(rejected<SdkException> { runBlocking { client.listPets(ListPetsInput(limit = Presence.Present(JsonNumber.of(0)))) } }.kind == FailureKind.REQUEST_VALIDATION, "limit 0 rejected")
        expect(seen.size == before, "request validation precedes transport")
        val created = client.createPet(CreatePetInput(Pet(id = JsonNumber.of(2), name = "Tom", tag = null, owner = Owner(name = "Bo"), nickname = Presence.Present(null))))
        expect(created.data.tag == null && created.data.nickname == Presence.Present(null), "round-tripped nullable body")
        expect(seen.last().endsWith("{\"id\":2,\"name\":\"Tom\",\"nickname\":null,\"owner\":{\"name\":\"Bo\"},\"tag\":null}"), "request bytes: ${seen.last()}")
        expect(rejected<SdkException> { runBlocking { client.createPet(CreatePetInput(Pet(id = JsonNumber.of(2), name = "Tom", tag = null, owner = Owner(name = "Bo"), weight = Presence.Present(JsonNumber.of(50))))) } }.kind == FailureKind.REQUEST_VALIDATION, "exclusive maximum on request")
        expect(client.showPetById(ShowPetByIdInput(petId = "1")).data.name == "Rex", "typed show")
        expect(rejected<SdkException> { runBlocking { client.showPetById(ShowPetByIdInput(petId = "bad")) } }.kind == FailureKind.RESPONSE_VALIDATION, "exclusive maximum on response")
    }
}

fun main() {
    nullable()
    exclusiveBounds()
    referenceSiblings()
    petstore()
    println("KOTLIN_OAS30_PASSED checks=$checks")
}
