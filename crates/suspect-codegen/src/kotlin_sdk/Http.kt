package __PACKAGE__

import java.io.ByteArrayOutputStream
import java.net.Proxy
import java.net.ProxySelector
import java.net.SocketAddress
import java.net.URI
import java.net.http.HttpClient as JdkClient
import java.net.http.HttpRequest as JdkRequest
import java.net.http.HttpResponse as JdkResponse
import java.nio.ByteBuffer
import java.time.Duration
import java.util.concurrent.CompletableFuture
import java.util.concurrent.CompletionException
import java.util.concurrent.CompletionStage
import java.util.concurrent.Flow
import java.util.concurrent.atomic.AtomicReference
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.ensureActive
import kotlinx.coroutines.suspendCancellableCoroutine
import kotlinx.coroutines.withTimeoutOrNull
import kotlinx.coroutines.withContext
import kotlin.coroutines.CoroutineContext
import kotlin.coroutines.resume
import kotlin.coroutines.resumeWithException

/** Transport-neutral prepared request. Do not mutate its collections or bytes.
 * @property method Source HTTP method.
 * @property url Fully encoded absolute URI.
 * @property headers Prepared headers, including explicit runtime credentials.
 * @property body Source-encoded body bytes, or null for an omitted body.
 * @property timeout Total operation timeout. The enclosing coroutine enforces it.
 * @property maxResponseBytes Finite body ceiling the transport must respect.
 */
public class HttpRequest(
    /** Exact source HTTP method. */
    public val method: String,
    /** Fully encoded absolute request URI. */
    public val url: URI,
    /** Prepared request headers, including explicit credentials. */
    public val headers: Map<String, String>,
    /** Source-encoded bytes, or null when the body is omitted. */
    public val body: ByteArray?,
    /** Total operation timeout enforced by the enclosing coroutine. */
    public val timeout: Duration,
    /** Finite response-body ceiling. */
    public val maxResponseBytes: Int,
)

/** Completed transport response. A custom transport must bound I/O before
 * returning this value; the client independently checks all returned limits.
 * @property status HTTP status code.
 * @property headers All header values; names are case-insensitive.
 * @property body Captured raw body bytes.
 * @property truncated True if transport collection was stopped before the body ended.
 */
public class HttpResponse(
    /** HTTP status code. */
    public val status: Int,
    /** All response header values. */
    public val headers: Map<String, List<String>>,
    /** Captured raw body bytes. */
    public val body: ByteArray,
    /** Whether collection ended before the body completed. */
    public val truncated: Boolean = false,
)

/** Native suspend transport injection. Implementations participate in the
 * caller's coroutine, stop I/O on cancellation, and release response resources
 * before returning or throwing. No detached SDK coroutine is created.
 */
public fun interface Transport {
    /** Perform one exchange within the prepared limits and coroutine lifetime. */
    public suspend fun execute(request: HttpRequest): HttpResponse
}

/** A transport failure after response headers, with an optional bounded prefix.
 * @property response Available raw response; client capture policy is still applied.
 */
public class TransportException(/** Available response prefix, still subject to client limits. */ public val response: HttpResponse?, cause: Throwable) : RuntimeException("HTTP I/O failed", cause)

/** Bounded response metadata, shared by successful results and SDK failures.
 * @property status HTTP status code.
 * @property headers Header snapshot, at most 32 KiB and 128 fields/values.
 * @property bodyPreview Independent prefix, bounded by captureBytes.
 * @property truncated Whether either capture was truncated.
 * @property headersTruncated Whether the header snapshot was incomplete.
 */
public class ResponseInfo(
    /** HTTP status code. */
    public val status: Int,
    /** Finite header snapshot. */
    public val headers: Map<String, List<String>>,
    /** Independent bounded raw-body prefix. */
    public val bodyPreview: ByteArray,
    /** Whether either capture is incomplete. */
    public val truncated: Boolean,
    /** Whether the header snapshot is incomplete. */
    public val headersTruncated: Boolean = false,
    /** Selected concrete media type, when one applies. */ public val contentType: String? = null,
    /** Source-declared links, retained as metadata only. */ public val links: List<LinkMetadata> = emptyList(),
) {
    internal fun withProtocol(type: String?, links: List<LinkMetadata>): ResponseInfo =
        ResponseInfo(status, headers, bodyPreview, truncated, headersTruncated, type, links)
}

/** Client runtime policy, independent of source semantics.
 * @property serverUrl Explicit HTTPS environment or loopback HTTP fixture override.
 * @property timeout Total deadline, positive and at most one day.
 * @property maxResponseBytes Response-body ceiling, from 1 byte through 4 MiB.
 * @property captureBytes Preview ceiling, from zero through maxResponseBytes.
 * @property maxRequestBytes Ceiling for each assembled URL and JSON request body.
 * @property codecLimits Per-codec shared validation and native traversal policy.
 */
public data class ClientOptions(
    /** Explicit HTTPS environment or loopback HTTP override. */
    public val serverUrl: URI? = null,
    /** Positive total operation timeout, at most one day. */
    public val timeout: Duration = Duration.ofSeconds(30),
    /** Finite response-body ceiling. */
    public val maxResponseBytes: Int = 4 * 1024 * 1024,
    /** Raw-body preview ceiling. */
    public val captureBytes: Int = minOf(8192, maxResponseBytes),
    /** Ceiling for each assembled URL and JSON request body. */
    public val maxRequestBytes: Int = 4 * 1024 * 1024,
    /** Shared per-codec conversion and evaluation policy. */
    public val codecLimits: CodecLimits = CodecLimits(),
    /** Source server candidate index. */ public val serverIndex: Int = 0,
    /** Literal source server variable overrides. */ public val serverVariables: Map<String, String> = emptyMap(),
    /** Retrieval URL base for relative servers loaded from local files. */ public val documentUrl: URI? = null,
    /** Maximum raw transport chunk. */ public val maxChunkBytes: Int = 65536,
    /** Maximum buffered SSE/JSON-lines item. */ public val maxStreamItemBytes: Int = 1024 * 1024,
    /** Complete User-Agent override; an explicit empty value suppresses the automatic ua/v1 attribution header entirely. */
    public val userAgent: String? = null,
    /** Replaces the SDK identity token in the automatic ua/v1 attribution header: `name` or `name/version` of RFC 9110 tokens; an invalid identifier suppresses the header. */
    public val applicationId: String? = null,
) {
    init {
        require(maxResponseBytes in 1..Json.MAX_BYTES && maxRequestBytes in 1..Json.MAX_BYTES) { "HTTP byte ceilings must be in 1..4194304" }
        require(captureBytes in 0..maxResponseBytes) { "captureBytes must be in 0..maxResponseBytes" }
        timeoutMillis(timeout)
        require(serverIndex >= 0 && maxChunkBytes in 1..65536 && maxStreamItemBytes in 1..Json.MAX_BYTES)
        serverUrl?.let(::validateServer)
    }
}

/** Per-call runtime options; caller cancellation uses ordinary coroutine context.
 * @property timeout Override the total client deadline for this call.
 */
public data class RequestOptions(
    /** Optional total deadline override. */ public val timeout: Duration? = null,
    /** Optional source server choice. */ public val serverIndex: Int? = null,
    /** Per-call server variables; null inherits the client map. */ public val serverVariables: Map<String, String>? = null,
    /** Per-call HTTP document URL for relative server resolution. */ public val documentUrl: URI? = null,
    /** Explicit source OR-security alternative. */ public val securityAlternative: Int? = null,
    /** Concrete request Content-Type for a declared media/range. */ public val requestMedia: String? = null,
    /** Select and require a successful response representation. */ public val responseMedia: String? = null,
) {
    init { timeout?.let(::timeoutMillis) }
}

/** Stable SDK failure categories. Declared API failures have operation-specific types. */
public enum class FailureKind {
    /** Required source credential is missing or malformed. */ AUTHENTICATION,
    /** Native input violates its source schema or JSON representation. */ REQUEST_VALIDATION,
    /** URL or another wire representation cannot be constructed. */ REQUEST_REPRESENTATION,
    /** The URL/request body construction ceiling was exceeded. */ REQUEST_LIMIT,
    /** Response JSON violates its declared source schema. */ RESPONSE_VALIDATION,
    /** Schema, JSON or native conversion could not complete within its limits. */ EVALUATION,
    /** I/O or custom transport cleanup failed. Causes are explicitly accessible. */ TRANSPORT,
    /** The configured operation deadline expired. Caller cancellation propagates separately. */ TIMEOUT,
    /** Response status is not declared. */ UNEXPECTED_STATUS,
    /** Response media type, charset or content encoding is outside its declaration. */ CONTENT_TYPE,
    /** Response body or headers exceed capture limits. */ RESPONSE_LIMIT,
    /** Transport returned malformed HTTP status or header metadata. */ RESPONSE_METADATA,
}

/** SDK failure with a safe message, original cause, source and bounded capture.
 * @property kind Stable failure category.
 * @property response Available bounded response metadata.
 * @property operationId Stable source operation ID, when called through Client.
 * @property source Original operation identity, when called through Client.
 */
public class SdkException(
    /** Stable failure category. */
    public val kind: FailureKind,
    message: String,
    /** Bounded response information when available. */
    public val response: ResponseInfo? = null,
    cause: Throwable? = null,
    /** Original operation ID. */
    public val operationId: String? = null,
    /** Original operation source identity. */
    public val source: SourceLocation? = null,
) : RuntimeException(message, cause)

/** Base for source-declared, operation-specific typed API failures.
 * @property operationId Original operation ID.
 * @property source Original operation identity.
 * @property response Bounded response metadata.
 */
public abstract class ApiException internal constructor(
    /** Original operation ID. */
    public val operationId: String,
    /** Original operation source identity. */
    public val source: SourceLocation,
    /** Bounded HTTP response information. */
    public val response: ResponseInfo,
) : RuntimeException("declared HTTP API failure")

internal class CallContext(private val context: CoroutineContext, timeout: Duration) {
    private val start = System.nanoTime()
    private val nanos = timeout.toNanos()
    var response: ResponseInfo? = null
    fun check() {
        context.ensureActive()
        if (System.nanoTime() - start >= nanos) throw SdkException(FailureKind.TIMEOUT, "operation deadline expired", response)
    }
}

internal suspend fun <T : Any> operation(id: String, source: SourceLocation, timeout: Duration, block: suspend (CallContext) -> T): T {
    currentCoroutineContext().ensureActive()
    var control: CallContext? = null
    try {
        return withTimeoutOrNull(timeoutMillis(timeout)) {
            // Keep CPU-bound validation off a caller's single-thread event loop,
            // so its timeout/cancellation scheduler can continue to run.
            withContext(Dispatchers.Default) {
                val active = CallContext(currentCoroutineContext(), timeout)
                control = active
                active.check()
                block(active).also { active.check() }
            }
        } ?: throw SdkException(FailureKind.TIMEOUT, "operation deadline expired", control?.response)
    } catch (cancelled: CancellationException) { throw cancelled }
    catch (error: SdkException) {
        currentCoroutineContext().ensureActive()
        val wrapped = SdkException(error.kind, error.message ?: "SDK operation failed", error.response, error.cause, id, source)
        error.suppressed.forEach(wrapped::addSuppressed)
        throw wrapped
    }
}

/** JDK 21 asynchronous adapter with structured coroutine cancellation.
 *
 * Redirects, cookies, automatic credential acquisition and implicit proxies are
 * disabled. The SDK performs one exchange with no retry loop. Each body
 * subscription retains at most maxResponseBytes + 1 bytes, then cancels upstream.
 */
public class JdkTransport : StreamingTransport, AutoCloseable {
    private val client: JdkClient = JdkClient.newBuilder()
        .followRedirects(JdkClient.Redirect.NEVER)
        .connectTimeout(Duration.ofSeconds(10))
        .proxy(object : ProxySelector() {
            override fun select(uri: URI): List<Proxy> = listOf(Proxy.NO_PROXY)
            override fun connectFailed(uri: URI, sa: SocketAddress, ioe: java.io.IOException) = Unit
        }).build()
    private val streaming by lazy { JdkStreaming(client) }

    /** Open a cancellable response reader; its collector owns and closes the body. */
    override suspend fun open(request: HttpRequest): StreamingResponse = streaming.open(request)

    /** Send asynchronously; cancellation stops both the future and body subscription. */
    override suspend fun execute(request: HttpRequest): HttpResponse {
        currentCoroutineContext().ensureActive()
        require(request.maxResponseBytes in 1..Json.MAX_BYTES) { "invalid response ceiling" }
        val builder = JdkRequest.newBuilder(request.url)
        for ((key, value) in request.headers) builder.header(key, value)
        builder.method(request.method, request.body?.let(JdkRequest.BodyPublishers::ofByteArray) ?: JdkRequest.BodyPublishers.noBody())
        return suspendCancellableCoroutine { continuation ->
            val subscriber = AtomicReference<BoundedBody?>()
            val future = client.sendAsync(builder.build(), JdkResponse.BodyHandler { info ->
                val headers = boundedHeaders(jdkResponseHeaders(info.headers().map()))
                BoundedBody(if (headers.limited || headers.malformed || bodyForbidden(request.method, info.statusCode())) 0 else request.maxResponseBytes + 1).also {
                    subscriber.set(it)
                    if (!continuation.isActive) it.cancel()
                }
            })
            continuation.invokeOnCancellation { subscriber.get()?.cancel(); future.cancel(true) }
            future.whenComplete { response, error ->
                if (error != null) {
                    continuation.resumeWithException(if (error is CompletionException) error.cause ?: error else error)
                } else {
                    val body = response.body()
                    val captured = HttpResponse(response.statusCode(), jdkResponseHeaders(response.headers().map()), body.bytes, body.limited)
                    if (body.failure == null) continuation.resume(captured)
                    else continuation.resumeWithException(TransportException(captured, body.failure))
                }
            }
        }
    }

    /** Cancel outstanding work and release this adapter's connections. */
    override fun close() { streaming.close(); client.shutdownNow() }

    private class BodyResult(val bytes: ByteArray, val limited: Boolean, val failure: Throwable? = null)
    private class BoundedBody(private val maximum: Int) : JdkResponse.BodySubscriber<BodyResult> {
        private val completed = CompletableFuture<BodyResult>()
        private val bytes = ByteArrayOutputStream(minOf(maximum, 8192))
        private var subscription: Flow.Subscription? = null
        override fun getBody(): CompletionStage<BodyResult> = completed
        @Synchronized fun cancel() { subscription?.cancel(); completed.cancel(true) }
        @Synchronized override fun onSubscribe(value: Flow.Subscription) {
            if (subscription != null || completed.isDone) value.cancel()
            else {
                subscription = value
                if (maximum == 0) { value.cancel(); completed.complete(BodyResult(byteArrayOf(), false)) }
                else value.request(1)
            }
        }
        @Synchronized override fun onNext(items: List<ByteBuffer>) {
            if (completed.isDone) return
            for (item in items) {
                val count = minOf(item.remaining(), maximum - bytes.size())
                if (count > 0) {
                    val chunk = ByteArray(count)
                    item.get(chunk); bytes.write(chunk)
                }
                if (bytes.size() >= maximum) {
                    subscription?.cancel()
                    completed.complete(BodyResult(bytes.toByteArray(), true))
                    return
                }
            }
            subscription?.request(1)
        }
        @Synchronized override fun onError(error: Throwable) {
            subscription?.cancel()
            completed.complete(BodyResult(bytes.toByteArray(), true, error))
        }
        @Synchronized override fun onComplete() { completed.complete(BodyResult(bytes.toByteArray(), false)) }
    }
}

internal fun timeoutMillis(timeout: Duration): Long {
    require(!timeout.isNegative && !timeout.isZero && timeout <= Duration.ofDays(1)) { "timeout must be positive and at most one day" }
    // Round up, preserving positive sub-millisecond deadlines (also checked by nanoTime).
    return (timeout.toNanos() + 999999) / 1000000
}

internal fun validateServer(server: URI) {
    val loopback = server.host in setOf("127.0.0.1", "[::1]", "::1", "localhost")
    require(server.isAbsolute && server.host != null && server.rawUserInfo == null && server.rawQuery == null && server.rawFragment == null
        && server.port in -1..65535 && (server.scheme.equals("https", true) || server.scheme.equals("http", true) && loopback)) {
        "server must be absolute HTTPS or loopback HTTP without userinfo, query or fragment"
    }
    val decoded = server.path ?: ""
    require(!decoded.contains('\\') && decoded.none { it < ' ' || it == '\u007f' }
        && decoded.split('/').none { it == "." || it == ".." }) { "server path must not contain decoded dot segments, controls or backslashes" }
}

internal fun bearer(token: String?): String {
    if (token == null || token.length > 8192 || !Regex("[A-Za-z0-9._~+/-]+=*").matches(token)) {
        throw SdkException(FailureKind.AUTHENTICATION, "missing or malformed bearer credential")
    }
    return "Bearer $token"
}

internal fun scalar(value: JsonValue): String = when (value) {
    is JsonString -> value.value
    is JsonNumber -> value.token
    is JsonBoolean -> value.value.toString()
    else -> throw SdkException(FailureKind.REQUEST_REPRESENTATION, "wire parameter is not a scalar")
}

internal class WireUrl(server: URI, path: String, parameters: Map<String, String>, private val maximum: Int, private val checkpoint: () -> Unit) {
    private val out = StringBuilder()
    private var hasQuery = false
    init {
        validateServer(server)
        append(server.toASCIIString().trimEnd('/'))
        var position = 0
        while (position < path.length) {
            val open = path.indexOf('{', position)
            if (open < 0) { append(path.substring(position)); break }
            append(path.substring(position, open))
            val close = path.indexOf('}', open)
            if (close < 0) throw SdkException(FailureKind.REQUEST_REPRESENTATION, "invalid path template")
            val value = parameters[path.substring(open + 1, close)]
                ?: throw SdkException(FailureKind.REQUEST_REPRESENTATION, "missing path parameter")
            percent(value, segment = true)
            position = close + 1
        }
    }
    private fun append(value: String) {
        checkpoint()
        if (value.length > maximum - out.length) throw SdkException(FailureKind.REQUEST_LIMIT, "request URL byte limit exceeded")
        out.append(value)
    }
    private fun percent(value: String, segment: Boolean = false) {
        // The request URL budget owns this classification: resource
        // exhaustion while sizing a URL field is a request limit, not a JSON
        // evaluation failure.
        try { Json.utf8Size(value, maximum, checkpoint) } catch (error: JsonException) {
            if (error.kind == JsonErrorKind.RESOURCE_LIMIT) throw SdkException(FailureKind.REQUEST_LIMIT, "request URL byte limit exceeded", cause = error) else throw error }
        val dotSegment = segment && (value == "." || value == "..")
        for (byte in value.toByteArray(Charsets.UTF_8)) {
            val b = byte.toInt() and 255
            if (b in 65..90 || b in 97..122 || b in 48..57 || b == 45 || b == 95 || b == 126 || b == 46 && !dotSegment) append(b.toChar().toString())
            else append("%" + "0123456789ABCDEF"[b ushr 4] + "0123456789ABCDEF"[b and 15])
        }
    }
    private fun pair(name: String) {
        append(if (hasQuery) "&" else "?")
        hasQuery = true
        percent(name); append("=")
    }
    fun query(name: String, value: JsonValue, explode: Boolean) {
        if (value is JsonArray) {
            if (value.values.isEmpty()) return
            if (explode) for (item in value.values) { pair(name); percent(scalar(item)) }
            else {
                pair(name)
                value.values.forEachIndexed { i, item -> if (i > 0) append(","); percent(scalar(item)) }
            }
        } else { pair(name); percent(scalar(value)) }
    }
    fun uri(): URI = try { URI(out.toString()) }
    catch (error: java.net.URISyntaxException) { throw SdkException(FailureKind.REQUEST_REPRESENTATION, "invalid assembled URL", cause = error) }
}

internal inline fun <T> requestValue(block: () -> T): T = try { block() }
catch (error: ValidationException) { throw SdkException(FailureKind.REQUEST_VALIDATION, "request schema validation failed", cause = error) }
catch (error: JsonException) { throw SdkException(if (error.kind == JsonErrorKind.RESOURCE_LIMIT) FailureKind.EVALUATION else FailureKind.REQUEST_VALIDATION, "request JSON encoding failed", cause = error) }
catch (error: EvaluationException) { throw SdkException(FailureKind.EVALUATION, "request validation could not complete", cause = error) }
catch (error: IllegalArgumentException) { throw SdkException(FailureKind.REQUEST_REPRESENTATION, "request representation failed", cause = error) }

internal class HeaderSnapshot(val values: Map<String, List<String>>, val limited: Boolean, val malformed: Boolean)
// JDK HTTP/2 may include :status in HttpHeaders. It is transport metadata, not
// an HTTP field. Filter only at the JDK boundary and keep a lazy view so hostile
// header collections still reach boundedHeaders before copying or allocation.
internal fun jdkResponseHeaders(input: Map<String, List<String>>): Map<String, List<String>> {
    if (!input.containsKey(":status")) return input
    return object : kotlin.collections.AbstractMap<String, List<String>>() {
        override fun get(key: String): List<String>? = if (key == ":status") null else input[key]
        override fun containsKey(key: String): Boolean = key != ":status" && input.containsKey(key)
        override val entries: Set<Map.Entry<String, List<String>>> = object : kotlin.collections.AbstractSet<Map.Entry<String, List<String>>>() {
            override val size: Int get() = input.size - 1
            override fun iterator(): Iterator<Map.Entry<String, List<String>>> = input.entries.asSequence().filter { it.key != ":status" }.iterator()
        }
    }
}
internal fun boundedHeaders(input: Map<String, List<String>>): HeaderSnapshot {
    var remaining = 32768
    var count = 0
    var malformed = false
    val headers = linkedMapOf<String, List<String>>()
    for ((key, values) in input) {
        if (key.length > remaining || count >= 128) return HeaderSnapshot(headers, true, malformed)
        remaining -= key.length
        count++ // Empty value lists still consume a header slot.
        if (key.isEmpty() || key.any { !it.isLetterOrDigit() && it !in "!#$%&'*+-.^_`|~" } || key.any { it.code > 127 }) malformed = true
        val kept = mutableListOf<String>()
        for ((index, value) in values.withIndex()) {
            if (value.length > remaining || index > 0 && count >= 128) {
                headers[key] = kept.toList()
                return HeaderSnapshot(headers, true, malformed)
            }
            remaining -= value.length
            if (index > 0) count++
            if (value.any { it == '\u007f' || it < ' ' && it != '\t' || it.code > 255 }) malformed = true
            kept.add(value)
        }
        headers[key] = kept.toList()
    }
    return HeaderSnapshot(headers.toMap(), false, malformed)
}

internal fun capture(response: HttpResponse, maximum: Int): ResponseInfo {
    val headers = boundedHeaders(response.headers)
    return ResponseInfo(response.status, headers.values, response.body.copyOf(minOf(maximum, response.body.size)),
        response.truncated || response.body.size > maximum || headers.limited, headers.limited)
}

internal suspend fun exchange(transport: Transport, request: HttpRequest, captureBytes: Int, control: CallContext): Pair<HttpResponse, ResponseInfo> {
    control.check()
    val response = try { transport.execute(request) }
    catch (cancelled: CancellationException) { throw cancelled }
    catch (error: Exception) {
        // A failing finally/closer must not replace caller cancellation or the SDK deadline.
        control.check()
        if (error is SdkException) throw error
        val info = (error as? TransportException)?.response?.let { capture(it, captureBytes) }
        throw SdkException(FailureKind.TRANSPORT, "HTTP transport failed", info, error)
    }
    val received = if (bodyForbidden(request.method, response.status)) HttpResponse(response.status, response.headers, byteArrayOf()) else response
    val info = capture(received, captureBytes)
    control.response = info
    control.check()
    val headers = boundedHeaders(response.headers)
    if (received.truncated || received.body.size > request.maxResponseBytes || headers.limited) {
        throw SdkException(FailureKind.RESPONSE_LIMIT, "response capture limit exceeded", info)
    }
    if (headers.malformed || response.status !in 100..599) throw SdkException(FailureKind.RESPONSE_METADATA, "malformed response metadata", info)
    return received to info
}

internal fun bodyForbidden(method: String, status: Int): Boolean =
    method == "HEAD" || status in 100..199 || status in listOf(204, 205, 304)

private fun jsonMedia(value: String): Boolean {
    val parts = value.split(';')
    if (!parts[0].trim().equals("application/json", true)) return false
    var charset = false
    for (part in parts.drop(1)) {
        val pair = part.trim().split('=', limit = 2)
        if (pair.size != 2 || pair[0].trim().isEmpty()) return false
        val raw = pair[1].trim()
        val parameter = if (raw.startsWith('"') && raw.endsWith('"') && raw.length >= 2) raw.substring(1, raw.length - 1) else raw
        if (parameter.isEmpty() || parameter.any { it <= ' ' || it in "\"\\,;" }) return false
        if (pair[0].trim().equals("charset", true)) {
            if (charset || !parameter.equals("utf-8", true)) return false
            charset = true
        }
    }
    return true
}

internal fun <T> responseValue(codec: ModelCodec<T>, response: HttpResponse, info: ResponseInfo, limits: CodecLimits, checkpoint: () -> Unit): T {
    val types = response.headers.entries.filter { it.key.equals("content-type", true) }.flatMap { it.value }
    val encodings = response.headers.entries.filter { it.key.equals("content-encoding", true) }.flatMap { it.value }
    if (types.size != 1 || !jsonMedia(types.single()) || encodings.size > 1 || encodings.any { !it.trim().equals("identity", true) }) {
        throw SdkException(FailureKind.CONTENT_TYPE, "response is not UTF-8 unencoded application/json", info)
    }
    try {
        val value = Json.parseChecked(response.body, limits.json, checkpoint)
        return codec.decodeChecked(value, limits, checkpoint)
    } catch (error: JsonException) {
        throw SdkException(if (error.kind == JsonErrorKind.RESOURCE_LIMIT) FailureKind.EVALUATION else FailureKind.RESPONSE_VALIDATION, "response JSON parsing failed", info, error)
    } catch (error: ValidationException) { throw SdkException(FailureKind.RESPONSE_VALIDATION, "response schema validation failed", info, error) }
    catch (error: EvaluationException) { throw SdkException(FailureKind.EVALUATION, "response validation could not complete", info, error) }
}
