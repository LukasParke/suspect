package example.scoped

/** Independent source-program expectations; executes the actual scoped kernel. */
public object GeneratedExamples {
    /** Run all maintained independent expectations under their compiled limits. */
    @JvmStatic public fun main(args: Array<String>) {
        val resource = GeneratedExamples::class.java.getResourceAsStream("vectors.json")!!.use { it.readBytes() }
        val suite = Json.parse(resource) as JsonObject
        val cases = suite.array("cases").map { it as JsonObject }
        check(cases.size == 32)
        val controls = suite.array("controls").map { it as JsonObject }
        for (case in cases + controls) {
            val program = ScopedProgram(case.obj("program"))
            val root = program.roots.entries.single()
            val value = Json.parse(case.text("instanceJson"))
            var findings = emptyList<ValidationFinding>()
            val result = try { ValidationSession(CodecLimits(), {}, program).validate(root.value, value, ""); "Valid" }
            catch (failure: ValidationException) { findings = failure.findings; "Invalid" }
            catch (failure: EvaluationException) { findings = listOf(failure.finding); "EvaluationFailure" }
            check(result == case.text("expected")) { "${case.text("id")}: expected ${case.text("expected")}, got $result: $findings" }
            val source = (case.values["source"] as? JsonString)?.value
            if (source != null) check(findings.any { it.source.document == root.key.document && it.source.pointer == source && it.instancePath == case.text("instancePath") }) {
                "${case.text("id")}: expected $source at ${case.text("instancePath")}: $findings"
            }
            (case.values["findingsCount"] as? JsonNumber)?.let { check(findings.size.toLong() == it.toLongExact()) }
            println("SCOPED_CASE=${case.text("id")}:$result")
        }
        println("KOTLIN_SCOPED_32_PASSED")
        println("KOTLIN_SCOPED_CONTROLS=${controls.size}")
        for ((index, raw) in suite.array("invalidPrograms").withIndex()) {
            var rejected = false
            try { ScopedProgram(raw as JsonObject) } catch (_: Exception) { rejected = true }
            check(rejected) { "malformed native program $index was admitted" }
        }
        println("KOTLIN_SCOPED_NATIVE_GUARDS=${suite.array("invalidPrograms").size}")
        val recursion = ScopedProgram(cases.single { it.text("id") == "recursive-ref-annotations-with-instance-progress" }.obj("program"))
        val nested = Json.parse(buildString { repeat(100) { append("{\"next\":") }; append("{}"); repeat(100) { append('}') } })
        val failure = java.util.concurrent.atomic.AtomicReference<Throwable?>()
        val worker = Thread(null, Runnable {
            try { ValidationSession(CodecLimits(), {}, recursion).validate(recursion.roots.values.single(), nested, "") }
            catch (error: Throwable) { failure.set(error) }
        }, "scoped-depth", 2L * 1024 * 1024)
        worker.isDaemon = true
        worker.start(); worker.join(5000)
        check(!worker.isAlive && failure.get() is EvaluationException) { "bounded recursion did not fail cleanly: ${failure.get()}" }
        for (id in listOf("if-then-selected", "selected-branch-failure-survives-anyof", "contains-marks-all-matches", "not-discards-annotations")) {
            val case = cases.single { it.text("id") == id }
            val program = ScopedProgram(case.obj("program"))
            val cancelled = kotlinx.coroutines.CancellationException("native scoped cancellation")
            var checkpoints = 0
            var observed: Throwable? = null
            try { ValidationSession(CodecLimits(), { if (++checkpoints == 4) throw cancelled }, program).validate(program.roots.values.single(), Json.parse(case.text("instanceJson")), "") }
            catch (error: Throwable) { observed = error }
            check(observed === cancelled) { "$id suppressed or replaced cancellation: $observed" }
        }
        println("KOTLIN_SCOPED_DEPTH_AND_CANCELLATION_PASSED")
    }
}
