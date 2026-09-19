package consumer

import example.sdk.*

private inline fun <reified T : Throwable> rejected(block: () -> Unit): T {
    try { block() } catch (error: Throwable) { check(error is T) { "expected ${T::class.simpleName}, got $error" }; return error }
    error("expected rejection")
}

fun main() {
    val bytes = object {}.javaClass.getResourceAsStream("/vectors.json")!!.use { it.readAllBytes() }
    val corpus = Json.parse(bytes) as JsonObject
    val cases = (corpus.values.getValue("cases") as JsonArray).values
    check(cases.size == 17)
    fun root(name: String) = SchemaValidation.sources().single { it.pointer == "/components/schemas/$name" }
    var valid = 0
    var invalid = 0
    for ((index, raw) in cases.withIndex()) {
        val case = (raw as JsonObject).values
        val name = (case.getValue("name") as JsonString).value
        for (entry in (case.getValue("valid") as JsonArray).values) {
            try { SchemaValidation.validate(root("Case$index"), Json.parse((entry as JsonString).value)) }
            catch (error: Exception) { throw AssertionError("valid vector $name failed", error) }
            valid++
        }
        for (entry in (case.getValue("invalid") as JsonArray).values) {
            val issue = rejected<ValidationException> { SchemaValidation.validate(root("Case$index"), Json.parse((entry as JsonString).value)) }
            check(issue.findings.isNotEmpty() && issue.findings.first().source.pointer.startsWith("/components/schemas/Case$index"))
            invalid++
        }
    }
    for (name in listOf("Cycle", "UnionFailure", "NotFailure")) rejected<EvaluationException> { SchemaValidation.validate(root(name), JsonNull) }
    rejected<EvaluationException> { SchemaValidation.validate(root("EqualityFailure"), JsonString("x"), CodecLimits(maxEqualitySteps = 0)) }
    rejected<EvaluationException> { SchemaValidation.validate(root("PatternFailure"), JsonString("aaaaaaab"), CodecLimits(maxEvaluationSteps = 16)) }
    rejected<EvaluationException> { SchemaValidation.validate(root("Case0"), JsonNumber.of(1), CodecLimits(maxEvaluationSteps = 0)) }
    rejected<EvaluationException> { SchemaValidation.validate(root("Case5"), Json.parse("{\"a\":[1,null,true]}"), CodecLimits(maxEqualitySteps = 1)) }
    rejected<EvaluationException> { SchemaValidation.validate(root("Case11"), JsonString("😀x"), CodecLimits(maxValidationBytes = 1)) }
    for (token in listOf("1e" + "0".repeat(41), "10e-" + "0".repeat(40) + "1", "1e999999999999999999999999999999999")) {
        SchemaValidation.validate(root("Case1"), JsonNumber.parse(token))
    }
    rejected<ValidationException> { SchemaValidation.validate(root("Case1"), JsonNumber.parse("0.1e" + "0".repeat(41))) }
    SchemaValidation.validate(root("Case4"), JsonNumber.parse("3e999999999999999999999999999999999"))
    SchemaValidation.validate(root("Case2"), JsonNumber.parse("125e-" + "0".repeat(40) + "1"))
    SchemaValidation.validate(root("Case13"), Json.parse("[\"tuple\",1.0,1e3,9007199254740993]"))
    rejected<EvaluationException> { SchemaValidation.validate(SourceLocation("https://unknown.test/", ""), JsonNull) }
    println("KOTLIN_SHARED_VECTORS=17 valid=$valid invalid=$invalid resource-controls=passed")
}
