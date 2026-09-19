package __PACKAGE__

import java.net.URI
import java.util.Base64
import kotlinx.coroutines.CancellationException

/** Explicit HTTP Basic values. UTF-8 is the declared native attachment policy. */
public class BasicCredentials(
    /** Username, without the Basic delimiter colon. */ public val username: String,
    /** Password, preserved as supplied. */ public val password: String,
) { override fun toString(): String = "BasicCredentials([redacted])" }

/** Caller-owned OAuth/OIDC authorization hook; returns the complete Authorization field. */
public fun interface CredentialProvider {
    /** Attach an authorization value; acquisition/refresh policy belongs to the caller. */
    public suspend fun authorization(context: CredentialContext): String
}

/** Immutable source metadata supplied to an authorization hook. */
public class CredentialContext(
    /** Canonical operation ID. */ public val operationId: String,
    /** Source scheme name. */ public val scheme: String,
    /** Source-declared scopes, in order. */ public val scopes: List<String>,
    /** Source-declared roles, in order. */ public val roles: List<String>,
    /** Source identity of the scheme definition. */ public val source: SourceLocation,
    /** Original checked hook metadata: OAuth flows or OIDC discovery URI. */ public val metadata: JsonObject,
    /** Selected effective server, the base for relative OAuth/OIDC metadata URLs. */ public val effectiveServer: URI? = null,
)

/** Binary multipart value. Bytes are never sent through a JSON schema surrogate. */
public class Upload(
    /** Actual file/part bytes. */ public val data: ByteArray,
    /** Optional wire filename. */ public val filename: String? = null,
    /** Concrete source-permitted content type override. */ public val contentType: String? = null,
)

/** Source response-link metadata. Values and expressions are retained, never followed. */
public class LinkMetadata(
    /** Source link name. */ public val name: String,
    /** Original target/parameter/body/server descriptors. */ public val declaration: JsonObject,
)

internal sealed interface ProtocolCredential {
    class Token(val value: String): ProtocolCredential
    class Basic(val value: BasicCredentials): ProtocolCredential
    class Provider(val value: CredentialProvider): ProtocolCredential
}
internal data class PreparedInput(val parameters: Map<Int, JsonValue>, val body: PreparedBody?)
internal data class PreparedBody(val media: Int, val value: ProtocolValue)
internal sealed interface ProtocolValue {
    class Json(val value: JsonValue): ProtocolValue
    class Bytes(val value: ByteArray): ProtocolValue
    class Parts(val values: List<PreparedPart>): ProtocolValue
    data object None: ProtocolValue
}
internal data class PreparedPart(val name: String, val values: List<ProtocolPartValue>)
internal data class ProtocolPartValue(val value: ProtocolValue, val headers: Map<String, JsonValue> = emptyMap(), val contentType: String? = null, val filename: String? = null)
internal data class ProtocolResponse(val responseIndex: Int, val mediaIndex: Int?, val forbidden: Boolean, val value: ProtocolValue, val headers: Map<String, JsonValue>, val info: ResponseInfo)

internal object ProtocolData {
    val operations: List<JsonObject> by lazy {
        val bytes = ProtocolData::class.java.getResourceAsStream("protocol.json")?.use { it.readNBytes(Json.MAX_BYTES + 1) }
            ?: error("generated protocol metadata is missing")
        val value = Json.parse(bytes) as JsonObject
        check(value.number("version") == 1) { "unsupported protocol descriptor version" }
        value.array("operations").map { it as JsonObject }
    }
}
internal fun JsonObject.optional(key: String): JsonValue? = values[key]?.takeUnless { it === JsonNull }
internal fun JsonObject.string(key: String, default: String = ""): String = (optional(key) as? JsonString)?.value ?: default
internal fun JsonObject.list(key: String): List<JsonValue> = (optional(key) as? JsonArray)?.values ?: emptyList()
internal fun JsonObject.boolean(key: String, default: Boolean = false): Boolean = (optional(key) as? JsonBoolean)?.value ?: default
internal fun protocolSource(value: JsonObject): SourceLocation = value.let { SourceLocation(it.text("document"), it.text("pointer")) }
internal fun protocolAddress(value: JsonObject): String = protocolSource(value).let { it.document + "#" + it.pointer }
internal fun protocolMetadata(value: JsonObject): JsonObject = Json.parse(Json.stringify(value)) as JsonObject

internal class ProtocolBudget(val maximum: Int, val checkpoint: () -> Unit) {
    private var remaining = maximum
    fun spend(bytes: Int) { checkpoint(); if (bytes < 0 || bytes > remaining) throw SdkException(FailureKind.REQUEST_LIMIT, "protocol byte limit exceeded"); remaining -= bytes }
    fun text(value: String): String {
        // The wire budget owns this classification: resource exhaustion while
        // sizing a protocol field is a request limit, not a JSON evaluation
        // failure. Rethrowing SdkException keeps the operation id and source
        // location attached by the operation wrapper.
        val count = try { Json.utf8Size(value, maximum, checkpoint) } catch (error: JsonException) {
            if (error.kind == JsonErrorKind.RESOURCE_LIMIT) throw SdkException(FailureKind.REQUEST_LIMIT, "protocol byte limit exceeded", cause = error) else throw error }
        spend(count); return value
    }
}

internal data class ProtocolMedia(val type: String, val subtype: String, val parameters: Map<String,String>) {
    val specificity: Int get() = (if (type == "*") 0 else if (subtype == "*") 1 else 2) * 1000 + parameters.size
    fun accepts(other: ProtocolMedia): Boolean = (type == "*" || type == other.type) && (subtype == "*" || subtype == other.subtype) && parameters.all { (key,value) -> other.parameters[key]?.let { if (key == "charset") it.equals(value,true) else it == value } == true }
}
internal fun protocolMedia(value: String, ranges: Boolean = false): ProtocolMedia {
    require(value.length <= 32768) { "media field exceeds header limit" }
    var at = 0
    fun ws() { while (at < value.length && value[at] in " \t") at++ }
    fun token(): String { val start = at; while (at < value.length && (value[at].isLetterOrDigit() && value[at].code < 128 || value[at] in "!#$%&'*+-.^_`|~")) at++; require(at > start) { "invalid media token" }; return value.substring(start,at) }
    ws(); val type = token().lowercase(); require(at < value.length && value[at++] == '/'); val subtype = token().lowercase()
    val parameters = linkedMapOf<String,String>(); ws()
    while (at < value.length) {
        require(value[at++] == ';'); ws(); val name = token().lowercase(); ws(); require(at < value.length && value[at++] == '='); ws()
        val parameter = if (at < value.length && value[at] == '"') {
            at++; val result = StringBuilder(); var closed = false
            while (at < value.length) { val c = value[at++]; if (c == '"') { closed = true; break }; if (c == '\\') { require(at < value.length); result.append(value[at++]) } else { require(c >= ' ' && c != '\u007f'); result.append(c) } }
            require(closed); result.toString()
        } else token()
        require(parameters.put(name,parameter) == null); ws()
    }
    require((type != "*" || subtype == "*") && (!type.contains('*') || type == "*") && (!subtype.contains('*') || subtype == "*"))
    require(ranges || type != "*" && subtype != "*")
    return ProtocolMedia(type,subtype,parameters)
}
internal fun selectMedia(media: List<JsonValue>, actual: String): Int {
    val value = try { protocolMedia(actual) } catch (e: IllegalArgumentException) { throw SdkException(FailureKind.CONTENT_TYPE,"malformed Content-Type",cause=e) }
    val selected = media.indices.filter { protocolMedia((media[it] as JsonObject).obj("media_type").text("declared"),true).accepts(value) }
        .maxByOrNull { protocolMedia((media[it] as JsonObject).obj("media_type").text("declared"),true).specificity }
        ?: throw SdkException(FailureKind.CONTENT_TYPE,"media type is not declared")
    if ((media[selected] as JsonObject).obj("representation").text("kind") in listOf("json","text","stream","form") && value.parameters["charset"]?.let { !it.equals("utf-8",true) } == true) throw SdkException(FailureKind.CONTENT_TYPE,"unsupported response charset")
    return selected
}
internal fun concreteMedia(media: JsonObject, override: String?): String {
    val declared = media.obj("media_type").text("declared")
    val parsed = protocolMedia(declared,true)
    val result = override ?: declared.takeUnless { parsed.type == "*" || parsed.subtype == "*" }
        ?: throw SdkException(FailureKind.REQUEST_REPRESENTATION,"a wildcard request requires a concrete media selection")
    val actual = try { protocolMedia(result) } catch (e: IllegalArgumentException) { throw SdkException(FailureKind.REQUEST_REPRESENTATION,"invalid concrete request media",cause=e) }
    if (!parsed.accepts(actual)) throw SdkException(FailureKind.REQUEST_REPRESENTATION,"selected request media does not match its declaration")
    return result
}

internal val protocolUnicodeOrder: Comparator<String> = Comparator { a,b ->
    var i=0;var j=0;var result=0
    while (i<a.length && j<b.length) { val x=a.codePointAt(i);val y=b.codePointAt(j);if(x!=y){result=x.compareTo(y);break};i+=Character.charCount(x);j+=Character.charCount(y) }
    if(result!=0) result else if(i==a.length && j==b.length) 0 else if(i==a.length) -1 else 1
}
internal fun protocolPercent(value: String, mode: String, location: String, style: String?, composite: Boolean, budget: ProtocolBudget): String {
    // The wire budget owns this classification: resource exhaustion while
    // sizing a protocol field is a request limit, not a JSON evaluation
    // failure. Rethrowing SdkException keeps the operation id and source
    // location attached by the operation wrapper.
    try { Json.utf8Size(value,budget.maximum,budget.checkpoint) } catch (error: JsonException) {
        if (error.kind == JsonErrorKind.RESOURCE_LIMIT) throw SdkException(FailureKind.REQUEST_LIMIT, "protocol byte limit exceeded", cause = error) else throw error }
    if (mode == "none") {
        if (value.any { it < ' ' && !(location == "header" && it == '\t') || it == '\u007f' }) throw SdkException(FailureKind.REQUEST_REPRESENTATION,"control character in protocol field")
        if (location == "cookie" && value.any { it.code > 126 || it in " \t\",;\\" }) throw SdkException(FailureKind.REQUEST_REPRESENTATION,"cookie value needs caller-defined escaping")
        return budget.text(value)
    }
    if (style == "spaceDelimited" && value.contains(' ') || style == "pipeDelimited" && value.contains('|') || style == "deepObject" && value.any { it == '[' || it == ']' }) throw SdkException(FailureKind.REQUEST_REPRESENTATION,"data contains an ambiguous style delimiter")
    if (mode == "reserved-expansion") {
        val hazard = when(location) { "path" -> "#[]/?"; "query","querystring" -> "#[]&=+"; "cookie" -> ";,"; else -> "" }
        val delimiters = if (!composite) "" else when(style) { "simple","form","cookie" -> ","; "label" -> ".,"; "matrix" -> ";,"; else -> "" }
        if (value.any { it in hazard || it in delimiters }) throw SdkException(FailureKind.REQUEST_REPRESENTATION,"allowReserved data requires explicit pre-escaping")
    }
    val bytes=value.toByteArray(Charsets.UTF_8);val out=StringBuilder();var i=0
    while(i<bytes.size) {
        budget.checkpoint();val b=bytes[i].toInt() and 255;val c=b.toChar()
        if(mode=="reserved-expansion" && c=='%' && i+2<bytes.size && bytes[i+1].toInt().toChar() in "0123456789abcdefABCDEF" && bytes[i+2].toInt().toChar() in "0123456789abcdefABCDEF") { out.append('%').append(bytes[++i].toInt().toChar()).append(bytes[++i].toInt().toChar());budget.spend(3) }
        else if(b in 48..57||b in 65..90||b in 97..122||c in (if(mode=="form-url-encoded") "*-._" else "-._~") || mode=="reserved-expansion" && c in ":/?#[]@!$&'()*+,;=") {out.append(c);budget.spend(1)}
        else if(mode=="form-url-encoded" && c==' '){out.append('+');budget.spend(1)}
        else{out.append('%').append("0123456789ABCDEF"[b ushr 4]).append("0123456789ABCDEF"[b and 15]);budget.spend(3)}
        i++
    }
    return out.toString()
}
internal fun protocolScalar(value: JsonValue, type: String? = null): String = when(value) {
    is JsonString -> if(type==null||type=="string") value.value else throw SdkException(FailureKind.REQUEST_REPRESENTATION,"scalar type mismatch")
    is JsonBoolean -> if(type==null||type=="boolean") value.value.toString() else throw SdkException(FailureKind.REQUEST_REPRESENTATION,"scalar type mismatch")
    is JsonNumber -> if(type==null||type=="number"||type=="integer"&&value.isInteger()) value.token else throw SdkException(FailureKind.REQUEST_REPRESENTATION,"scalar type mismatch")
    else -> throw SdkException(FailureKind.REQUEST_REPRESENTATION,"protocol scalar must be non-null")
}
internal fun serializeParameter(name: String, location: String, serialization: JsonObject, value: JsonValue, budget: ProtocolBudget): String {
    val mode=serialization.text("percent_encoding")
    if(serialization.text("kind")=="content") {
        val media=protocolMedia(serialization.obj("media_type").text("declared"),true)
        val raw=if(media.subtype=="json"||media.subtype.endsWith("+json")) Json.stringifyChecked(value,JsonLimits(maxBytes=budget.maximum),budget.checkpoint) else protocolScalar(value)
        val encoded=protocolPercent(raw,mode,location,null,false,budget)
        return if(location=="query"||location=="cookie") protocolPercent(name,"uri-component",location,null,false,budget)+"="+encoded else encoded
    }
    val style=serialization.text("style");val explode=serialization.flag("explode");val shape=serialization.obj("shape");val kind=shape.text("kind")
    val key=protocolPercent(name,if(location=="header"||style=="cookie")"none" else "uri-component",location,null,false,budget)
    fun enc(s:String)=protocolPercent(s,mode,location,style,kind!="scalar",budget)
    if(kind=="scalar") {
        val raw=protocolScalar(value,shape.text("scalar"));val scalar=if(location=="path"&&style=="simple"&&(raw=="."||raw=="..")) raw.map {"%2E"}.joinToString("") else enc(raw)
        return when(style){"simple"->scalar;"label"->".$scalar";"matrix"->";"+key+if(scalar.isEmpty())"" else "=$scalar";else->"$key=$scalar"}
    }
    val items=mutableListOf<String>();val properties=mutableListOf<Pair<String,String>>()
    if(kind=="array") {
        val array=value as? JsonArray ?: throw SdkException(FailureKind.REQUEST_REPRESENTATION,"array wire shape required")
        if(array.values.isEmpty()) throw SdkException(FailureKind.REQUEST_REPRESENTATION,"empty composite has no value expansion; omit optional parameter")
        for(item in array.values) items.add(enc(protocolScalar(item,shape.text("items"))))
    } else {
        val obj=value as? JsonObject ?: throw SdkException(FailureKind.REQUEST_REPRESENTATION,"flat object wire shape required")
        if(obj.values.isEmpty()) throw SdkException(FailureKind.REQUEST_REPRESENTATION,"empty object has no value expansion")
        val declared=shape.obj("properties").values;val extra=shape.obj("additional")
        for(k in obj.values.keys.sortedWith(protocolUnicodeOrder)) {
            val scalar=(declared[k] as? JsonString)?.value ?: when(extra.text("kind")){"forbidden"->throw SdkException(FailureKind.REQUEST_REPRESENTATION,"undeclared wire object member");"typed"->extra.text("scalar");else->null}
            properties.add(enc(k) to enc(protocolScalar(obj.values.getValue(k),scalar)))
        }
    }
    fun flat(delim:String)=properties.flatMap {listOf(it.first,it.second)}.joinToString(delim)
    fun pairs(delim:String)=properties.joinToString(delim){"${it.first}=${it.second}"}
    val result=when(style){
        "simple"->if(kind=="array")items.joinToString(",")else if(explode)pairs(",")else flat(",")
        "label"->"."+if(kind=="array")items.joinToString(if(explode)"."else",")else if(explode)pairs(".")else flat(",")
        "matrix"->if(kind=="array"&&explode)items.joinToString(""){";$key"+if(it.isEmpty())""else"=$it"}else if(explode)properties.joinToString(""){";${it.first}"+if(it.second.isEmpty())""else"=${it.second}"}else";$key="+if(kind=="array")items.joinToString(",")else flat(",")
        "form","cookie"->{val d=if(style=="cookie")"; "else"&";if(kind=="array"&&explode)items.joinToString(d){"$key=$it"}else if(explode)pairs(d)else"$key="+if(kind=="array")items.joinToString(",")else flat(",")}
        "spaceDelimited","pipeDelimited"->"$key="+if(kind=="array")items.joinToString(if(style=="spaceDelimited")"%20"else"%7C")else flat(if(style=="spaceDelimited")"%20"else"%7C")
        "deepObject"->properties.joinToString("&"){"$key%5B${it.first}%5D=${it.second}"}
        else->throw SdkException(FailureKind.REQUEST_REPRESENTATION,"unsupported planned serialization")
    }
    if(result.length>budget.maximum)throw SdkException(FailureKind.REQUEST_LIMIT,"serialized parameter exceeds byte limit")
    return result
}

internal fun protocolHeader(headers: MutableMap<String,String>, name: String, value: String) {
    if(name.isEmpty()||name.any { it.code>127||!it.isLetterOrDigit()&&it !in "!#$%&'*+-.^_`|~" }||value.any { it<' '&&it!='\t'||it=='\u007f'||it.code>255 }) throw SdkException(FailureKind.REQUEST_REPRESENTATION,"invalid HTTP header")
    if(headers.keys.any {it.equals(name,true)})throw SdkException(FailureKind.REQUEST_REPRESENTATION,"duplicate generated HTTP header")
    if(headers.size>=128||headers.entries.sumOf {it.key.length+it.value.length}+name.length+value.length>32768)throw SdkException(FailureKind.REQUEST_LIMIT,"request header limit exceeded")
    headers[name]=value
}
internal fun protocolToken(value: String): Boolean = value.isNotEmpty() && value.length <= 128 && value.all { it.code < 128 && (it.isLetterOrDigit() || it in "!#$%&'*+-.^_`|~") }
internal fun protocolApplicationIdentity(value: String): Boolean {
    val at=value.indexOf('/')
    return if(at<0)protocolToken(value)else protocolToken(value.substring(0,at))&&protocolToken(value.substring(at+1))
}

/** ua/v1 attribution: an explicit non-empty override wins, an explicit empty value suppresses the header entirely, and the default identifies suspect as the generator and the SDK package or a caller-supplied application as the client. */
internal fun resolveUserAgent(options: ClientOptions): String? {
    val explicit=options.userAgent
    if(explicit!=null)return if(explicit.isEmpty())null else explicit
    if(ATTRIBUTION_SUSPECT_VERSION.isEmpty())return null
    val application=options.applicationId
    val identity=if(application.isNullOrEmpty())"$ATTRIBUTION_SDK_NAME/$ATTRIBUTION_SDK_VERSION" else {
        if(application.length>128||!protocolApplicationIdentity(application))return null
        application
    }
    return "suspect/$ATTRIBUTION_SUSPECT_VERSION $identity (kotlin/${System.getProperty("java.version") ?: "unknown"}; openapi/$ATTRIBUTION_SPEC_VERSION)"
}
internal fun protocolServer(op:JsonObject, options:ClientOptions, call:RequestOptions, budget:ProtocolBudget):URI {
    options.serverUrl?.let { return it }
    val servers=op.obj("servers").array("candidates");val index=call.serverIndex?:options.serverIndex
    if(index !in servers.indices)throw SdkException(FailureKind.REQUEST_REPRESENTATION,"server choice is out of range")
    val server=servers[index] as JsonObject;val variables=server.list("variables").map{it as JsonObject};val overrides=call.serverVariables?:options.serverVariables
    if(overrides.keys.any { key->variables.none {it.text("name")==key} })throw SdkException(FailureKind.REQUEST_REPRESENTATION,"unknown server variable")
    val template=server.text("template");val out=StringBuilder();var at=0
    while(at<template.length){val open=template.indexOf('{',at);if(open<0){out.append(budget.text(template.substring(at)));break};out.append(budget.text(template.substring(at,open)));val close=template.indexOf('}',open);check(close>=0);val key=template.substring(open+1,close);val variable=variables.first {it.text("name")==key};val value=overrides[key]?:variable.obj("default").text("value");val choices=variable.list("values");if(choices.isNotEmpty()&&choices.none{(it as JsonObject).text("value")==value})throw SdkException(FailureKind.REQUEST_REPRESENTATION,"server variable is not an enum member");out.append(budget.text(value));at=close+1}
    val raw=out.toString();if(raw.any{it.isWhitespace()||it<' '||it in "\\?#{}"})throw SdkException(FailureKind.REQUEST_REPRESENTATION,"invalid expanded server URL")
    val uri=try {URI(raw)}catch(e:Exception){throw SdkException(FailureKind.REQUEST_REPRESENTATION,"invalid expanded server URL",cause=e)}
    val resolved=if(uri.isAbsolute)uri else {
        val declared=(server.optional("document_base") as? JsonObject)?.obj("source")?.text("document")
            ?:(server.optional("source") as? JsonObject)?.obj("terminal")?.obj("source")?.text("document")
            ?:server.obj("default_from").obj("source").text("document")
        val base=call.documentUrl?:options.documentUrl?:runCatching{URI(declared)}.getOrNull()
        if(base==null||base.scheme?.lowercase() !in listOf("http","https"))throw SdkException(FailureKind.REQUEST_REPRESENTATION,"relative server requires an HTTP document URL")
        budget.text(base.toASCIIString())
        resolveDocumentServer(base,uri)
    }
    if(resolved.scheme?.lowercase() !in listOf("http","https")||resolved.host==null||resolved.rawUserInfo!=null||resolved.rawQuery!=null||resolved.rawFragment!=null||resolved.port !in -1..65535)throw SdkException(FailureKind.REQUEST_REPRESENTATION,"server must be an HTTP URI without userinfo, query or fragment")
    return resolved
}

internal suspend fun protocolAuth(op:JsonObject, values:Map<String,ProtocolCredential>, selected:Int?, headers:MutableMap<String,String>, query:MutableList<String>, cookies:MutableList<String>, budget:ProtocolBudget, server:URI__ENV_MODE_DECL__) {
    val security=op.obj("security");if(security.text("kind")!="alternatives"){if(selected!=null)throw SdkException(FailureKind.AUTHENTICATION,"anonymous operation has no security alternatives");return}
    val alternatives=security.array("alternatives").map{it as JsonObject}
    fun key(c:JsonObject)=protocolAddress(c.obj("scheme").obj(__CREDENTIAL_IDENTITY__).obj("source"))
    val index=selected?:alternatives.indexOfFirst{a->a.array("requirements").all{values.containsKey(key(it as JsonObject))}}
    if(index !in alternatives.indices)throw SdkException(FailureKind.AUTHENTICATION,"no complete source security alternative is available")
    for(item in alternatives[index].array("requirements")) {
        val requirement=item as JsonObject;val credential=values[key(requirement)]?:throw SdkException(FailureKind.AUTHENTICATION,"security alternative is incomplete")
        val hook=requirement.obj("credential");val name=requirement.text("name")
        when(hook.text("kind")) {
            "bearer"->protocolHeader(headers,"Authorization",bearer((credential as ProtocolCredential.Token).value))
            "basic"->{
                val basic=(credential as ProtocolCredential.Basic).value
                if(basic.username.contains(':')||basic.username.any{it<' '||it=='\u007f'}||basic.password.any{it<' '||it=='\u007f'})throw SdkException(FailureKind.AUTHENTICATION,"Basic credentials contain a forbidden delimiter or control")
                budget.text(basic.username);budget.text(basic.password)
                val text=basic.username+":"+basic.password
                protocolHeader(headers,"Authorization","Basic "+Base64.getEncoder().encodeToString(text.toByteArray(Charsets.UTF_8)))
            }
            "api-key"->{val token=(credential as ProtocolCredential.Token).value;budget.text(token);val field=hook.obj("name").text("value");when(hook.text("location")){"header"->protocolHeader(headers,field,token);"query"->query.add(protocolPercent(field,"uri-component","query",null,false,budget)+"="+protocolPercent(token,"uri-component","query",null,false,budget));"cookie"->cookies.add(protocolPercent(field,"none","cookie",null,false,budget)+"="+protocolPercent(token,"none","cookie",null,false,budget));else->error("unknown API key location")}}
            "o-auth2","oauth2","open-id-connect"->{
                val permissions=requirement.obj("permissions");val names=permissions.list("names").map{(it as JsonObject).text("value")};val source=protocolSource(requirement.obj("scheme").obj("terminal").obj("source"))
                val context=CredentialContext(op.obj("operation_id").text("value"),name,if(permissions.text("kind")=="scopes")names else emptyList(),if(permissions.text("kind")=="roles")names else emptyList(),source,protocolMetadata(hook),server)
                val value=try { (credential as ProtocolCredential.Provider).value.authorization(context) }
                    catch(cancelled:CancellationException){throw cancelled}
                    catch(error:Exception){throw SdkException(FailureKind.AUTHENTICATION,"credential hook failed",cause=error)}
                if(value.isBlank()||!value.contains(' '))throw SdkException(FailureKind.AUTHENTICATION,"credential hook must return a complete Authorization value")
                protocolHeader(headers,"Authorization",value)
            }
            else->throw SdkException(FailureKind.AUTHENTICATION,"unsupported credential hook")
        }
    }
}

internal fun jsonScalarText(raw:String,type:String):JsonValue = when(type) {
    "string"->JsonString(raw)
    "boolean"->when(raw){"true"->JsonBoolean(true);"false"->JsonBoolean(false);else->throw SdkException(FailureKind.RESPONSE_VALIDATION,"invalid boolean protocol value")}
    "integer","number"->try {JsonNumber.parse(raw)}catch(e:JsonException){throw SdkException(FailureKind.RESPONSE_VALIDATION,"invalid numeric protocol value",cause=e)}
    else->throw SdkException(FailureKind.RESPONSE_VALIDATION,"untyped protocol scalar")
}
internal fun decodeHeader(header:JsonObject, values:List<String>):JsonValue {
    val serialization=header.obj("serialization")
    if(serialization.text("kind")=="content") {
        if(values.size!=1)throw SdkException(FailureKind.RESPONSE_METADATA,"content header must have one value")
        val media=protocolMedia(serialization.obj("media_type").text("declared"),true)
        return if(media.subtype=="json"||media.subtype.endsWith("+json"))Json.parse(values.single())else jsonScalarText(values.single(),header.obj("content_media").obj("representation").text("scalar"))
    }
    val shape=serialization.obj("shape");val kind=shape.text("kind")
    if(kind=="scalar") {if(values.size!=1)throw SdkException(FailureKind.RESPONSE_METADATA,"scalar header must have one value");return jsonScalarText(values.single().trim(),shape.text("scalar"))}
    val elements=values.flatMap{it.split(',')}.map{it.trim()}
    if(kind=="array")return JsonArray(elements.map{jsonScalarText(it,shape.text("items"))})
    val pairs=if(serialization.flag("explode"))elements.map{val p=it.split('=',limit=2);if(p.size!=2)throw SdkException(FailureKind.RESPONSE_VALIDATION,"invalid exploded header object");p[0] to p[1]}else{if(elements.size%2!=0)throw SdkException(FailureKind.RESPONSE_VALIDATION,"invalid header object");elements.chunked(2).map{it[0] to it[1]}}
    val declared=shape.obj("properties").values;val extra=shape.obj("additional");val result=linkedMapOf<String,JsonValue>()
    for((k,v)in pairs){if(result.containsKey(k))throw SdkException(FailureKind.RESPONSE_VALIDATION,"duplicate header object key");val type=(declared[k]as?JsonString)?.value?:if(extra.text("kind")=="typed")extra.text("scalar")else throw SdkException(FailureKind.RESPONSE_VALIDATION,"header extra lacks a scalar type");result[k]=jsonScalarText(v,type)}
    return JsonObject(result)
}
internal fun typedHeaders(plans:List<JsonValue>, raw:Map<String,List<String>>):Map<String,JsonValue> {
    val out=linkedMapOf<String,JsonValue>()
    for(item in plans){val header=item as JsonObject;val name=header.text("name");val values=raw.entries.filter{it.key.equals(name,true)}.flatMap{it.value};if(values.isEmpty()){if(header.flag("required"))throw SdkException(FailureKind.RESPONSE_VALIDATION,"required response header is absent")}else out[name]=decodeHeader(header,values)}
    return out
}

internal object ProtocolRuntime {
    suspend fun prepare(op:JsonObject,input:PreparedInput,credentials:Map<String,ProtocolCredential>,options:ClientOptions,call:RequestOptions,control:CallContext__ENV_MODE_DECL__):HttpRequest {
        call.responseMedia?.let{protocolMedia(it,true)}
        val budget=ProtocolBudget(options.maxRequestBytes,control::check);val server=protocolServer(op,options,call,budget)
        val paths=linkedMapOf<String,String>();val query=mutableListOf<String>();val cookies=mutableListOf<String>();val headers=linkedMapOf<String,String>()
        for((index,raw)in op.array("parameters").withIndex()) {
            val parameter=raw as JsonObject;val value=input.parameters[index]?:continue;val location=parameter.text("location")
            val content=parameter.optional("content_media") as? JsonObject
            val encoded=if(content?.obj("representation")?.text("kind")=="form") encodeFormJson(content.obj("representation").obj("form"),value,budget) else serializeParameter(parameter.text("name"),location,parameter.obj("serialization"),value,budget)
            when(location){"path"->paths[parameter.text("name")]=encoded;"query","querystring"->query.add(encoded);"header"->protocolHeader(headers,parameter.text("name"),encoded);"cookie"->cookies.add(encoded);else->error("unsupported planned location")}
        }
        protocolAuth(op,credentials,call.securityAlternative,headers,query,cookies,budget,server__ENV_MODE_PASS__)
        if(cookies.isNotEmpty())protocolHeader(headers,"Cookie",cookies.joinToString("; "))
        var path=op.text("path");for((key,value)in paths)path=path.replace("{$key}",value)
        val url=server.toASCIIString().removeSuffix("/")+path+(if(query.isEmpty())""else"?"+query.joinToString("&"))
        if(url.length>options.maxRequestBytes)throw SdkException(FailureKind.REQUEST_LIMIT,"assembled URL exceeds limit")
        var bytes:ByteArray?=null
        if(input.body!=null) {
            val media=op.obj("body").array("media");val declaration=media.getOrNull(input.body.media)as?JsonObject?:throw SdkException(FailureKind.REQUEST_REPRESENTATION,"unknown request media variant")
            var actual=concreteMedia(declaration,call.requestMedia)
            if(selectMedia(media,actual)!=input.body.media)throw SdkException(FailureKind.REQUEST_REPRESENTATION,"a more specific request media declaration applies")
            val encoded=encodeBody(declaration,input.body.value,actual,ProtocolBudget(options.maxRequestBytes,control::check));bytes=encoded.first;actual=encoded.second
            val prior=headers.entries.firstOrNull{it.key.equals("Content-Type",true)}
            if(prior!=null && !prior.value.equals(actual,true))throw SdkException(FailureKind.REQUEST_REPRESENTATION,"Content-Type parameter conflicts with request body")
            if(prior==null)protocolHeader(headers,"Content-Type",actual)
        } else if((op.optional("body")as?JsonObject)?.flag("required")==true)throw SdkException(FailureKind.REQUEST_VALIDATION,"required request body is absent")
        if(headers.keys.none{it.equals("Accept",true)}) {
            val choices=op.array("responses").flatMap{(it as JsonObject).list("media")}.map{(it as JsonObject).obj("media_type").text("declared")}.distinct()
            val accept=call.responseMedia?:choices.joinToString(", ")
            if(accept.isNotEmpty())protocolHeader(headers,"Accept",accept)
        }
        // ua/v1 attribution is applied after declared parameters so an explicit caller-supplied User-Agent header parameter keeps precedence over the automatic value.
        if(headers.keys.none{it.equals("User-Agent",true)})resolveUserAgent(options)?.let{protocolHeader(headers,"User-Agent",it)}
        control.check()
        return HttpRequest(op.text("method"),URI(url),headers,bytes,call.timeout?:options.timeout,options.maxResponseBytes)
    }
    fun select(op:JsonObject,status:Int):Int {
        if(status !in 100..599)throw SdkException(FailureKind.RESPONSE_METADATA,"invalid HTTP status")
        var found=-1;var rank=0
        for((i,item)in op.array("responses").withIndex()){val s=(item as JsonObject).obj("status");val r=when(s.text("kind")){"exact"->if(s.number("value")==status)3 else 0;"range"->if(s.number("value")==status/100)2 else 0;"default"->1;else->0};if(r>rank){found=i;rank=r}}
        if(found<0)throw SdkException(FailureKind.UNEXPECTED_STATUS,"response status is not declared")
        return found
    }
    fun forbidden(op:JsonObject,status:Int)=bodyForbidden(op.text("method"),status)
    fun response(op:JsonObject,response:HttpResponse,info:ResponseInfo,call:RequestOptions,control:CallContext):ProtocolResponse {
        val ri=select(op,response.status);val declaration=op.array("responses")[ri]as JsonObject
        val headers=typedHeaders(declaration.list("headers"),response.headers)
        val links=declaration.list("links").map{val link=it as JsonObject;LinkMetadata(link.text("name"),protocolMetadata(link))}
        if(forbidden(op,response.status))return ProtocolResponse(ri,null,true,ProtocolValue.None,headers,info.withProtocol(null,links))
        val media=declaration.list("media")
        if(media.isEmpty())return ProtocolResponse(ri,null,false,ProtocolValue.Bytes(response.body.copyOf()),headers,info.withProtocol(null,links))
        val types=response.headers.entries.filter{it.key.equals("content-type",true)}.flatMap{it.value}
        if(types.size!=1)throw SdkException(FailureKind.CONTENT_TYPE,"one Content-Type is required",info)
        val encodings=response.headers.entries.filter{it.key.equals("content-encoding",true)}.flatMap{it.value}
        if(encodings.size>1||encodings.any{!it.trim().equals("identity",true)})throw SdkException(FailureKind.CONTENT_TYPE,"unsupported Content-Encoding",info)
        val actual=types.single();val mi=selectMedia(media,actual)
        if(response.status in 200..299 && call.responseMedia!=null && !protocolMedia(call.responseMedia,true).accepts(protocolMedia(actual)))throw SdkException(FailureKind.CONTENT_TYPE,"response differs from requested representation",info)
        val value=decodeBody(media[mi]as JsonObject,response.body,actual,control::check)
        return ProtocolResponse(ri,mi,false,value,headers,info.withProtocol(actual,links))
    }
}

internal inline fun <T> responseWork(info: ResponseInfo, block: () -> T): T = try { block() }
catch (error: ValidationException) { throw SdkException(FailureKind.RESPONSE_VALIDATION,"response schema validation failed",info,error) }
catch (error: EvaluationException) { throw SdkException(FailureKind.EVALUATION,"response evaluation did not complete",info,error) }
catch (error: JsonException) { throw SdkException(if(error.kind==JsonErrorKind.RESOURCE_LIMIT)FailureKind.EVALUATION else FailureKind.RESPONSE_VALIDATION,"response representation failed",info,error) }
catch (error: SdkException) {
    val mapped = SdkException(when(error.kind){FailureKind.REQUEST_VALIDATION,FailureKind.REQUEST_REPRESENTATION->FailureKind.RESPONSE_VALIDATION;FailureKind.REQUEST_LIMIT->FailureKind.RESPONSE_LIMIT;else->error.kind},error.message?:"response failed",error.response?:info,error.cause)
    error.suppressed.forEach(mapped::addSuppressed)
    throw mapped
}
catch (error: IllegalArgumentException) { throw SdkException(FailureKind.RESPONSE_VALIDATION,"malformed response representation",info,error) }
