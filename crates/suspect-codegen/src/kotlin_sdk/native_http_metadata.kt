package __PACKAGE__

/** JDK metadata stays outside ordinary HTTP field validation. */
public object HttpMetadataRegression {
    /** Exercise the same normalizer used by buffered and streaming JDK responses. */
    @JvmStatic public fun main(args: Array<String>) {
        val raw = linkedMapOf(":status" to listOf("401"), "content-type" to listOf("application/json"))
        val headers = jdkResponseHeaders(raw)
        check(headers.size == 1 && headers[":status"] == null && !headers.containsKey(":status"))
        check(headers["content-type"] == listOf("application/json"))
        check(!boundedHeaders(headers).malformed)
        check(boundedHeaders(raw).malformed)
        check(boundedHeaders(jdkResponseHeaders(mapOf(":unknown" to listOf("x")))).malformed)
        check(boundedHeaders(jdkResponseHeaders(raw + (0..128).associate { "x-$it" to emptyList<String>() })).limited)
        println("KOTLIN_JDK_METADATA_REGRESSION_PASSED")
    }
}
