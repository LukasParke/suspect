package __PACKAGE__

import java.io.ByteArrayOutputStream
import java.nio.ByteBuffer
import java.nio.charset.CodingErrorAction
import java.util.UUID

internal fun strictText(bytes:ByteArray):String = try {
    Charsets.UTF_8.newDecoder().onMalformedInput(CodingErrorAction.REPORT).onUnmappableCharacter(CodingErrorAction.REPORT).decode(ByteBuffer.wrap(bytes)).toString()
} catch (_:java.nio.charset.CharacterCodingException) { throw JsonException("invalid UTF-8",0) }

internal fun protocolBound(bytes:ByteArray, maximum:Int) {
    if(bytes.size>maximum)throw SdkException(FailureKind.RESPONSE_LIMIT,"payload exceeds its declared byte bound")
}
internal fun encodeBody(media:JsonObject,value:ProtocolValue,actual:String,budget:ProtocolBudget):Pair<ByteArray,String> {
    val representation=media.obj("representation")
    return when(representation.text("kind")) {
        "json"->{val bytes=Json.stringifyChecked((value as ProtocolValue.Json).value,JsonLimits(maxBytes=budget.maximum),budget.checkpoint).toByteArray();budget.spend(bytes.size);bytes to actual}
        "text"->{val text=protocolScalar((value as ProtocolValue.Json).value,representation.text("scalar"));budget.text(text);text.toByteArray(Charsets.UTF_8) to actual}
        "binary"->{val bytes=(value as ProtocolValue.Bytes).value;val maximum=representation.obj("bytes").number("max_bytes");if(bytes.size>maximum)throw SdkException(FailureKind.REQUEST_LIMIT,"binary request exceeds source byte bound");budget.spend(bytes.size);bytes.copyOf() to actual}
        "form"->{val text=encodeForm(representation.obj("form"),(value as ProtocolValue.Parts).values,budget);text.toByteArray() to actual}
        "multipart"->encodeMultipart(representation.obj("multipart"),(value as ProtocolValue.Parts).values,actual,budget)
        "stream"->{val bytes=(value as ProtocolValue.Bytes).value;budget.spend(bytes.size);bytes to actual}
        else->throw SdkException(FailureKind.REQUEST_REPRESENTATION,"request representation is not implemented")
    }
}

internal fun structural(rules:JsonObject, fields:List<PreparedPart>) {
    if(fields.size>100000)throw SdkException(FailureKind.REQUEST_LIMIT,"too many form fields")
    val names=fields.map{it.name}.toSet()
    if(names.size!=fields.size)throw SdkException(FailureKind.REQUEST_REPRESENTATION,"duplicate native form field")
    for(required in rules.list("required")){if((required as JsonObject).text("value") !in names)throw SdkException(FailureKind.REQUEST_VALIDATION,"required form property is absent")}
    val count=JsonNumber.of(names.size.toLong())
    for((key,minimum)in listOf("min_properties" to true,"max_properties" to false)) {
        val bound=(rules.optional(key)as?JsonObject)?.values?.get("value")as?JsonNumber?:continue
        if(if(minimum)count<bound else count>bound)throw SdkException(FailureKind.REQUEST_VALIDATION,"form property cardinality violates source rules")
    }
}
internal fun fieldPlan(container:JsonObject, name:String):JsonObject {
    val fields=container.list(if(container.values.containsKey("fields"))"fields"else"parts")
    return (fields.firstOrNull{(it as JsonObject).string("name")==name} as? JsonObject) ?:run{
        val additional=container.obj("additional")
        if(additional.text("kind")!="allowed")throw SdkException(FailureKind.REQUEST_VALIDATION,"undeclared form property")
        additional.obj("part")
    }
}
internal fun checkMultiplicity(plan:JsonObject, count:Int) {
    if(plan.text("multiplicity")=="one"&&count!=1)throw SdkException(FailureKind.REQUEST_REPRESENTATION,"single form part has multiple values")
    if(plan.text("multiplicity")=="repeated-array-items"&&count==0)throw SdkException(FailureKind.REQUEST_REPRESENTATION,"present empty repeated field has no wire representation")
    val actual=JsonNumber.of(count.toLong())
    for((key,minimum)in listOf("min_items" to true,"max_items" to false)) {
        val bound=(plan.optional(key)as?JsonObject)?.values?.get("value")as?JsonNumber?:continue
        if(if(minimum)actual<bound else actual>bound)throw SdkException(FailureKind.REQUEST_VALIDATION,"repeated part violates cardinality")
    }
}
internal fun encodeFormJson(form:JsonObject,value:JsonValue,budget:ProtocolBudget):String {
    val fields=(value as?JsonObject)?.values?:throw SdkException(FailureKind.REQUEST_REPRESENTATION,"form querystring requires an object")
    if(fields.size>100000)throw SdkException(FailureKind.REQUEST_LIMIT,"too many form fields")
    val parts=fields.map{(name,member)->val plan=fieldPlan(form,name);PreparedPart(name,if(plan.text("multiplicity")=="repeated-array-items") {
        val array=member as?JsonArray?:throw SdkException(FailureKind.REQUEST_REPRESENTATION,"form array required")
        array.values.map{ProtocolPartValue(ProtocolValue.Json(it))}
    } else listOf(ProtocolPartValue(ProtocolValue.Json(member))))}
    return encodeForm(form,parts,budget)
}
internal fun encodeForm(form:JsonObject,fields:List<PreparedPart>,budget:ProtocolBudget):String {
    structural(form.obj("rules"),fields);val output=mutableListOf<String>()
    for(field in fields.sortedWith(compareBy(protocolUnicodeOrder){it.name})) {
        val plan=fieldPlan(form,field.name);checkMultiplicity(plan,field.values.size);val representation=plan.obj("representation")
        for(part in field.values) {
            val value=(part.value as?ProtocolValue.Json)?.value?:throw SdkException(FailureKind.REQUEST_REPRESENTATION,"form field has no text/JSON representation")
            val wire=when(representation.text("kind")) {
                "style"->serializeParameter(field.name,"query",representation.obj("serialization"),value,budget)
                "json","text"->{val text=if(representation.text("kind")=="json")Json.stringifyChecked(value,JsonLimits(maxBytes=budget.maximum),budget.checkpoint)else protocolScalar(value,representation.text("scalar"));protocolPercent(field.name,"form-url-encoded","query",null,false,budget)+"="+protocolPercent(text,representation.text("outer_encoding"),"query",null,false,budget)}
                else->throw SdkException(FailureKind.REQUEST_REPRESENTATION,"binary form field is unsupported")
            }
            if(output.isNotEmpty())budget.spend(1);output.add(wire)
        }
    }
    return output.joinToString("&")
}
internal fun dispositionQuote(value:String):String {
    Json.unicode(value)
    if(value.any{it<' '||it=='\u007f'||it=='\r'||it=='\n'})throw SdkException(FailureKind.REQUEST_REPRESENTATION,"invalid multipart name or filename")
    return "\""+value.replace("\\","\\\\").replace("\"","\\\"")+"\""
}
internal fun encodeMultipart(container:JsonObject,fields:List<PreparedPart>,actual:String,budget:ProtocolBudget):Pair<ByteArray,String> {
    if(container.text("kind")!="named")throw SdkException(FailureKind.REQUEST_REPRESENTATION,"positional multipart is not enabled")
    structural(container.obj("rules"),fields)
    val boundary="suspect-"+UUID.randomUUID().toString();val out=ByteArrayOutputStream()
    fun append(bytes:ByteArray){budget.spend(bytes.size);out.write(bytes)}
    fun text(value:String){budget.text(value);out.write(value.toByteArray(Charsets.UTF_8))}
    for(field in fields.sortedWith(compareBy(protocolUnicodeOrder){it.name})) {
        val plan=fieldPlan(container,field.name);checkMultiplicity(plan,field.values.size)
        for(part in field.values){
            val representation=plan.obj("representation");val types=plan.list("content_types").map{(it as JsonObject).text("declared")}
            val selected=part.contentType?:types.singleOrNull()?.takeUnless{protocolMedia(it,true).let{m->m.type=="*"||m.subtype=="*"}}
                ?:throw SdkException(FailureKind.REQUEST_REPRESENTATION,"multipart part requires a concrete content type")
            val concrete=protocolMedia(selected);if(types.none{protocolMedia(it,true).accepts(concrete)})throw SdkException(FailureKind.REQUEST_REPRESENTATION,"part Content-Type is not declared")
            Json.utf8Size(field.name,budget.maximum,budget.checkpoint)
            part.filename?.let{Json.utf8Size(it,budget.maximum,budget.checkpoint)}
            val bytes=when(representation.text("kind")) {
                "binary"->{val bytes=(part.value as?ProtocolValue.Bytes)?.value?:throw SdkException(FailureKind.REQUEST_REPRESENTATION,"multipart bytes required");if(bytes.size>representation.obj("bytes").number("max_bytes"))throw SdkException(FailureKind.REQUEST_LIMIT,"multipart byte policy exceeded");bytes}
                "json"->Json.stringifyChecked((part.value as ProtocolValue.Json).value,JsonLimits(maxBytes=budget.maximum),budget.checkpoint).toByteArray()
                "text"->protocolScalar((part.value as ProtocolValue.Json).value,representation.text("scalar")).also{Json.utf8Size(it,budget.maximum,budget.checkpoint)}.toByteArray(Charsets.UTF_8)
                "style"->serializeParameter(field.name,"header",representation.obj("serialization"),(part.value as ProtocolValue.Json).value,ProtocolBudget(budget.maximum,budget.checkpoint)).toByteArray()
                else->throw SdkException(FailureKind.REQUEST_REPRESENTATION,"unsupported part representation")
            }
            val marker=("\r\n--"+boundary).toByteArray()
            if(indexOfBytes(bytes,marker,0,budget.checkpoint)>=0)throw SdkException(FailureKind.REQUEST_REPRESENTATION,"multipart boundary occurs in part data")
            text("--$boundary\r\nContent-Disposition: form-data; name="+dispositionQuote(field.name)+(part.filename?.let{"; filename="+dispositionQuote(it)}?:"")+"\r\nContent-Type: $selected\r\n")
            val headers=linkedMapOf<String,String>()
            for(raw in plan.list("headers")){val header=raw as JsonObject;val name=header.text("name");val value=part.headers[name];if(value==null){if(header.flag("required"))throw SdkException(FailureKind.REQUEST_VALIDATION,"required part header is absent")}else protocolHeader(headers,name,serializeParameter(name,"header",header.obj("serialization"),value,ProtocolBudget(32768,budget.checkpoint)))}
            for((name,value)in headers){if(name.equals("content-type",true)||name.equals("content-disposition",true)||name.equals("content-length",true)||name.equals("content-transfer-encoding",true))throw SdkException(FailureKind.REQUEST_REPRESENTATION,"part header conflicts with framing");text("$name: $value\r\n")}
            text("\r\n");append(bytes);text("\r\n")
        }
    }
    text("--$boundary--\r\n")
    val parsed=protocolMedia(actual)
    if(parsed.parameters.containsKey("boundary"))throw SdkException(FailureKind.REQUEST_REPRESENTATION,"multipart boundary is runtime-owned")
    return out.toByteArray() to "$actual; boundary=$boundary"
}
internal fun decodeBody(media:JsonObject,bytes:ByteArray,actual:String,checkpoint:()->Unit):ProtocolValue {
    checkpoint();val representation=media.obj("representation")
    return when(representation.text("kind")) {
        "json"->ProtocolValue.Json(Json.parseChecked(bytes,JsonLimits(),checkpoint))
        "text"->ProtocolValue.Json(jsonScalarText(strictText(bytes),representation.text("scalar")))
        "binary"->{protocolBound(bytes,representation.obj("bytes").number("max_bytes"));ProtocolValue.Bytes(bytes.copyOf())}
        "form"->ProtocolValue.Parts(decodeForm(representation.obj("form"),bytes,checkpoint))
        "multipart"->ProtocolValue.Parts(decodeMultipart(representation.obj("multipart"),bytes,actual,checkpoint))
        else->throw SdkException(FailureKind.RESPONSE_VALIDATION,"item stream must be consumed through its Flow")
    }
}
internal fun decodePercent(value:String,form:Boolean):String {
    val out=ByteArrayOutputStream();var i=0
    while(i<value.length){val c=value[i++];when{c=='%'->{if(i+1>=value.length)throw SdkException(FailureKind.RESPONSE_VALIDATION,"short percent escape");val a=value[i++].digitToIntOrNull(16);val b=value[i++].digitToIntOrNull(16);if(a==null||b==null)throw SdkException(FailureKind.RESPONSE_VALIDATION,"invalid percent escape");out.write(a*16+b)};form&&c=='+'->out.write(32);c.code<128->out.write(c.code);else->{val start=i-1;if(c.isHighSurrogate()){if(i>=value.length||!value[i].isLowSurrogate())throw JsonException("unpaired surrogate",i);i++};out.write(value.substring(start,i).toByteArray(Charsets.UTF_8))}}}
    return strictText(out.toByteArray())
}
internal fun decodeForm(form:JsonObject,bytes:ByteArray,checkpoint:()->Unit):List<PreparedPart> {
    val text=strictText(bytes);val fields=linkedMapOf<String,MutableList<ProtocolPartValue>>()
    var start=0;var count=0
    while(start<text.length){
        checkpoint();if(++count>100000)throw SdkException(FailureKind.RESPONSE_LIMIT,"form field count exceeds limit")
        val end=text.indexOf('&',start).let{if(it<0)text.length else it};val field=text.substring(start,end);start=end+1
        val equal=field.indexOf('=');if(equal<0)throw SdkException(FailureKind.RESPONSE_VALIDATION,"form field lacks equals");val name=decodePercent(field.substring(0,equal),true);val plan=fieldPlan(form,name);val representation=plan.obj("representation");val raw=decodePercent(field.substring(equal+1),true)
        val value=when(representation.text("kind")){"json"->Json.parse(raw);"text"->jsonScalarText(raw,representation.text("scalar"));"style"->{val serial=representation.obj("serialization");val shape=serial.obj("shape");if(shape.text("kind")!="scalar")throw SdkException(FailureKind.RESPONSE_VALIDATION,"composite form style needs an unambiguous response profile");jsonScalarText(raw,shape.text("scalar"))};else->throw SdkException(FailureKind.RESPONSE_VALIDATION,"unsupported form field")}
        fields.getOrPut(name){mutableListOf()}.add(ProtocolPartValue(ProtocolValue.Json(value)))
    }
    val result=fields.map{PreparedPart(it.key,it.value)};structural(form.obj("rules"),result);for(field in result)checkMultiplicity(fieldPlan(form,field.name),field.values.size);return result
}
internal fun indexOfBytes(value:ByteArray,needle:ByteArray,start:Int,checkpoint:()->Unit={}):Int {
    if(needle.isEmpty())return start
    var i=start
    while(i<=value.size-needle.size){if(i%1024==0)checkpoint();var j=0;while(j<needle.size&&value[i+j]==needle[j])j++;if(j==needle.size)return i;i++}
    return -1
}
internal fun multipartBoundary(bytes:ByteArray,marker:ByteArray,start:Int,checkpoint:()->Unit):Int {
    var at=start
    while(true){
        at=indexOfBytes(bytes,marker,at,checkpoint);if(at<0)return -1
        if(at==0||at>=2&&bytes[at-2]==13.toByte()&&bytes[at-1]==10.toByte()){
            var tail=at+marker.size
            val closing=tail+1<bytes.size&&bytes[tail]==45.toByte()&&bytes[tail+1]==45.toByte()
            if(closing)tail+=2
            while(tail<bytes.size&&(bytes[tail]==32.toByte()||bytes[tail]==9.toByte()))tail++
            if(closing&&tail==bytes.size||tail+1<bytes.size&&bytes[tail]==13.toByte()&&bytes[tail+1]==10.toByte())return at
        }
        at++
    }
}
internal fun decodeMultipart(container:JsonObject,bytes:ByteArray,actual:String,checkpoint:()->Unit):List<PreparedPart> {
    val boundary=protocolMedia(actual).parameters["boundary"]?:throw SdkException(FailureKind.RESPONSE_VALIDATION,"multipart boundary missing")
    if(boundary.isEmpty()||boundary.length>70||boundary.endsWith(' ')||boundary.any{!it.isLetterOrDigit()&&it !in "'()+_,-./:=? "}||boundary.any{it.code>127})throw SdkException(FailureKind.RESPONSE_VALIDATION,"invalid multipart boundary")
    val marker=("--"+boundary).toByteArray();val headerEnd="\r\n\r\n".toByteArray()
    var at=multipartBoundary(bytes,marker,0,checkpoint);if(at<0)throw SdkException(FailureKind.RESPONSE_VALIDATION,"multipart boundary not found")
    val fields=linkedMapOf<String,MutableList<ProtocolPartValue>>()
    while(true) {
        checkpoint();at+=marker.size
        if(at+1<bytes.size&&bytes[at]==45.toByte()&&bytes[at+1]==45.toByte())break
        while(at<bytes.size&&(bytes[at]==32.toByte()||bytes[at]==9.toByte()))at++
        if(at+1>=bytes.size||bytes[at]!=13.toByte()||bytes[at+1]!=10.toByte())throw SdkException(FailureKind.RESPONSE_VALIDATION,"invalid multipart delimiter")
        at+=2;val end=indexOfBytes(bytes,headerEnd,at,checkpoint);if(end<0||end-at>32768)throw SdkException(FailureKind.RESPONSE_LIMIT,"multipart headers exceed limit")
        val rawHeaders=linkedMapOf<String,List<String>>()
        for(line in strictText(bytes.copyOfRange(at,end)).split("\r\n")){val colon=line.indexOf(':');if(colon<=0||line.startsWith(' ')||line.startsWith('\t'))throw SdkException(FailureKind.RESPONSE_VALIDATION,"malformed multipart header");val name=line.substring(0,colon);val value=line.substring(colon+1).trim();if(rawHeaders.keys.any{it.equals(name,true)}||rawHeaders.size>=128)throw SdkException(FailureKind.RESPONSE_VALIDATION,"duplicate or excessive part headers");rawHeaders[name]=listOf(value)}
        if(boundedHeaders(rawHeaders).malformed)throw SdkException(FailureKind.RESPONSE_METADATA,"invalid multipart header field")
        fun header(name:String)=rawHeaders.entries.firstOrNull{it.key.equals(name,true)}?.value?.singleOrNull()
        if(header("Content-Transfer-Encoding")!=null)throw SdkException(FailureKind.RESPONSE_VALIDATION,"Content-Transfer-Encoding is not supported")
        val disposition=header("Content-Disposition")?:throw SdkException(FailureKind.RESPONSE_VALIDATION,"named multipart part lacks disposition")
        val parsed=protocolMedia("form/"+disposition)
        if(parsed.subtype!="form-data")throw SdkException(FailureKind.RESPONSE_VALIDATION,"expected form-data disposition")
        val name=parsed.parameters["name"]?:throw SdkException(FailureKind.RESPONSE_VALIDATION,"multipart name missing")
        val plan=fieldPlan(container,name);val contentType=header("Content-Type")?:plan.list("content_types").singleOrNull()?.let{(it as JsonObject).text("declared")}?:throw SdkException(FailureKind.RESPONSE_VALIDATION,"part Content-Type missing")
        val concrete=protocolMedia(contentType);if(plan.list("content_types").none{protocolMedia((it as JsonObject).text("declared"),true).accepts(concrete)})throw SdkException(FailureKind.CONTENT_TYPE,"part Content-Type is not declared")
        at=end+4;val next=multipartBoundary(bytes,marker,at,checkpoint);if(next<at+2)throw SdkException(FailureKind.RESPONSE_VALIDATION,"unterminated multipart part")
        val payload=bytes.copyOfRange(at,next-2);val rep=plan.obj("representation")
        val value=when(rep.text("kind")){"binary"->{protocolBound(payload,rep.obj("bytes").number("max_bytes"));ProtocolValue.Bytes(payload)};"json"->ProtocolValue.Json(Json.parseChecked(payload,JsonLimits(),checkpoint));"text"->ProtocolValue.Json(jsonScalarText(strictText(payload),rep.text("scalar")));"style"->{val temp=JsonObject(mapOf("serialization" to rep.obj("serialization")));ProtocolValue.Json(decodeHeader(temp,listOf(strictText(payload))))};else->throw SdkException(FailureKind.RESPONSE_VALIDATION,"unsupported multipart part")}
        val values=fields.getOrPut(name){mutableListOf()};if(values.size>=100000)throw SdkException(FailureKind.RESPONSE_LIMIT,"too many multipart parts")
        values.add(ProtocolPartValue(value,typedHeaders(plan.list("headers"),rawHeaders),contentType,parsed.parameters["filename"]))
        at=next
    }
    val result=fields.map{PreparedPart(it.key,it.value)};structural(container.obj("rules"),result);for(field in result)checkMultiplicity(fieldPlan(container,field.name),field.values.size);return result
}
