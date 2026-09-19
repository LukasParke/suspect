package consumer

import example.protocol.*
import com.sun.net.httpserver.HttpServer
import java.net.InetSocketAddress
import java.net.URI
import java.time.Duration
import java.util.Collections
import java.util.Base64
import java.util.concurrent.CountDownLatch
import java.util.concurrent.Executors
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicInteger
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.*

private const val REPLY = """{"name":"ok","value":9007199254740993.000000000000000001}"""
private val FILE = byteArrayOf(0, -1, 10, 65, 13, 10, 66)
private val MIME_FILE = FILE + "\r\n--independent-lookalike\r\nstill part data".toByteArray()
private data class Seen(val method:String,val target:String,val headers:Map<String,List<String>>,val body:ByteArray) {
    fun header(name:String)=headers.entries.firstOrNull{it.key.equals(name,true)}?.value?.firstOrNull()
}
private suspend inline fun <reified T:Throwable> fails(crossinline block:suspend()->Unit):T {
    try {block()}catch(error:Throwable){check(error is T){"expected ${T::class.simpleName}, got $error"};return error};error("expected failure")
}
private fun multipartFixture():ByteArray {
    val head="--independent\r\nContent-Disposition: form-data; name=\"file\"; filename=\"raw.bin\"\r\nContent-Type: application/octet-stream\r\nX-Part: 7\r\n\r\n".toByteArray()
    val tail="\r\n--independent\r\nContent-Disposition: form-data; name=\"meta\"\r\nContent-Type: application/json\r\n\r\n{\"flag\":true}\r\n--independent\r\nContent-Disposition: form-data; name=\"title\"\r\nContent-Type: text/plain\r\n\r\nfrom-wire\r\n--independent--\r\n".toByteArray()
    return head+MIME_FILE+tail
}

private suspend fun wire() {
    val records=Collections.synchronizedList(mutableListOf<Seen>())
    val server=HttpServer.create(InetSocketAddress("127.0.0.1",0),0)
    val executor=Executors.newVirtualThreadPerTaskExecutor();server.executor=executor
    val streamClosed=CountDownLatch(1);val streamEntered=CompletableDeferred<Unit>()
    server.createContext("/"){exchange->
        try {
            val target=exchange.requestURI.toASCIIString();val path=exchange.requestURI.path
            val body=exchange.requestBody.readAllBytes()
            records.add(Seen(exchange.requestMethod,target,exchange.requestHeaders.mapValues{it.value.toList()},body))
            var status=200;var type="application/json";var response=REPLY.toByteArray()
            when {
                path.endsWith("/anonymous")->{exchange.responseHeaders.add("X-Count","9007199254740993");exchange.responseHeaders.add("X-Json","{\"ok\":true}")}
                path.endsWith("/choose")->when {
                    target.contains("case=range")-> {status=202;type="text/plain";response="range value".toByteArray()}
                    target.contains("case=none")-> {status=204;response=byteArrayOf()}
                    target.contains("case=error")-> {status=418;type="application/octet-stream";response=FILE}
                }
                path.endsWith("/fallback")->status=if(target.contains("code=418"))418 else 201
                path.endsWith("/media")->when {
                    target.contains("case=json")->type="application/json; profile=one; charset=utf-8"
                    target.contains("case=text")-> {type="text/example";response=FILE}
                    target.contains("case=bad")-> {type="application/json; profile=one";response="{}".toByteArray()}
                    else->{type="image/png";response=FILE}
                }
                path.endsWith("/text")-> {type="text/plain; charset=utf-8";response="9007199254740993".toByteArray()}
                path.endsWith("/structured")->type="application/problem+json"
                path.endsWith("/form-reply")-> {type="application/x-www-form-urlencoded";response="name=from+wire&tags=one&tags=two%2Cthree&config=%7B%22ok%22%3Atrue%7D&extra=a%2Bb".toByteArray()}
                path.endsWith("/typed-headers")-> {exchange.responseHeaders.add("X-List","1,2.0");exchange.responseHeaders.add("X-Object","active=true,count=9007199254740993");exchange.responseHeaders.add("X-Text","3e1")}
                path.endsWith("/multipart")-> {type="multipart/form-data; boundary=independent";response=multipartFixture()}
                path.endsWith("/none")-> {type="application/octet-stream";response=FILE}
                path.endsWith("/head")->response=byteArrayOf()
                path.endsWith("/events")-> {
                    type="text/event-stream"
                    exchange.responseHeaders.add("Content-Type",type)
                    exchange.sendResponseHeaders(200,0)
                    val prefix="\uFEFF: comment\r\nevent: message\r\nid: a\r\nretry: 0007\r\nunknown: ignored\r\ndata: {\"literal\":1}\r\ndata: 😀\r\n\r\n".toByteArray()
                    for(byte in prefix){exchange.responseBody.write(byte.toInt());exchange.responseBody.flush()}
                    streamEntered.complete(Unit)
                    try {repeat(512){exchange.responseBody.write("data: later\n\n".toByteArray());exchange.responseBody.flush();Thread.sleep(5)}}
                    catch(_:java.io.IOException){streamClosed.countDown()}
                    return@createContext
                }
                path.endsWith("/lines")-> {type="application/x-ndjson";response=(REPLY+"\r\n"+REPLY+"\n").toByteArray()}
            }
            exchange.responseHeaders.add("Content-Type",type)
            if(exchange.requestMethod=="HEAD"||status==204)exchange.sendResponseHeaders(status,-1)
            else{exchange.sendResponseHeaders(status,response.size.toLong());exchange.responseBody.write(response)}
        }catch(_:java.io.IOException){}finally{exchange.close()}
    }
    server.start();val base=URI("http://127.0.0.1:${server.address.port}/api")
    val scopes=mutableListOf<CredentialContext>()
    val credentials=Credentials(token="bearer-token",queryKey="a/b +雪",headerKey="header-token",cookieKey="cookie-token",
        basicAuth=BasicCredentials("üser","password"),oauth=CredentialProvider{ctx->scopes.add(ctx);"Bearer oauth-token"},oidc=CredentialProvider{ctx->scopes.add(ctx);"Bearer oidc-token"})
    try {
        Client(credentials,options=ClientOptions(serverUrl=base)).use{client->
            val anonymous=client.anonymous() as AnonymousResult.Status200
            check(anonymous.data.value.token=="9007199254740993.000000000000000001")
            check(anonymous.responseHeaders.xCount.toBigIntegerExact().toString()=="9007199254740993")
            check(anonymous.responseHeaders.xJson.ok)
            check(anonymous.response.links.single().name=="text")
            check(records.last().header("Authorization")==null)
            client.secure();check(records.last().header("Authorization")=="Bearer bearer-token")
            check(records.last().target=="/api/secure?key=a%2Fb%20%2B%E9%9B%AA")
            client.secure(requestOptions=RequestOptions(securityAlternative=2));check(records.last().header("Authorization")==null&&records.last().target=="/api/secure")
            client.basic();check(records.last().header("Authorization")=="Basic "+Base64.getEncoder().encodeToString("üser:password".toByteArray()))
            client.oauthCall();client.oidcCall()
            check(scopes[0].scopes==listOf("read:items")&&scopes[0].scheme=="oauth")
            check(Json.stringify(scopes[0].metadata).contains("https://identity.test/token"))
            check(Json.stringify(scopes[1].metadata).contains("openid-configuration"))
            client.parameters(ParametersInput(id=listOf("one","two"),where=Presence.Present(ParametersParameter1(name="a b",count=Presence.Present(JsonNumber.of(2)))),xNumbers=Presence.Present(listOf(JsonNumber.of(1),JsonNumber.parse("2.0"))),p=Presence.Present("hello%20world"),reserved=Presence.Present("a/b?x%3D1%26y"),filter=Presence.Present(ParametersParameter5(JsonNumber.of(3)))))
            val parameters=records.last()
            check(parameters.target=="/api/params/.one.two?where%5Bcount%5D=2&where%5Bname%5D=a%20b&reserved=a/b?x%3D1%26y&filter=%7B%22count%22%3A3%7D") {parameters.target}
            check(parameters.header("X-Key")=="header-token"&&parameters.header("X-Numbers")=="1,2.0")
            check(parameters.header("Cookie")=="p=hello%20world; session=cookie-token")
            client.matrix(MatrixInput(value=listOf("a",""),pipe=Presence.Present(listOf("x","y")),space=Presence.Present(listOf("x","y"))))
            check(records.last().target=="/api/matrix/;value=a;value?pipe=x%7Cy&space=x%20y") {records.last().target}
            val exact=client.choose(ChooseInput(body=ChooseRequestBody.Json(Input("native"))))
            check(exact is ChooseResult.Status200 && exact.data.name=="ok")
            check(records.last().body.toString(Charsets.UTF_8)=="{\"name\":\"native\"}")
            val ranged=client.choose(ChooseInput(case=Presence.Present("range"),body=ChooseRequestBody.Text("plain 😀")),RequestOptions(responseMedia="text/plain"))
            check(ranged is ChooseResult.Range2XX && ranged.response.status==202 && ranged.data=="range value")
            check(records.last().body.toString(Charsets.UTF_8)=="plain 😀"&&records.last().header("Content-Type")=="text/plain")
            check(client.choose(ChooseInput(case=Presence.Present("none"),body=ChooseRequestBody.Bytes(FILE))) is ChooseResult.Range2XXNoContent)
            check(records.last().body.contentEquals(FILE))
            val denied=fails<ChooseApiException.Default>{client.choose(ChooseInput(case=Presence.Present("error"),body=ChooseRequestBody.Bytes(FILE)))}
            check(denied.response.status==418&&denied.data.contentEquals(FILE))
            check(client.fallback() is FallbackResult.Default)
            val fallback=fails<FallbackApiException.Default>{client.fallback(FallbackInput(Presence.Present(JsonNumber.of(418))))}
            check(fallback.response.status==418&&fallback.data.name=="ok")
            check(client.media(MediaInput(Presence.Present("json"))) is MediaResult.Status200Json)
            check((client.media() as MediaResult.Status200Bytes).data.contentEquals(FILE))
            check((client.media(MediaInput(Presence.Present("text"))) as MediaResult.Status200Bytes3).data.contentEquals(FILE))
            check(fails<SdkException>{client.media(MediaInput(Presence.Present("bad")))}.kind==FailureKind.RESPONSE_VALIDATION)
            val number=client.textValue(TextValueInput(JsonNumber.parse("10e-00000000000000000000000000000000000000001")))
            check(number.data.toBigIntegerExact().toString()=="9007199254740993")
            check(records.last().body.toString(Charsets.UTF_8)=="10e-00000000000000000000000000000000000000001")
            client.form(FormInput(FormRequestFormBody(config=FormBodyConfig(true),name="a b+雪",tags=listOf("one","two,three"))))
            check(records.last().body.toString(Charsets.UTF_8)=="config=%7B%22ok%22%3Atrue%7D&name=a+b%2B%E9%9B%AA&tags=one&tags=two%2Cthree") {records.last().body.toString(Charsets.UTF_8)}
            val upload=UploadRequestMultipartBody(file=UploadRequestMultipartBodyFilePart(Upload(FILE,"file\".bin"),UploadRequestMultipartBodyFileHeaders(JsonNumber.of(7))),title="a title",meta=UploadBodyMeta(true),chunks=Presence.Present(listOf(Upload(byteArrayOf(1)),Upload(byteArrayOf(2)))))
            client.upload(UploadInput(upload))
            val wire=records.last();check(wire.header("Content-Type")!!.startsWith("multipart/form-data; boundary=suspect-"))
            val latin=wire.body.toString(Charsets.ISO_8859_1)
            check(latin.contains("filename=\"file\\\".bin\"")&&latin.contains("X-Part: 7\r\n")&&latin.contains(FILE.toString(Charsets.ISO_8859_1)))
            check(latin.contains("name=\"meta\"\r\nContent-Type: application/json\r\n\r\n{\"flag\":true}"))
            check(latin.split("name=\"chunks\"").size==3)
            val formReply=client.formReply().data
            check(formReply.name=="from wire"&&formReply.tags==listOf("one","two,three")&&formReply.config.ok&&formReply.additionalProperties["extra"]=="a+b")
            val typed=client.typedHeaders() as TypedHeadersResult.Status200
            check(typed.responseHeaders.xList==listOf(JsonNumber.of(1),JsonNumber.of(2)))
            check(typed.responseHeaders.xObject.active&&typed.responseHeaders.xObject.additionalProperties.getValue("count").token=="9007199254740993")
            check(typed.responseHeaders.xText.toLongExact()==30L)
            val multipart=client.multipartReply().data
            check(multipart.file.value.data.contentEquals(MIME_FILE)&&multipart.file.headers.xPart.toLongExact()==7L)
            check(multipart.file.value.filename=="raw.bin"&&multipart.meta.flag&&multipart.title=="from-wire")
            check(client.head().data==Unit)
            check((client.noDeclaredBody() as NoDeclaredBodyResult.Status200).data.contentEquals(FILE))
            val before=records.size;val cold=client.events();check(records.size==before)
            val event=cold.take(1).toList().single().data
            check(event.data=="{\"literal\":1}\n😀"&&event.event==Presence.Present("message")&&event.id==Presence.Present("a"))
            check((event.retry as Presence.Present).value.toLongExact()==7L)
            withTimeout(2000){streamEntered.await()}
            check(streamClosed.await(3,TimeUnit.SECONDS)){"early Flow completion left the HTTP body alive"}
            val lines=client.lines().toList();check(lines.size==2&&lines.all{it.data.value.token=="9007199254740993.000000000000000001"})
            client.postEvents(PostEventsInput(flowOf(Event(data="{\"text\":1}\n😀",retry=Presence.Present(JsonNumber.parse("1.0"))))))
            check(records.last().body.toString(Charsets.UTF_8)=="retry: 1\ndata: {\"text\":1}\ndata: 😀\n\n")
            check(records.last().header("Content-Type")=="text/event-stream")
            client.postLines(PostLinesInput(flowOf(Reply("one",JsonNumber.parse("9007199254740993")),Reply("two",JsonNumber.parse("1e-400")))))
            check(records.last().body.toString(Charsets.UTF_8)=="{\"name\":\"one\",\"value\":9007199254740993}\n{\"name\":\"two\",\"value\":1e-400}\n")
            client.structured(StructuredInput(Input("patch")));check(records.last().header("Content-Type")=="application/merge-patch+json")
            check(client.schemaFree(SchemaFreeInput(JsonObject(mapOf("free" to JsonNumber.parse("1e99999999999"))))).data is JsonObject)
            check(client.mixed().toList().single().data.name=="ok")
            client.wildcard(WildcardInput(WildcardRequestBody.Bytes(FILE)),RequestOptions(requestMedia="application/octet-stream"))
            check(records.last().body.contentEquals(FILE))
            val undocumented=fails<SdkException>{client.undocumented()}
            check(undocumented.kind==FailureKind.UNEXPECTED_STATUS&&undocumented.response!!.status==200)
            client.custom();check(records.last().method=="REPORT")
            client.search(SearchInput(SearchParameter0(JsonNumber.of(1),"a b+雪")))
            check(records.last().target=="/api/search?a=1&z=a+b%2B%E9%9B%AA")
            val sent=records.size
            fails<SdkException>{client.matrix(MatrixInput(listOf("a"),pipe=Presence.Present(listOf("x|y"))))}
            fails<SdkException>{client.parameters(ParametersInput(id=emptyList()))}
            fails<SdkException>{client.parameters(ParametersInput(id=listOf("a"),reserved=Presence.Present("bad&query")))}
            fails<SdkException>{client.upload(UploadInput(upload.copy(chunks=Presence.Present(emptyList()))))}
            fails<SdkException>{client.wildcard(WildcardInput(WildcardRequestBody.Bytes(FILE)),RequestOptions(requestMedia="application/json"))}
            check(records.size==sent)
        }
        Client(options=ClientOptions(documentUrl=URI("HTTP://127.0.0.1:${server.address.port}/docs/openapi.json"),serverIndex=1,serverVariables=mapOf("version" to "api"))).use{client->
            client.servers();check(records.last().target=="/api/servers")
            val count=records.size;fails<SdkException>{client.servers(requestOptions=RequestOptions(serverVariables=mapOf("version" to "invalid")))};check(records.size==count)
        }
        println("KOTLIN_PROTOCOL_WIRE=${records.size}")
    }finally{server.stop(0);executor.shutdownNow();check(executor.awaitTermination(5,TimeUnit.SECONDS))}
}

private class StreamStub(private val chunks:List<ByteArray>,private val status:Int=200,private val media:String="text/event-stream",private val wait:Boolean=false,private val badClose:Boolean=false):StreamingTransport {
    var opened=0;var closed=0;val entered=CompletableDeferred<Unit>()
    override suspend fun open(request:HttpRequest):StreamingResponse {
        opened++;var index=0
        return StreamingResponse(status,mapOf("Content-Type" to listOf(media)),object:BodyReader{
            override suspend fun read():ByteArray?{if(index<chunks.size)return chunks[index++];if(wait){entered.complete(Unit);awaitCancellation()};return null}
            override fun close(){closed++;if(badClose)throw IllegalStateException("private cleanup")}
        })
    }
}
private suspend fun resources() {
    val split="data: first\r\n\r\ndata: second\r\n\r\ndata: unfinished".toByteArray().map{byteArrayOf(it)}
    val stream=StreamStub(split)
    Client(transport=stream).use{client->val flow=client.events();check(stream.opened==0);check(flow.toList().map{it.data.data}==listOf("first","second"));check(stream.closed==1)}
    val bad=StreamStub(listOf("data: ".toByteArray()+ByteArray(1000){120}),badClose=true)
    Client(transport=bad,options=ClientOptions(maxStreamItemBytes=32,captureBytes=8)).use{client->
        val error=fails<SdkException>{client.events().toList()};check(error.kind==FailureKind.RESPONSE_LIMIT&&error.response!!.bodyPreview.size<=8);check(error.suppressed.isNotEmpty()||error.cause?.suppressed?.isNotEmpty()==true)
    };check(bad.closed==1)
    val badJson=StreamStub(listOf("not-json\n".toByteArray()),media="application/x-ndjson")
    Client(transport=badJson).use{client->check(fails<SdkException>{client.lines().toList()}.kind==FailureKind.RESPONSE_VALIDATION)};check(badJson.closed==1)
    val cancelled=StreamStub(listOf("data: one\n\n".toByteArray()),wait=true,badClose=true)
    Client(transport=cancelled).use{client->coroutineScope{var saw=false;val job=launch{try{client.events().collect{}}catch(e:CancellationException){saw=true;throw e}};withTimeout(2000){cancelled.entered.await()};job.cancelAndJoin();check(saw)}};check(cancelled.closed==1)
    val timeout=StreamStub(emptyList(),wait=true,badClose=true)
    Client(transport=timeout,options=ClientOptions(timeout=Duration.ofMillis(25))).use{client->check(fails<SdkException>{client.events().toList()}.kind==FailureKind.TIMEOUT)};check(timeout.closed==1)
    val declared=StreamStub(listOf("{\"message\":\"denied\"}".toByteArray()),status=400,media="application/json")
    Client(transport=declared).use{client->check(fails<EventsApiException.Status400>{client.events().toList()}.data.message=="denied")};check(declared.closed==1)
    val calls=AtomicInteger()
    val transport=Transport{calls.incrementAndGet();HttpResponse(200,mapOf("Content-Type" to listOf("application/json")),REPLY.toByteArray())}
    Client(transport=transport).use{client->fails<SdkException>{client.basic()};fails<SdkException>{client.secure(requestOptions=RequestOptions(securityAlternative=0))}}
    Client(Credentials(basicAuth=BasicCredentials("user\n","password")),transport).use{client->check(fails<SdkException>{client.basic()}.kind==FailureKind.AUTHENTICATION)}
    check(calls.get()==0)
    var producerClosed=false
    Client(transport=transport,options=ClientOptions(maxRequestBytes=128,maxStreamItemBytes=64)).use{client->
        val items=flow{try{repeat(100){emit(Reply("oversized",JsonNumber.of(it.toLong())))}}finally{producerClosed=true}}
        fails<SdkException>{client.postLines(PostLinesInput(items))}
    }
    check(producerClosed&&calls.get()==0)
    val producerEntered=CompletableDeferred<Unit>();producerClosed=false
    Client(transport=transport).use{client->coroutineScope{
        val items=flow<Reply>{try{producerEntered.complete(Unit);awaitCancellation()}finally{producerClosed=true}}
        val pending=launch{client.postLines(PostLinesInput(items))};producerEntered.await();pending.cancelAndJoin()
    }}
    check(producerClosed&&calls.get()==0)
    Client(transport=transport).use{client->check(Quickstart.firstRequest(client).response.status==200)}
    check(calls.get()==1)
    Client(transport=transport).use{client->check(uploadBytes(client,FILE).name=="ok")}
    check(calls.get()==2)
    val guideStream=StreamStub(listOf("data: native guide\n\n".toByteArray()))
    Client(transport=guideStream).use{client->check(eventData(client).take(1).toList()==listOf("native guide"))};check(guideStream.closed==1)
    val lower=ClientOptions(codecLimits=CodecLimits(json=JsonLimits(maxBytes=64,maxNumberBytes=8)))
    Client(transport=transport,options=lower).use{client->
        check(fails<SdkException>{client.schemaFree(SchemaFreeInput(JsonNumber.parse("123456789")))}.kind==FailureKind.EVALUATION)
        check(fails<SdkException>{client.textValue(TextValueInput(JsonNumber.parse("123456789")))}.kind==FailureKind.EVALUATION)
    }
    check(calls.get()==2)
    for(status in listOf(103,204,205,304)){
        var reads=0;var closed=0
        val forbidden=object:StreamingTransport{
            override suspend fun open(request:HttpRequest)=StreamingResponse(status,emptyMap(),object:BodyReader{
                override suspend fun read():ByteArray?{reads++;error("forbidden response body was read")}
                override fun close(){closed++}
            })
        }
        Client(transport=forbidden).use{client->
            if(status in 200..299)check(client.fallback() is FallbackResult.DefaultNoContent)
            else fails<FallbackApiException.DefaultNoContent>{client.fallback()}
        }
        check(reads==0&&closed==1)
    }
    val mixed=StreamStub(listOf((REPLY+"\n").toByteArray()),media="application/x-ndjson")
    Client(transport=mixed).use{client->check(client.mixed().toList().single().data.name=="ok")};check(mixed.closed==1)
    for(bytes in listOf(byteArrayOf(-1,10),"\n".toByteArray(),"{\"name\":\"ok\"}\n".toByteArray())) {
        val invalid=StreamStub(listOf(bytes),media="application/x-ndjson")
        Client(transport=invalid).use{client->check(fails<SdkException>{client.lines().toList()}.kind==FailureKind.RESPONSE_VALIDATION)}
        check(invalid.closed==1)
    }
    val oversized=StreamStub(listOf(ByteArray(65537){120}))
    Client(transport=oversized).use{client->check(fails<SdkException>{client.events().toList()}.kind==FailureKind.RESPONSE_LIMIT)};check(oversized.closed==1)
    for(payload in listOf(
        multipartFixture().toString(Charsets.ISO_8859_1).replace("name=\"meta\"","name=\"unknown\"").toByteArray(Charsets.ISO_8859_1),
        multipartFixture().toString(Charsets.ISO_8859_1).replace("X-Part: 7","X-Part: not-an-integer").toByteArray(Charsets.ISO_8859_1),
        multipartFixture().toString(Charsets.ISO_8859_1).replace("--independent--\r\n","--independent--suffix").toByteArray(Charsets.ISO_8859_1),
    )) {
        Client(transport=Transport{HttpResponse(200,mapOf("Content-Type" to listOf("multipart/form-data; boundary=independent")),payload)}).use{client->
            check(fails<SdkException>{client.multipartReply()}.kind==FailureKind.RESPONSE_VALIDATION)
        }
    }
    Client(transport=Transport{HttpResponse(200,mapOf("Content-Type" to listOf("application/x-www-form-urlencoded")),"name=x&tags=a&config=%GG".toByteArray())}).use{client->
        check(fails<SdkException>{client.formReply()}.kind==FailureKind.RESPONSE_VALIDATION)
    }
    println("KOTLIN_PROTOCOL_RESOURCES_PASSED")
}

fun main()=runBlocking {withTimeout(60000){wire();resources()};println("KOTLIN_PROTOCOL_NATIVE_PASSED")}
