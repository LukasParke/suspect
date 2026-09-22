package __PACKAGE__

import java.io.ByteArrayOutputStream
import java.io.InputStream
import java.net.http.HttpClient
import java.net.http.HttpRequest as JdkRequest
import java.net.http.HttpResponse as JdkResponse
import java.util.concurrent.CompletionException
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.Executors
import java.util.concurrent.atomic.AtomicBoolean
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.ensureActive
import kotlinx.coroutines.suspendCancellableCoroutine
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.collect
import kotlin.coroutines.resume
import kotlin.coroutines.resumeWithException

/** One owned, cancellable body reader. A null result is EOF; chunks must be finite. */
public interface BodyReader : AutoCloseable {
    /** Read the next transport chunk; cooperate with caller cancellation. */
    public suspend fun read(): ByteArray?
    /** Close the body exactly once from the owning operation/collector. */
    override fun close()
}

/** Response headers and an owned live body. The operation's collector closes it. */
public class StreamingResponse(
    /** Actual status code. */ public val status: Int,
    /** All raw response header values. */ public val headers: Map<String, List<String>>,
    /** Live body, owned by the caller of open. */ public val body: BodyReader,
) : AutoCloseable { override fun close() { body.close() } }

/** Optional streaming extension of the transport seam. */
public interface StreamingTransport : Transport {
    /** Open response headers/body without consuming the stream. */
    public suspend fun open(request: HttpRequest): StreamingResponse
    /** Buffer a finite response with a sentinel byte, retaining cleanup failures as causes. */
    override suspend fun execute(request: HttpRequest): HttpResponse {
        val response = open(request)
        var failure: Throwable? = null
        try {
            if (bodyForbidden(request.method, response.status)) return HttpResponse(response.status, response.headers, byteArrayOf())
            val headers = boundedHeaders(response.headers)
            if (headers.limited || headers.malformed) return HttpResponse(response.status, response.headers, byteArrayOf())
            val bytes = ByteArrayOutputStream()
            while (true) {
                currentCoroutineContext().ensureActive()
                val chunk = response.body.read() ?: break
                if (chunk.size > 65536) throw SdkException(FailureKind.RESPONSE_LIMIT, "transport chunk exceeds the profile")
                val count = minOf(chunk.size, request.maxResponseBytes + 1 - bytes.size())
                if (count > 0) bytes.write(chunk, 0, count)
                if (bytes.size() > request.maxResponseBytes) return HttpResponse(response.status, response.headers, bytes.toByteArray(), true)
            }
            return HttpResponse(response.status, response.headers, bytes.toByteArray())
        } catch (error: Throwable) { failure = error; throw error }
        finally { closeProtocol(response, failure) }
    }
}

internal fun closeProtocol(resource: AutoCloseable, primary: Throwable?) {
    try { resource.close() }
    catch (error: Exception) {
        val wrapped = SdkException(FailureKind.TRANSPORT, "response cleanup failed", cause = error)
        if (primary != null) primary.addSuppressed(wrapped) else throw wrapped
    }
}

internal class JdkStreaming(private val client: HttpClient) : AutoCloseable {
    private val reads = Executors.newVirtualThreadPerTaskExecutor()
    private val bodies = ConcurrentHashMap.newKeySet<InputStream>()
    suspend fun open(request: HttpRequest): StreamingResponse {
        currentCoroutineContext().ensureActive()
        val builder = JdkRequest.newBuilder(request.url)
        for ((key, value) in request.headers) builder.header(key, value)
        builder.method(request.method, request.body?.let(JdkRequest.BodyPublishers::ofByteArray) ?: JdkRequest.BodyPublishers.noBody())
        return suspendCancellableCoroutine { continuation ->
            val future = client.sendAsync(builder.build(), JdkResponse.BodyHandlers.ofInputStream())
            continuation.invokeOnCancellation { future.cancel(true) }
            future.whenComplete { response, error ->
                if (error != null) continuation.resumeWithException(if (error is CompletionException) error.cause ?: error else error)
                else {
                    val input = response.body(); bodies.add(input)
                    val body = object : BodyReader {
                        private val closed = AtomicBoolean(false)
                        override suspend fun read(): ByteArray? = suspendCancellableCoroutine { next ->
                            if (closed.get()) { next.resume(null); return@suspendCancellableCoroutine }
                            val task = reads.submit {
                                try {
                                    val bytes = ByteArray(8192)
                                    val count = input.read(bytes)
                                    next.resume(if (count < 0) null else bytes.copyOf(count))
                                } catch (failure: Exception) { next.resumeWithException(failure) }
                            }
            next.invokeOnCancellation { runCatching { close() }; task.cancel(true) }
                        }
                        override fun close() { if (closed.compareAndSet(false,true)) { bodies.remove(input); input.close() } }
                    }
                    val value = StreamingResponse(response.statusCode(), jdkResponseHeaders(response.headers().map()), body)
                    continuation.resume(value, onCancellation = { _, abandoned, _ -> runCatching { abandoned.close() } })
                }
            }
        }
    }
    override fun close() { for (body in bodies) runCatching { body.close() }; bodies.clear(); reads.shutdownNow() }
}

internal class ProtocolLease(val response: StreamingResponse, private val options: ClientOptions, private val control: CallContext) : AutoCloseable {
    private val prefix = ByteArrayOutputStream()
    private var seen = 0L
    private var complete = false
    private var closed = false
    var contentType: String? = null
    var links: List<LinkMetadata> = emptyList()
    fun info(): ResponseInfo = capture(HttpResponse(response.status,response.headers,prefix.toByteArray(),!complete || seen>prefix.size()),options.captureBytes).withProtocol(contentType,links)
    suspend fun read(): ByteArray? {
        control.check()
        val chunk = try { response.body.read() }
        catch (error: Exception) { control.check(); throw SdkException(FailureKind.TRANSPORT,"stream body read failed",info(),error) }
        control.check()
        if (chunk != null) {
            val kept = minOf(chunk.size, options.captureBytes-prefix.size())
            if(kept>0)prefix.write(chunk,0,kept)
            seen += chunk.size
            control.response = info()
            if(chunk.size>options.maxChunkBytes)throw SdkException(FailureKind.RESPONSE_LIMIT,"stream transport chunk exceeds limit",info())
        }
        else complete = true
        return chunk
    }
    suspend fun finite(): ByteArray {
        val bytes=ByteArrayOutputStream()
        while(true){val chunk=read()?:break;if(chunk.size>options.maxResponseBytes-bytes.size())throw SdkException(FailureKind.RESPONSE_LIMIT,"response body exceeds limit",info());bytes.write(chunk)}
        return bytes.toByteArray()
    }
    override fun close(){if(!closed){closed=true;response.close()}}
}

internal suspend fun openProtocol(transport: Transport, request: HttpRequest, options: ClientOptions, control: CallContext): ProtocolLease {
    val response = try {
        if(transport is StreamingTransport) transport.open(request) else {
            val buffered=transport.execute(request)
            if(!bodyForbidden(request.method,buffered.status)&&(buffered.truncated||buffered.body.size>options.maxResponseBytes))throw SdkException(FailureKind.RESPONSE_LIMIT,"buffered transport response exceeds limit",capture(buffered,options.captureBytes))
            var offset=0
            StreamingResponse(buffered.status,buffered.headers,object:BodyReader{
                override suspend fun read():ByteArray? {if(offset>=buffered.body.size)return null;val end=minOf(offset+8192,buffered.body.size);return buffered.body.copyOfRange(offset,end).also{offset=end}}
                override fun close(){offset=buffered.body.size}
            })
        }
    } catch(cancelled:CancellationException){throw cancelled}
    catch(error:Exception){control.check();if(error is SdkException)throw error;throw SdkException(FailureKind.TRANSPORT,"HTTP stream open failed",cause=error)}
    val lease=ProtocolLease(response,options,control)
    try {
        val info=lease.info();control.response=info;control.check()
        val headers=boundedHeaders(response.headers)
        if(headers.limited)throw SdkException(FailureKind.RESPONSE_LIMIT,"stream header limit exceeded",info)
        if(headers.malformed)throw SdkException(FailureKind.RESPONSE_METADATA,"malformed stream response headers",info)
        if(response.status !in 100..599)throw SdkException(FailureKind.RESPONSE_METADATA,"invalid HTTP status",info)
        return lease
    } catch(error:Throwable){closeProtocol(lease,error);throw error}
}

internal suspend fun protocolItems(lease: ProtocolLease, framing: String, maximum: Int, checkpoint: () -> Unit, emit: suspend (JsonValue) -> Unit) {
    val line=ByteArrayOutputStream();var sawCr=false;var first=true;var eventBytes=0
    val data=mutableListOf<String>();var fields=linkedMapOf<String,JsonValue>()
    suspend fun consumeLine() {
        checkpoint()
        val bytes=line.toByteArray();line.reset()
        var text=try { strictText(bytes) } catch(e:JsonException){throw SdkException(FailureKind.RESPONSE_VALIDATION,"invalid stream UTF-8",lease.info(),e)}
        if(framing=="json-lines") {
            if(text.isBlank())throw SdkException(FailureKind.RESPONSE_VALIDATION,"blank JSON-lines record",lease.info())
            emit(Json.parseChecked(bytes,JsonLimits(maxBytes=maximum),checkpoint));return
        }
        if(first){first=false;if(text.startsWith('\uFEFF'))text=text.substring(1)}
        eventBytes += bytes.size + 1
        if(eventBytes>maximum)throw SdkException(FailureKind.RESPONSE_LIMIT,"SSE frame exceeds item limit",lease.info())
        if(text.isEmpty()) {
            if(data.isNotEmpty()){fields["data"]=JsonString(data.joinToString("\n"));emit(JsonObject(fields))}
            data.clear();fields=linkedMapOf();eventBytes=0;return
        }
        if(text.startsWith(':'))return
        val colon=text.indexOf(':');val name=if(colon<0)text else text.substring(0,colon);var value=if(colon<0)"" else text.substring(colon+1)
        if(value.startsWith(' '))value=value.substring(1)
        when(name){"data"->data.add(value);"event"->fields["event"]=JsonString(value);"id"->if(!value.contains('\u0000'))fields["id"]=JsonString(value);"retry"->if(value.isNotEmpty()&&value.all{it in '0'..'9'}){val digits=value.trimStart('0').ifEmpty{"0"};fields["retry"]=JsonNumber.parse(digits)}}
    }
    while(true) {
        val chunk=lease.read()?:break
        for(byte in chunk) {
            checkpoint();val n=byte.toInt() and 255
            if(sawCr){sawCr=false;if(n==10)continue}
            if(n==10||framing=="server-sent-events"&&n==13){consumeLine();sawCr=n==13}
            else {if(line.size()>=maximum)throw SdkException(FailureKind.RESPONSE_LIMIT,"stream line exceeds item limit",lease.info());line.write(n)}
        }
    }
    if(framing=="json-lines"&&line.size()>0)consumeLine()
    // HTML framing does not dispatch an unfinished SSE block at EOF.
}

internal suspend fun collectProtocol(op: JsonObject, transport: Transport, request: HttpRequest, options: ClientOptions, call: RequestOptions, control: CallContext, emit: suspend (ProtocolResponse) -> Unit) {
    val lease=openProtocol(transport,request,options,control)
    var failure:Throwable?=null
    try {
        val status=lease.response.status
        val ri=responseWork(lease.info()){ProtocolRuntime.select(op,status)}
        val declaration=op.array("responses")[ri]as JsonObject
        if(ProtocolRuntime.forbidden(op,status)) {
            val info=lease.info()
            val result=responseWork(info){ProtocolRuntime.response(op,HttpResponse(status,lease.response.headers,byteArrayOf()),info,call,control)}
            emit(result);return
        }
        val media=declaration.list("media")
        val types=lease.response.headers.entries.filter{it.key.equals("content-type",true)}.flatMap{it.value}
        val mi=if(media.isEmpty())null else responseWork(lease.info()){if(types.size!=1)throw SdkException(FailureKind.CONTENT_TYPE,"one Content-Type is required");selectMedia(media,types.single())}
        val selected=mi?.let{media[it]as JsonObject}
        val representation=selected?.obj("representation")
        if(representation?.text("kind")!="stream") {
            val body=lease.finite();val info=lease.info()
            emit(responseWork(info){ProtocolRuntime.response(op,HttpResponse(status,lease.response.headers,body),info,call,control)});return
        }
        val encodings=lease.response.headers.entries.filter{it.key.equals("content-encoding",true)}.flatMap{it.value}
        if(encodings.size>1||encodings.any{!it.trim().equals("identity",true)})throw SdkException(FailureKind.CONTENT_TYPE,"unsupported stream Content-Encoding",lease.info())
        if(status in 200..299 && call.responseMedia!=null && !protocolMedia(call.responseMedia,true).accepts(protocolMedia(types.single())))throw SdkException(FailureKind.CONTENT_TYPE,"stream representation differs from requested media",lease.info())
        val headerValues=responseWork(lease.info()){typedHeaders(declaration.list("headers"),lease.response.headers)}
        lease.contentType=types.single()
        lease.links=declaration.list("links").map{val link=it as JsonObject;LinkMetadata(link.text("name"),protocolMetadata(link))}
        val stream=representation.obj("stream");val maximum=minOf(options.maxStreamItemBytes,stream.number("max_item_bytes"))
        protocolItems(lease,stream.text("framing"),maximum,control::check){item->
            emit(ProtocolResponse(ri,mi,false,ProtocolValue.Json(item),headerValues,lease.info()))
        }
    } catch(cancelled:CancellationException){failure=cancelled;throw cancelled}
    catch(error:Throwable){
        val mapped=try { control.check(); responseWork(lease.info()){throw error} } catch(mapped:Throwable){mapped}
        failure=mapped
        throw mapped
    }
    finally{closeProtocol(lease,failure)}
}

// Request item sequences are collected exactly once under a finite total byte
// policy before sending. Producer failure/cancellation cannot send a partial POST.
internal suspend fun <T> encodeRequestItems(items:Flow<T>, stream:JsonObject, maximum:Int, itemMaximum:Int, checkpoint:()->Unit, encode:(T)->JsonValue):ByteArray {
    val out=ByteArrayOutputStream();var count=0;val limit=minOf(itemMaximum,stream.number("max_item_bytes"))
    try {
        items.collect { item ->
            checkpoint();if(++count>100000)throw SdkException(FailureKind.REQUEST_LIMIT,"request item count exceeds limit")
            val value=encode(item)
            val encoded=if(stream.text("framing")=="json-lines")Json.stringifyChecked(value,JsonLimits(maxBytes=limit),checkpoint)+"\n" else {
                val fields=(value as?JsonObject)?.values?:throw SdkException(FailureKind.REQUEST_REPRESENTATION,"SSE request item must be an event envelope")
                if(fields.keys.any{it !in setOf("data","event","id","retry")})throw SdkException(FailureKind.REQUEST_REPRESENTATION,"SSE has no representation for extra fields")
                val data=(fields["data"]as?JsonString)?.value?:throw SdkException(FailureKind.REQUEST_REPRESENTATION,"SSE request data must be a string")
                if(data.contains('\r'))throw SdkException(FailureKind.REQUEST_REPRESENTATION,"SSE data cannot preserve CR; supply LF-separated data")
                val dataBytes=Json.utf8Size(data,limit,checkpoint)
                val newlines=data.count{it=='\n'}
                if(dataBytes.toLong()-newlines+(newlines.toLong()+1)*7+1>limit)throw SdkException(FailureKind.REQUEST_LIMIT,"SSE framing expansion exceeds item limit")
                val text=StringBuilder()
                for(key in listOf("event","id")){val raw=fields[key]?:continue;val field=(raw as?JsonString)?.value?:throw SdkException(FailureKind.REQUEST_REPRESENTATION,"SSE field must be string");if(field.any{it=='\n'||it=='\r'||it=='\u0000'})throw SdkException(FailureKind.REQUEST_REPRESENTATION,"SSE event/id contains framing characters");text.append(key).append(": ").append(field).append('\n')}
                fields["retry"]?.let { raw -> val retry=(raw as?JsonNumber)?.toBigIntegerExact()?:throw SdkException(FailureKind.REQUEST_REPRESENTATION,"SSE retry must be integer");if(retry.signum()<0)throw SdkException(FailureKind.REQUEST_REPRESENTATION,"SSE retry must be nonnegative");text.append("retry: ").append(retry).append('\n') }
                for(line in data.split('\n'))text.append("data: ").append(line).append('\n')
                text.append('\n').toString()
            }
            val size=Json.utf8Size(encoded,limit,checkpoint)
            if(size>maximum-out.size())throw SdkException(FailureKind.REQUEST_LIMIT,"request item bytes exceed total limit")
            out.write(encoded.toByteArray(Charsets.UTF_8))
        }
    } catch(cancelled:CancellationException){throw cancelled}
    catch(error:SdkException){throw error}
    catch(error:Exception){throw SdkException(FailureKind.REQUEST_REPRESENTATION,"request item producer failed",cause=error)}
    return out.toByteArray()
}
