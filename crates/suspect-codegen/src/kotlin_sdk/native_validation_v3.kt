package example.resources

/** Unmodified official expectations executed by the native indexed kernel. */
public object GeneratedExamples {
    /** Run source-compiled V3 programs with no acquisition. */
    @JvmStatic public fun main(args: Array<String>) {
        val suite = Json.parse(GeneratedExamples::class.java.getResourceAsStream("vectors.json")!!.use { it.readBytes() }) as JsonObject
        val cases = suite.array("cases").map { it as JsonObject }
        check(cases.size == 44)
        val controls = (suite.values["controls"] as? JsonArray)?.values?.map { it as JsonObject } ?: emptyList()
        for (case in cases + controls) {
            val program = ScopedProgram(case.obj("program"))
            var details = emptyList<ValidationFinding>()
            val result = try { ValidationSession(CodecLimits(), {}, program).validate(case.number("rootTarget"), Json.parse(case.text("instanceJson")), ""); "Valid" }
            catch (failure: ValidationException) { details = failure.findings; "Invalid" }
            catch (failure: EvaluationException) { details = listOf(failure.finding); "EvaluationFailure" }
            check(result == case.text("expected")) { "${case.text("id")}: $result instead of ${case.text("expected")}: $details" }
            if (result == "Invalid") check(details.isNotEmpty())
            (case.values["source"] as? JsonString)?.value?.let { expected -> check(details.any { it.source.document == "https://physical.test/api.json" && it.source.pointer == expected && it.instancePath == case.text("instancePath") }) { "wrong physical failure location: $details" } }
            println("RESOURCE_CASE=${case.text("id")}:$result")
        }
        println("KOTLIN_RESOURCE_44_PASSED")
        println("KOTLIN_RESOURCE_CONTROLS=${controls.size}")
        for (raw in (suite.values["invalidPrograms"] as? JsonArray)?.values ?: emptyList()) {
            var failed = false
            try { ScopedProgram(raw as JsonObject) } catch (_: Exception) { failed = true }
            check(failed) { "malformed resource metadata was admitted" }
        }
        println("KOTLIN_RESOURCE_PROGRAM_GUARDS_PASSED")
        val deep = ScopedProgram(suite.obj("deep"))
        val failure = java.util.concurrent.atomic.AtomicReference<Throwable?>()
        val worker = Thread(null, Runnable {
            try { ValidationSession(CodecLimits(), {}, deep).validate(deep.roots.values.single(), JsonString("value"), "") }
            catch (error: Throwable) { failure.set(error) }
        }, "resource-depth", 2L * 1024 * 1024)
        worker.isDaemon = true; worker.start(); worker.join(5000)
        check(!worker.isAlive && failure.get() is EvaluationException) { "resource-depth failure was not bounded: ${failure.get()}" }
        val nodes = deep.nodes.indices.associateBy { deep.resources.resource(it) }
        val scope = ResourceScope(deep.resources)
        val parent = scope.enter(nodes.getValue(2)) {}
        val nested = scope.enter(nodes.getValue(0)) {}
        val first = scope.context
        scope.leave(nested); scope.leave(parent)
        val collision = scope.enter(nodes.getValue(31)) {}
        check(first != scope.context) { "context identities collapsed a Pair hash collision" }
        scope.leave(collision); check(scope.context == 0)
        val trial = controls.single { it.text("id") == "fresh-context-and-trials-3" }
        val program = ScopedProgram(trial.obj("program"))
        val cancelled = kotlinx.coroutines.CancellationException("resource cancellation")
        var checkpoints = 0
        val session = ValidationSession(CodecLimits(), { if (++checkpoints == 10) throw cancelled }, program)
        val instance = Json.parse(trial.text("instanceJson"))
        var observed: Throwable? = null
        try { session.validate(trial.number("rootTarget"), instance, "") } catch (error: Throwable) { observed = error }
        check(observed === cancelled)
        session.validate(trial.number("rootTarget"), instance, "")
        println("KOTLIN_RESOURCE_DEPTH_CONTEXT_CANCELLATION_PASSED")
    }
}
