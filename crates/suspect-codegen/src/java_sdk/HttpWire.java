package {package};

import java.io.ByteArrayOutputStream;
import java.nio.ByteBuffer;
import java.nio.charset.*;
import java.security.SecureRandom;
import java.util.*;
import static {package}.JsonRuntime.*;
import static {package}.Protocol.*;

/** Interprets the compiler's wire instructions. JSON assertions stay in ModelCodec. */
final class HttpWire {
    private HttpWire() {}
    static SdkException failure(String kind,JsonValue at) { return new SdkException(kind,source(at),0,new byte[0],false); }
    static final Comparator<String> UNICODE = (left,right) -> {
        int a=0,b=0; while(a<left.length()&&b<right.length()) { int x=left.codePointAt(a),y=right.codePointAt(b);if(x!=y)return Integer.compare(x,y);a+=Character.charCount(x);b+=Character.charCount(y); }
        return Integer.compare(left.length()-a,right.length()-b);
    };
    static final class Buffer {
        final int limit; final ModelCodec.Context context; final JsonValue source;
        final ByteArrayOutputStream out = new ByteArrayOutputStream();
        Buffer(int limit,ModelCodec.Context context,JsonValue source) { this.limit=limit;this.context=context;this.source=source; }
        void bytes(byte[] value) { bytes(value,0,value.length); }
        void bytes(byte[] value,int start,int length) {
            if(length<0||length>limit-out.size()) throw failure("resource-limit",source);
            context.spend(length); out.write(value,start,length);
        }
        void text(String value) { context.string(value);if(JsonRuntime.utf8Length(value)>limit-out.size())throw failure("resource-limit",source);bytes(value.getBytes(StandardCharsets.UTF_8)); }
        Bytes value() { return Bytes.owned(out.toByteArray()); }
        String text() { return out.toString(StandardCharsets.UTF_8); }
    }
    static String utf8(byte[] value) {
        try { return StandardCharsets.UTF_8.newDecoder().onMalformedInput(CodingErrorAction.REPORT).onUnmappableCharacter(CodingErrorAction.REPORT).decode(ByteBuffer.wrap(value)).toString(); }
        catch(CharacterCodingException error) { throw new CodecException("invalid","","","invalid UTF-8 wire text"); }
    }
    static boolean token(String value) { return !value.isEmpty() && value.chars().allMatch(c->c<128&&(Character.isLetterOrDigit(c)||"!#$%&'*+-.^_`|~".indexOf(c)>=0)); }
    static void headerValue(String value) { if(value.chars().anyMatch(c->c<32&&c!='\t'||c==127||c>255))throw new IllegalArgumentException("invalid header value"); }
    static void cookieValue(String value) { if(value.chars().anyMatch(c->c<=32||c>=127||"\",;\\".indexOf(c)>=0))throw new IllegalArgumentException("cookie value requires explicit escaping"); }
    static String scalar(JsonValue value,String kind) {
        if(value instanceof JsonString s && (kind==null||kind.equals("string"))) return s.value();
        if(value instanceof JsonBoolean b && (kind==null||kind.equals("boolean"))) return Boolean.toString(b.value());
        if(value instanceof JsonNumber n && (kind==null||kind.equals("number")||kind.equals("integer")&&n.isInteger())) return n.token();
        throw new CodecException("invalid","","","value does not match the planned non-null scalar");
    }
    static JsonValue scalarValue(String value,String kind) {
        return switch(kind) {
            case "string" -> new JsonString(value);
            case "boolean" -> { if(!value.equals("true")&&!value.equals("false"))throw new IllegalArgumentException("invalid boolean wire value");yield new JsonBoolean(value.equals("true")); }
            case "integer","number" -> { JsonNumber n=JsonNumber.parse(value);if(kind.equals("integer")&&!n.isInteger())throw new IllegalArgumentException("non-integral wire value");yield n; }
            default -> throw new IllegalArgumentException("unknown scalar wire kind");
        };
    }
    static String percent(String value,String encoding,String location,String style,boolean composite,int limit,ModelCodec.Context c,JsonValue at) {
        c.string(value);
        if(style!=null && (style.equals("spaceDelimited")&&value.indexOf(' ')>=0 || style.equals("pipeDelimited")&&value.indexOf('|')>=0 || style.equals("deepObject")&&(value.indexOf('[')>=0||value.indexOf(']')>=0)))throw failure("http-delimiter-escaping",at);
        if(encoding.equals("none")) {
            if(value.chars().anyMatch(ch->Character.isISOControl(ch)&&!(location.equals("header")&&ch=='\t')))throw failure("http-wire-control",at);
            if(location.equals("cookie"))cookieValue(value);
            if(value.getBytes(StandardCharsets.UTF_8).length>limit)throw failure("resource-limit",at);
            return value;
        }
        if(encoding.equals("reserved-expansion")) {
            String hazards=location.equals("path")?"#[]/?":location.equals("query")||location.equals("querystring")?"#[]&=+":location.equals("cookie")?";,":"";
            if(value.chars().anyMatch(ch->hazards.indexOf(ch)>=0))throw failure("http-reserved-value-escaping",at);
            if(composite&&style!=null) {
                String delimiters=switch(style){case "simple","form","cookie"->",";case "label"->".,";case "matrix"->";,";default->"";};
                if(value.chars().anyMatch(ch->delimiters.indexOf(ch)>=0))throw failure("http-reserved-value-escaping",at);
            }
        }
        Buffer out=new Buffer(limit,c,at);byte[] bytes=value.getBytes(StandardCharsets.UTF_8);
        for(int i=0;i<bytes.length;i++) {
            int b=bytes[i]&255;
            if(encoding.equals("reserved-expansion")&&b=='%'&&i+2<bytes.length&&hex(bytes[i+1])>=0&&hex(bytes[i+2])>=0){out.bytes(bytes,i,3);i+=2;continue;}
            boolean pass=b>='a'&&b<='z'||b>='A'&&b<='Z'||b>='0'&&b<='9'||(encoding.equals("form-url-encoded")?"*-._":"-._~").indexOf(b)>=0
                    ||encoding.equals("reserved-expansion")&&":/?#[]@!$&'()*+,;=".indexOf(b)>=0;
            if(pass)out.bytes(bytes,i,1);else if(encoding.equals("form-url-encoded")&&b==' ')out.text("+");else out.text("%"+"0123456789ABCDEF".charAt(b>>4)+"0123456789ABCDEF".charAt(b&15));
        }
        return out.text();
    }
    private static int hex(int ch){return ch>=0&&ch<128?Character.digit((char)ch,16):-1;}
    static String unpercent(String value,boolean plus) {
        byte[] bytes=value.getBytes(StandardCharsets.UTF_8);var result=new ByteArrayOutputStream();
        for(int i=0;i<bytes.length;i++) {int b=bytes[i]&255;if(b=='%'){if(i+2>=bytes.length||hex(bytes[i+1])<0||hex(bytes[i+2])<0)throw new IllegalArgumentException("invalid percent escape");result.write(hex(bytes[++i])*16+hex(bytes[++i]));}else result.write(plus&&b=='+'?' ':b);}
        return utf8(result.toByteArray());
    }
    static String parameter(JsonValue parameter,JsonValue value,int limit,ModelCodec.Context c) {
        String location=optionalText(parameter,"location");if(location==null)location="header";
        JsonValue content=get(parameter,"content_media");
        if(content!=JsonNull.INSTANCE&&text(get(content,"representation"),"kind").equals("form")) {
            JsonValue form=get(get(content,"representation"),"form");
            return form(form,jsonParts(form,object(value)),limit,c);
        }
        return serialize(parameter,text(parameter,"name"),location,get(parameter,"serialization"),value,limit,c);
    }
    static String serialize(JsonValue at,String name,String location,JsonValue serialization,JsonValue value,int limit,ModelCodec.Context c) {
        String encoding=text(serialization,"percent_encoding");
        if(text(serialization,"kind").equals("content")) {
            String valueText=isJson(get(serialization,"media_type"))?JsonRuntime.stringify(value,c.jsonBudget):scalar(value,null);
            String encoded=percent(valueText,encoding,location,null,false,limit,c,at);
            if(location.equals("query")||location.equals("cookie"))return joinBounded(limit,c,at,percent(name,"uri-component",location,null,false,limit,c,at),"=",encoded);
            return encoded;
        }
        String style=text(serialization,"style");boolean explode=flag(serialization,"explode");JsonValue shape=get(serialization,"shape");String kind=text(shape,"kind");
        String encodedName=percent(name,location.equals("header")||style.equals("cookie")?"none":"uri-component",location,null,false,limit,c,at);
        var items=new ArrayList<String>();var properties=new ArrayList<Map.Entry<String,String>>();String one=null;long retained=0;
        if(kind.equals("scalar"))one=percent(scalar(value,text(shape,"scalar")),encoding,location,style,false,limit,c,at);
        else if(kind.equals("array")) {
            List<JsonValue> values=((JsonArray)value).values();if(values.isEmpty())throw failure("http-empty-composite",at);
            for(JsonValue item:values){c.spend(1);String encoded=percent(scalar(item,text(shape,"items")),encoding,location,style,true,limit,c,at);retained+=encoded.length();if(retained>limit)throw failure("resource-limit",at);items.add(encoded);}
        } else {
            var values=object(value).values();if(values.isEmpty())throw failure("http-empty-composite",at);
            var keys=new ArrayList<>(values.keySet());keys.sort(UNICODE);
            for(String key:keys) {
                c.spend(1);JsonValue scalar=get(get(shape,"properties"),key);String scalarKind;
                if(scalar instanceof JsonString s)scalarKind=s.value();else {JsonValue extra=get(shape,"additional");String extraKind=text(extra,"kind");if(extraKind.equals("forbidden"))throw failure("http-wire-value",at);scalarKind=extraKind.equals("typed")?text(extra,"scalar"):null;}
                String encodedKey=percent(key,encoding,location,style,true,limit,c,at),encodedValue=percent(scalar(values.get(key),scalarKind),encoding,location,style,true,limit,c,at);retained+=(long)encodedKey.length()+encodedValue.length();if(retained>limit)throw failure("resource-limit",at);properties.add(Map.entry(encodedKey,encodedValue));
            }
        }
        Buffer out=new Buffer(limit,c,at);
        if(style.equals("simple")||style.equals("label")) {
            if(style.equals("label"))out.text(".");
            if(one!=null)out.text(one);else if(kind.equals("array"))appendJoin(out,items,style.equals("label")&&explode?".":",");
            else appendProperties(out,properties,explode,style.equals("label")&&explode?".":",");
        } else if(style.equals("matrix")) {
            if(one!=null){out.text(";");named(out,encodedName,one,true);}
            else if(explode){if(kind.equals("array")){for(String item:items){out.text(";");named(out,encodedName,item,true);}}else{for(var entry:properties){out.text(";");named(out,entry.getKey(),entry.getValue(),true);}}}
            else {out.text(";"+encodedName+"=");if(kind.equals("array"))appendJoin(out,items,",");else appendProperties(out,properties,false,",");}
        } else if(style.equals("form")||style.equals("cookie")) {
            String delimiter=style.equals("cookie")?"; ":"&";
            if(one!=null)named(out,encodedName,one,false);
            else if(explode){boolean first=true;if(kind.equals("array")){for(String item:items){if(!first)out.text(delimiter);first=false;named(out,encodedName,item,false);}}else appendProperties(out,properties,true,delimiter);}
            else {out.text(encodedName+"=");if(kind.equals("array"))appendJoin(out,items,",");else appendProperties(out,properties,false,",");}
        } else if(style.equals("spaceDelimited")||style.equals("pipeDelimited")) {
            out.text(encodedName+"=");String delimiter=style.equals("spaceDelimited")?"%20":"%7C";if(kind.equals("array"))appendJoin(out,items,delimiter);else appendProperties(out,properties,false,delimiter);
        } else if(style.equals("deepObject")) {
            boolean first=true;for(var entry:properties){if(!first)out.text("&");first=false;out.text(encodedName+"%5B"+entry.getKey()+"%5D="+entry.getValue());}
        } else throw failure("unsupported-wire-instruction",at);
        return out.text();
    }
    static String joinBounded(int limit,ModelCodec.Context c,JsonValue at,String...values){Buffer b=new Buffer(limit,c,at);for(String value:values)b.text(value);return b.text();}
    static void uriLiteral(Buffer output,String value){
        output.context.string(value);if(JsonRuntime.utf8Length(value)>output.limit-output.out.size())throw failure("resource-limit",output.source);
        byte[] bytes=value.getBytes(StandardCharsets.UTF_8);
        for(int i=0;i<bytes.length;i++){int b=bytes[i]&255;if(b<128)output.bytes(bytes,i,1);else output.text("%"+"0123456789ABCDEF".charAt(b>>4)+"0123456789ABCDEF".charAt(b&15));}
    }
    private static void named(Buffer out,String name,String value,boolean matrix){out.text(name);if(!matrix||!value.isEmpty()){out.text("=");out.text(value);}}
    private static void appendJoin(Buffer out,List<String> values,String delimiter){boolean first=true;for(String v:values){if(!first)out.text(delimiter);first=false;out.text(v);}}
    private static void appendProperties(Buffer out,List<Map.Entry<String,String>> values,boolean explode,String delimiter){boolean first=true;for(var v:values){if(!first)out.text(delimiter);first=false;out.text(v.getKey());out.text(explode?"=":delimiter);out.text(v.getValue());}}

    record Media(String type,String subtype,Map<String,String> parameters) {}
    static List<String> splitQuoted(String value,char delimiter) {
        var result=new ArrayList<String>();boolean quoted=false,escape=false;int start=0;
        for(int i=0;i<value.length();i++){char ch=value.charAt(i);if(escape)escape=false;else if(quoted&&ch=='\\')escape=true;else if(ch=='"')quoted=!quoted;else if(ch==delimiter&&!quoted){result.add(value.substring(start,i));start=i+1;}}
        if(quoted||escape)throw new IllegalArgumentException("unterminated quoted value");result.add(value.substring(start));return result;
    }
    static Media media(String value) {
        if(value.chars().anyMatch(c->c<32&&c!='\t'||c==127))throw new IllegalArgumentException("invalid media control");
        List<String> pieces=splitQuoted(value,';');String[] essence=pieces.getFirst().trim().split("/",-1);
        if(essence.length!=2||!token(essence[0])||!token(essence[1])||essence[0].contains("*")||essence[1].contains("*"))throw new IllegalArgumentException("invalid concrete media type");
        Map<String,String> parameters=new LinkedHashMap<>();
        for(String piece:pieces.subList(1,pieces.size())){int equal=piece.trim().indexOf('=');String part=piece.trim();if(equal<1)throw new IllegalArgumentException("invalid media parameter");String key=part.substring(0,equal).toLowerCase(Locale.ROOT),raw=part.substring(equal+1);if(!token(key))throw new IllegalArgumentException("invalid media parameter name");String decoded=quotedValue(raw);if(parameters.putIfAbsent(key,decoded)!=null)throw new IllegalArgumentException("duplicate media parameter");}
        return new Media(essence[0].toLowerCase(Locale.ROOT),essence[1].toLowerCase(Locale.ROOT),Collections.unmodifiableMap(parameters));
    }
    static String quotedValue(String value){
        if(!value.startsWith("\"")){if(!token(value))throw new IllegalArgumentException("invalid token value");return value;}
        if(value.length()<2||!value.endsWith("\""))throw new IllegalArgumentException("invalid quoted value");StringBuilder out=new StringBuilder();boolean escape=false;
        for(int i=1;i<value.length()-1;i++){char ch=value.charAt(i);if(escape){out.append(ch);escape=false;}else if(ch=='\\')escape=true;else if(ch=='"')throw new IllegalArgumentException("unescaped quote");else out.append(ch);}
        if(escape)throw new IllegalArgumentException("unfinished escape");return out.toString();
    }
    static int mediaRank(JsonValue declared,Media actual){
        JsonValue range=get(declared,"range");String kind=text(range,"kind");int rank;
        if(kind.equals("any"))rank=0;else if(!text(range,"type_name").equals(actual.type()))return -1;else if(kind.equals("type"))rank=1;else if(text(range,"subtype").equals(actual.subtype()))rank=2;else return -1;
        for(var entry:object(get(declared,"parameters")).values().entrySet()){String expected=text(entry.getValue()),found=actual.parameters().get(entry.getKey());if(found==null||!(entry.getKey().equals("charset")?expected.equalsIgnoreCase(found):expected.equals(found)))return -1;}
        return rank*10000+object(get(declared,"parameters")).values().size();
    }
    static int chooseMedia(List<JsonValue> values,String contentType){
        Media actual=media(contentType);int best=-1,rank=-1;
        for(int i=0;i<values.size();i++){int score=mediaRank(get(values.get(i),"media_type"),actual);if(score>rank){rank=score;best=i;}}
        if(best<0)throw new IllegalArgumentException("undeclared media type");
        String kind=text(get(values.get(best),"representation"),"kind");
        if(Set.of("text","stream","form").contains(kind)&&actual.parameters().containsKey("charset")&&!actual.parameters().get("charset").equalsIgnoreCase("utf-8"))throw new IllegalArgumentException("unsupported charset");
        return best;
    }
    static boolean isJson(JsonValue media){JsonValue range=get(media,"range");return text(range,"kind").equals("concrete")&&((text(range,"type_name").equals("application")&&text(range,"subtype").equals("json"))||text(range,"subtype").endsWith("+json"));}
    static boolean forbidden(String method,int status){return method.equals("HEAD")||status<200||status==204||status==205||status==304;}
    static int chooseResponse(JsonValue operation,int status){
        if(status<100||status>599)throw failure("invalid-response-status",operation);int best=-1,rank=-1;List<JsonValue> responses=array(get(operation,"responses"));
        for(int i=0;i<responses.size();i++){JsonValue match=get(responses.get(i),"status");String kind=text(match,"kind");int score=kind.equals("exact")&&number(get(match,"value"))==status?3:kind.equals("range")&&number(get(match,"value"))==status/100?2:kind.equals("default")?1:-1;if(score>rank){rank=score;best=i;}}
        return best;
    }

    static Map<String,JsonValue> headers(JsonValue specifications,Map<String,List<String>> raw,ModelCodec.Context c){
        var result=new LinkedHashMap<String,JsonValue>();var insensitive=new TreeMap<String,List<String>>(String.CASE_INSENSITIVE_ORDER);insensitive.putAll(raw);
        for(JsonValue header:array(specifications)){
            String name=text(header,"name");List<String> values=insensitive.get(name);
            if(values==null){if(flag(header,"required"))throw failure("missing-response-header",header);continue;}
            c.spend(1);String value=String.join(", ",values);c.string(value);headerValue(value);
            JsonValue strategy=get(header,"serialization");
            JsonValue decoded=text(strategy,"kind").equals("content")?decodeContent(header,value,c):decodeStyle(strategy,List.of(value),false,false,c,header);
            result.put(name,decoded);
        }
        return Collections.unmodifiableMap(result);
    }
    static Map<String,String> encodeHeaders(JsonValue specifications,Map<String,JsonValue> values,ModelCodec.Context c,int limit){
        var result=new TreeMap<String,String>(String.CASE_INSENSITIVE_ORDER);
        for(JsonValue header:array(specifications)){
            String name=text(header,"name");JsonValue value=values.get(name);
            if(value==null){if(flag(header,"required"))throw failure("missing-request-header",header);continue;}
            String encoded=serialize(header,name,"header",get(header,"serialization"),value,limit,c);headerValue(encoded);result.put(name,encoded);
        }
        if(result.size()!=values.size())throw new IllegalArgumentException("undeclared typed header");
        return Collections.unmodifiableMap(result);
    }
    private static JsonValue decodeContent(JsonValue header,String value,ModelCodec.Context c){
        JsonValue media=get(get(header,"serialization"),"media_type");if(isJson(media))return JsonRuntime.parse(value,c.jsonBudget);
        JsonValue representation=get(get(header,"content_media"),"representation");String kind=optionalText(representation,"scalar");return scalarValue(c.string(value),kind==null?"string":kind);
    }
    private static String scalarKind(JsonValue shape,String key){
        JsonValue declared=get(get(shape,"properties"),key);if(declared instanceof JsonString s)return s.value();
        JsonValue extra=get(shape,"additional");if(text(extra,"kind").equals("typed"))return text(extra,"scalar");
        if(text(extra,"kind").equals("any-scalar"))throw new IllegalArgumentException("untyped scalar header/form extras need a decoding policy");
        throw new IllegalArgumentException("undeclared scalar property");
    }
    static JsonValue decodeStyle(JsonValue strategy,List<String> raw,boolean uri,boolean repeated,ModelCodec.Context c,JsonValue at){
        return decodeStyle(strategy,raw,uri,repeated,true,c,at);
    }
    private static JsonValue decodeStyle(JsonValue strategy,List<String> raw,boolean uri,boolean repeated,boolean trim,ModelCodec.Context c,JsonValue at){
        JsonValue shape=get(strategy,"shape");String kind=text(shape,"kind"),style=text(strategy,"style");
        java.util.function.Function<String,String> decode=v->{String s=uri?unpercent(v,true):trim?v.trim():v;return c.string(s);};
        if(kind.equals("scalar")){if(raw.size()!=1)throw failure("ambiguous-scalar-field",at);return scalarValue(decode.apply(raw.getFirst()),text(shape,"scalar"));}
        String delimiter=style.equals("spaceDelimited")?"%20":style.equals("pipeDelimited")?"%7C":",";
        var pieces=new ArrayList<String>();
        if(repeated)pieces.addAll(raw);else{if(raw.size()!=1)throw failure("ambiguous-composite-field",at);pieces.addAll(Arrays.asList(raw.getFirst().split(java.util.regex.Pattern.quote(delimiter),-1)));}
        if(kind.equals("array"))return new JsonArray(pieces.stream().map(v->scalarValue(decode.apply(v),text(shape,"items"))).toList());
        var object=new LinkedHashMap<String,JsonValue>();boolean explode=flag(strategy,"explode");
        if(explode){for(String piece:pieces){int equal=piece.indexOf('=');if(equal<0)throw failure("invalid-object-field",at);String key=decode.apply(piece.substring(0,equal));JsonValue value=scalarValue(decode.apply(piece.substring(equal+1)),scalarKind(shape,key));if(object.putIfAbsent(key,value)!=null)throw failure("duplicate-object-field",at);}}
        else{if(pieces.size()%2!=0)throw failure("invalid-object-field",at);for(int i=0;i<pieces.size();i+=2){String key=decode.apply(pieces.get(i));JsonValue value=scalarValue(decode.apply(pieces.get(i+1)),scalarKind(shape,key));if(object.putIfAbsent(key,value)!=null)throw failure("duplicate-object-field",at);}}
        return new JsonObject(object);
    }
    private static JsonValue decodePartStyle(JsonValue strategy,String name,String text,ModelCodec.Context c,JsonValue at){
        String style=text(strategy,"style"),kind=text(get(strategy,"shape"),"kind");boolean explode=flag(strategy,"explode"),uri=!text(strategy,"percent_encoding").equals("none");
        String encodedName=percent(name==null?"":name,"uri-component","query",null,false,MAX_BYTES,c,at);
        java.util.function.Function<String,String> decode=value->c.string(uri?unpercent(value,true):value);
        if(style.equals("deepObject")||style.equals("form")&&explode&&kind.equals("flat-object")){
            var result=new LinkedHashMap<String,JsonValue>();JsonValue shape=get(strategy,"shape");
            for(String pair:text.split("&",-1)){
                int equal=pair.indexOf('=');if(equal<0)throw failure("invalid-part-field",at);
                String key=pair.substring(0,equal);
                if(style.equals("deepObject")){
                    String prefix=encodedName+"%5B";
                    if(!key.startsWith(prefix)||!key.endsWith("%5D"))throw failure("invalid-part-field",at);
                    key=key.substring(prefix.length(),key.length()-3);
                }
                key=decode.apply(key);JsonValue value=scalarValue(decode.apply(pair.substring(equal+1)),scalarKind(shape,key));
                if(result.putIfAbsent(key,value)!=null)throw failure("duplicate-object-field",at);
            }
            return new JsonObject(result);
        }
        if(style.equals("form")&&explode&&kind.equals("array")){
            var values=new ArrayList<String>();String prefix=encodedName+"=";
            for(String pair:text.split("&",-1)){if(!pair.startsWith(prefix))throw failure("invalid-part-field",at);values.add(pair.substring(prefix.length()));}
            return decodeStyle(strategy,values,uri,true,false,c,at);
        }
        String prefix=encodedName+"=";
        if(!text.startsWith(prefix))throw failure("invalid-part-field",at);
        return decodeStyle(strategy,List.of(text.substring(prefix.length())),uri,false,false,c,at);
    }
    static JsonValue partSpec(JsonValue aggregate,String name,int position){
        boolean positional=get(aggregate,"kind") instanceof JsonString s&&s.value().equals("positional");
        if(positional){List<JsonValue> prefix=array(get(aggregate,"prefix"));if(position<prefix.size())return prefix.get(position);JsonValue tail=get(aggregate,"items");if(text(tail,"kind").equals("allowed"))return get(tail,"part");}
        else {for(JsonValue part:array(get(aggregate,"parts")!=JsonNull.INSTANCE?get(aggregate,"parts"):get(aggregate,"fields")))if(Objects.equals(optionalText(part,"name"),name))return part;JsonValue extra=get(aggregate,"additional");if(text(extra,"kind").equals("allowed"))return get(extra,"part");}
        throw failure("undeclared-part",aggregate);
    }
    static WireValue.Parts jsonParts(JsonValue form,JsonObject value){
        var result=new LinkedHashMap<String,List<WireValue.Part>>();
        value.values().forEach((name,item)->{
            JsonValue part=partSpec(form,name,0);var parts=new ArrayList<WireValue.Part>();
            if(text(part,"multiplicity").equals("repeated-array-items"))for(JsonValue v:((JsonArray)item).values())parts.add(new WireValue.Part(new WireValue.Scalar(v),null,null,Map.of()));
            else parts.add(new WireValue.Part(new WireValue.Scalar(item),null,null,Map.of()));result.put(name,parts);
        });return new WireValue.Parts(result,List.of());
    }
    static void validateParts(JsonValue aggregate,WireValue.Parts value,ModelCodec.Context c){
        boolean positional=get(aggregate,"kind") instanceof JsonString s&&s.value().equals("positional");
        if(positional){
            if(!value.named().isEmpty())throw failure("invalid-positional-parts",aggregate);
            count(value.positional().size(),aggregate,"min_items","max_items",aggregate);
            for(int i=0;i<value.positional().size();i++){c.spend(1);validatePart(partSpec(aggregate,null,i),value.positional().get(i),c);}
        }else{
            if(!value.positional().isEmpty())throw failure("invalid-named-parts",aggregate);JsonValue rules=get(aggregate,"rules");
            count(value.named().size(),rules,"min_properties","max_properties",aggregate);
            for(JsonValue required:array(get(rules,"required")))if(!value.named().containsKey(text(required,"value")))throw failure("missing-required-part",aggregate);
            for(var entry:value.named().entrySet()){
                c.spend(1);c.string(entry.getKey());JsonValue part=partSpec(aggregate,entry.getKey(),0);List<WireValue.Part> values=entry.getValue();
                if(text(part,"multiplicity").equals("one")){if(values.size()!=1)throw failure("invalid-part-multiplicity",part);}else{
                    count(values.size(),part,"min_items","max_items",part);
                    if(values.isEmpty()&&(flag(part,"required")||array(get(rules,"required")).stream().anyMatch(required->text(required,"value").equals(entry.getKey()))))throw failure("empty-repeated-part",part);
                }
                for(WireValue.Part item:values)validatePart(part,item,c);
            }
        }
    }
    private static void count(int value,JsonValue spec,String min,String max,JsonValue at){if(value<located(spec,min,0)||value>located(spec,max,Long.MAX_VALUE))throw failure("wire-cardinality",at);}
    static String partContentType(JsonValue part,WireValue.Part value){
        List<JsonValue> choices=array(get(part,"content_types"));String supplied=value.contentType();
        if(choices.isEmpty()){if(supplied!=null)media(supplied);return supplied;}
        if(supplied==null){if(choices.size()!=1||!text(get(choices.getFirst(),"range"),"kind").equals("concrete"))throw failure("part-content-type-required",part);supplied=text(choices.getFirst(),"declared");}
        Media actual=media(supplied);boolean matches=false;for(JsonValue choice:choices)if(mediaRank(choice,actual)>=0)matches=true;
        if(!matches)throw failure("part-content-type-mismatch",part);return supplied;
    }
    static void validatePart(JsonValue spec,WireValue.Part value,ModelCodec.Context c){
        c.spend(1);JsonValue representation=get(spec,"representation");String kind=text(representation,"kind");
        partContentType(spec,value);
        if(value.filename()!=null){c.string(value.filename());mimeQuoted(value.filename());}
        if(kind.equals("binary")){Bytes bytes=((WireValue.Binary)value.value()).value();if(bytes.size()>number(get(get(representation,"bytes"),"max_bytes")))throw failure("resource-limit",spec);c.spend(bytes.size());}
        else if(!(value.value() instanceof WireValue.Scalar))throw failure("invalid-part-representation",spec);
        for(JsonValue header:array(get(spec,"headers")))if(flag(header,"required")&&!value.headers().containsKey(text(header,"name")))throw failure("missing-part-header",header);
    }
    static String form(JsonValue spec,WireValue.Parts value,int limit,ModelCodec.Context c){
        validateParts(spec,value,c);Buffer out=new Buffer(limit,c,spec);var names=new ArrayList<>(value.named().keySet());names.sort(UNICODE);boolean first=true;
        for(String name:names){JsonValue part=partSpec(spec,name,0);for(WireValue.Part item:value.named().get(name)){
            if(!first)out.text("&");first=false;JsonValue representation=get(part,"representation");String kind=text(representation,"kind");JsonValue data=((WireValue.Scalar)item.value()).value();
            if(kind.equals("style"))out.text(serialize(part,name,"query",get(representation,"serialization"),data,limit,c));
            else {String text=kind.equals("json")?JsonRuntime.stringify(data,c.jsonBudget):scalar(data,text(representation,"scalar"));out.text(percent(name,"form-url-encoded","query",null,false,limit,c,part));out.text("=");out.text(percent(text,text(representation,"outer_encoding"),"query",null,false,limit,c,part));}
        }}return out.text();
    }
    private record Pair(String name,String rawName,String rawValue) {}
    static WireValue.Parts decodeForm(JsonValue spec,byte[] bytes,ModelCodec.Context c){
        String text=utf8(bytes);c.spend(bytes.length);var pairs=new ArrayList<Pair>();
        if(!text.isEmpty())for(String segment:text.split("&",-1)){int equals=segment.indexOf('=');if(equals<0)throw failure("invalid-form-pair",spec);String name=unpercent(segment.substring(0,equals),true);pairs.add(new Pair(name,segment.substring(0,equals),segment.substring(equals+1)));}
        var grouped=new LinkedHashMap<String,List<Pair>>();
        for(Pair pair:pairs){
            String owner=null;
            for(JsonValue part:array(get(spec,"fields"))){String name=optionalText(part,"name");JsonValue representation=get(part,"representation");
                boolean match=Objects.equals(name,pair.name());
                if(text(representation,"kind").equals("style")){
                    JsonValue serialization=get(representation,"serialization"),shape=get(serialization,"shape");String style=text(serialization,"style");
                    if(style.equals("deepObject"))match=pair.name().startsWith(name+"[")&&pair.name().endsWith("]");
                    else if(flag(serialization,"explode")&&text(shape,"kind").equals("flat-object"))match=get(get(shape,"properties"),pair.name())!=JsonNull.INSTANCE||!text(get(shape,"additional"),"kind").equals("forbidden");
                }
                if(match){if(owner!=null&&!owner.equals(name))throw failure("ambiguous-form-field",part);owner=name;}
            }
            if(owner==null){partSpec(spec,pair.name(),0);owner=pair.name();}grouped.computeIfAbsent(owner,ignored->new ArrayList<>()).add(pair);
        }
        var result=new LinkedHashMap<String,List<WireValue.Part>>();
        for(var entry:grouped.entrySet()){
            String name=entry.getKey();JsonValue part=partSpec(spec,name,0),representation=get(part,"representation");String kind=text(representation,"kind");var values=new ArrayList<WireValue.Part>();
            if(kind.equals("style")){
                JsonValue strategy=get(representation,"serialization"),shape=get(strategy,"shape");String style=text(strategy,"style");JsonValue value;
                if(text(part,"multiplicity").equals("repeated-array-items")){
                    for(Pair pair:entry.getValue()){JsonValue item=decodeStyle(strategy,List.of(pair.rawValue()),true,false,c,part);values.add(new WireValue.Part(new WireValue.Scalar(item),null,null,Map.of()));}
                    result.put(name,values);continue;
                }
                if(text(shape,"kind").equals("flat-object")&&(flag(strategy,"explode")||style.equals("deepObject"))){var object=new LinkedHashMap<String,JsonValue>();for(Pair pair:entry.getValue()){
                    String key=style.equals("deepObject")?pair.name().substring(name.length()+1,pair.name().length()-1):pair.name();JsonValue item=scalarValue(unpercent(pair.rawValue(),true),scalarKind(shape,key));if(object.putIfAbsent(key,item)!=null)throw failure("duplicate-form-field",part);
                }value=new JsonObject(object);}else value=decodeStyle(strategy,entry.getValue().stream().map(Pair::rawValue).toList(),true,flag(strategy,"explode"),c,part);
                values.add(new WireValue.Part(new WireValue.Scalar(value),null,null,Map.of()));
            }else{
                if(text(part,"multiplicity").equals("one")&&entry.getValue().size()!=1)throw failure("duplicate-form-field",part);
                for(Pair pair:entry.getValue()){String decoded=unpercent(pair.rawValue(),true);JsonValue value=kind.equals("json")?JsonRuntime.parse(decoded,c.jsonBudget):scalarValue(decoded,text(representation,"scalar"));values.add(new WireValue.Part(new WireValue.Scalar(value),null,null,Map.of()));}
            }
            result.put(name,values);
        }
        WireValue.Parts parts=new WireValue.Parts(result,List.of());validateParts(spec,parts,c);return parts;
    }
    static String mimeQuoted(String value){if(value.chars().anyMatch(ch->ch<32||ch==127))throw new IllegalArgumentException("invalid MIME quoted value");return "\""+value.replace("\\","\\\\").replace("\"","\\\"")+"\"";}

    record Encoded(Bytes bytes,String contentType) {}
    static Encoded encodeBody(JsonValue media,WireValue value,String actual,int limit,ModelCodec.Context c){
        JsonValue representation=get(media,"representation");String kind=text(representation,"kind");
        if(kind.equals("multipart"))return multipart(get(representation,"multipart"),(WireValue.Parts)value,actual,limit,c);
        Bytes bytes=switch(kind){
            case "json"->Bytes.owned(JsonRuntime.bytes(((WireValue.Scalar)value).value(),c.jsonBudget));
            case "text"->Bytes.owned(scalar(((WireValue.Scalar)value).value(),text(representation,"scalar")).getBytes(StandardCharsets.UTF_8));
            case "binary"->{Bytes b=((WireValue.Binary)value).value();if(b.size()>number(get(get(representation,"bytes"),"max_bytes")))throw failure("resource-limit",media);yield b;}
            case "form"->Bytes.owned(form(get(representation,"form"),(WireValue.Parts)value,limit,c).getBytes(StandardCharsets.UTF_8));
            case "stream"->encodeItems(get(representation,"stream"),(WireValue.Items)value,limit,c);
            default->throw failure("unsupported-wire-instruction",media);
        };
        if(bytes.size()>limit)throw failure("resource-limit",media);c.spend(bytes.size());return new Encoded(bytes,actual);
    }
    static WireValue decodeBody(JsonValue media,byte[] bytes,String actual,ModelCodec.Context c){
        JsonValue representation=get(media,"representation");return switch(text(representation,"kind")){
            case "json"->new WireValue.Scalar(JsonRuntime.parse(bytes,c.jsonBudget));
            case "text"->{String value=utf8(bytes);c.spend(bytes.length);yield new WireValue.Scalar(scalarValue(value,text(representation,"scalar")));}
            case "binary"->{if(bytes.length>number(get(get(representation,"bytes"),"max_bytes")))throw failure("resource-limit",media);c.spend(bytes.length);yield new WireValue.Binary(Bytes.owned(bytes));}
            case "form"->decodeForm(get(representation,"form"),bytes,c);
            case "multipart"->decodeMultipart(get(representation,"multipart"),bytes,actual,c);
            default->throw failure("unsupported-wire-instruction",media);
        };
    }
    private static Bytes encodeItems(JsonValue stream,WireValue.Items items,int limit,ModelCodec.Context c){
        Buffer out=new Buffer(limit,c,stream);int itemLimit=(int)Math.min(limit,number(get(stream,"max_item_bytes")));boolean sse=text(stream,"framing").equals("server-sent-events");String lastId=null;
        for(JsonValue value:items.values()){
            Buffer item=new Buffer(itemLimit,c,stream);
            if(!sse){item.bytes(JsonRuntime.bytes(value,c.jsonBudget));item.text("\n");}
            else {
                Map<String,JsonValue> event=object(value).values();
                if(!event.containsKey("data")||event.keySet().stream().anyMatch(key->!Set.of("data","id","event","retry").contains(key)))throw failure("invalid-sse-item",stream);
                if(lastId!=null&&!event.containsKey("id"))throw failure("sse-id-state-must-be-explicit",stream);
                for(String key:List.of("id","event","retry","data"))if(event.containsKey(key)){
                    JsonValue field=event.get(key);String text;
                    if(key.equals("retry")){JsonNumber n=(JsonNumber)field;if(!n.isInteger()||n.signum()<0)throw failure("invalid-sse-retry",stream);text=n.exactIntegerValue(MAX_NUMBER_BYTES).toString();}
                    else text=((JsonString)field).value();
                    if(!key.equals("data")){
                        if(text.contains("\n")||text.contains("\r")||key.equals("id")&&text.indexOf(0)>=0||key.equals("event")&&text.isEmpty())throw failure("unrepresentable-sse-field",stream);
                        item.text(key+": "+text+"\n");if(key.equals("id"))lastId=text;
                    }else{
                        if(text.contains("\r"))throw failure("unrepresentable-sse-data",stream);
                        for(String line:text.split("\n",-1))item.text("data: "+line+"\n");
                    }
                }
                item.text("\n");
            }
            out.bytes(item.value().internal());
        }
        return out.value();
    }
    private static final SecureRandom RANDOM=new SecureRandom();
    private static String boundary(String value){
        if(value==null){byte[] bytes=new byte[18];RANDOM.nextBytes(bytes);return "suspect_"+java.util.HexFormat.of().formatHex(bytes);}
        if(value.isEmpty()||value.length()>70||value.endsWith(" ")||value.chars().anyMatch(ch->ch>127||!(Character.isLetterOrDigit(ch)||"'()+_,-./:=? ".indexOf(ch)>=0)))throw new IllegalArgumentException("invalid MIME boundary");
        return value;
    }
    private record EncodedPart(Map<String,String> headers,Bytes body) {}
    private static Encoded multipart(JsonValue spec,WireValue.Parts parts,String actual,int limit,ModelCodec.Context c){
        validateParts(spec,parts,c);boolean positional=get(spec,"kind") instanceof JsonString s&&s.value().equals("positional");var encoded=new ArrayList<EncodedPart>();int remaining=limit;
        if(positional){for(int i=0;i<parts.positional().size();i++){EncodedPart part=encodePart(partSpec(spec,null,i),null,parts.positional().get(i),remaining,c);remaining=retainPart(part,remaining,spec);encoded.add(part);}}
        else {var names=new ArrayList<>(parts.named().keySet());names.sort(UNICODE);for(String name:names)for(WireValue.Part value:parts.named().get(name)){EncodedPart part=encodePart(partSpec(spec,name,0),name,value,remaining,c);remaining=retainPart(part,remaining,spec);encoded.add(part);}}
        Media type=media(actual);String fixed=type.parameters().get("boundary"),delimiter=null;
        for(int attempt=0;attempt<8;attempt++){
            String candidate=boundary(fixed);byte[] marker=("\r\n--"+candidate).getBytes(StandardCharsets.US_ASCII);
            boolean collision=encoded.stream().anyMatch(part->indexOf(part.body().internal(),marker,0)>=0);
            if(!collision){delimiter=candidate;break;}if(fixed!=null)throw failure("multipart-boundary-collision",spec);
        }
        if(delimiter==null)throw failure("resource-limit",spec);Buffer output=new Buffer(limit,c,spec);
        for(EncodedPart part:encoded){output.text("--"+delimiter+"\r\n");for(var header:part.headers().entrySet())output.text(header.getKey()+": "+header.getValue()+"\r\n");output.text("\r\n");output.bytes(part.body().internal());output.text("\r\n");}
        output.text("--"+delimiter+"--\r\n");return new Encoded(output.value(),fixed==null?actual+"; boundary="+delimiter:actual);
    }
    private static int retainPart(EncodedPart part,int remaining,JsonValue spec){
        long size=part.body().size()+4L;
        for(var header:part.headers().entrySet())size+=header.getKey().length()+JsonRuntime.utf8Length(header.getValue())+4L;
        if(size>remaining)throw failure("resource-limit",spec);return remaining-(int)size;
    }
    private static EncodedPart encodePart(JsonValue spec,String name,WireValue.Part part,int limit,ModelCodec.Context c){
        JsonValue representation=get(spec,"representation");String kind=text(representation,"kind"),contentType=partContentType(spec,part);Bytes value;
        if(kind.equals("binary"))value=((WireValue.Binary)part.value()).value();
        else {JsonValue json=((WireValue.Scalar)part.value()).value();String text=switch(kind){case "json"->JsonRuntime.stringify(json,c.jsonBudget);case "text"->scalar(json,text(representation,"scalar"));case "style"->serialize(spec,name==null?"":name,"query",get(representation,"serialization"),json,limit,c);default->throw failure("unsupported-part-instruction",spec);};
            if(kind.equals("style")&&!decodePartStyle(get(representation,"serialization"),name,text,c,spec).equals(json))throw failure("ambiguous-part-style",spec);
            if(JsonRuntime.utf8Length(text)>limit)throw failure("resource-limit",spec);value=Bytes.owned(text.getBytes(StandardCharsets.UTF_8));}
        if(value.size()>limit)throw failure("resource-limit",spec);
        var headers=new TreeMap<String,String>(String.CASE_INSENSITIVE_ORDER);headers.putAll(encodeHeaders(get(spec,"headers"),part.headers(),c,Math.min(limit,65536)));
        if(name!=null){if(headers.containsKey("Content-Disposition"))throw failure("multipart-disposition-conflict",spec);headers.put("Content-Disposition","form-data; name="+mimeQuoted(name)+(part.filename()==null?"":"; filename="+mimeQuoted(part.filename())));}
        else if(part.filename()!=null){if(headers.containsKey("Content-Disposition"))throw failure("multipart-disposition-conflict",spec);headers.put("Content-Disposition","attachment; filename="+mimeQuoted(part.filename()));}
        if(contentType!=null){String previous=headers.putIfAbsent("Content-Type",contentType);if(previous!=null&&!previous.equals(contentType))throw failure("multipart-content-type-conflict",spec);}
        for(var header:headers.entrySet()){if(!token(header.getKey()))throw new IllegalArgumentException("invalid MIME header name");headerValue(header.getValue());}
        checkPartEncoding(headers.get("Content-Transfer-Encoding"),headers.get("Content-Encoding"),value.internal(),0,value.size(),spec);
        return new EncodedPart(headers,value);
    }
    // KMP bounds delimiter scanning to input bytes plus boundary length.
    private static int indexOf(byte[] bytes,byte[] pattern,int from){
        int[] prefix=new int[pattern.length];for(int i=1,j=0;i<pattern.length;i++){while(j>0&&pattern[i]!=pattern[j])j=prefix[j-1];if(pattern[i]==pattern[j])j++;prefix[i]=j;}
        for(int i=from,j=0;i<bytes.length;i++){while(j>0&&bytes[i]!=pattern[j])j=prefix[j-1];if(bytes[i]==pattern[j])j++;if(j==pattern.length)return i-pattern.length+1;}return -1;
    }
    private static boolean bytesAt(byte[] bytes,int at,String value){byte[] expected=value.getBytes(StandardCharsets.US_ASCII);if(at<0||at+expected.length>bytes.length)return false;for(int i=0;i<expected.length;i++)if(bytes[at+i]!=expected[i])return false;return true;}
    private static int boundaryEnd(byte[] bytes,int at){int i=at;if(bytesAt(bytes,i,"--"))i+=2;while(i<bytes.length&&(bytes[i]==' '||bytes[i]=='\t'))i++;return i==bytes.length||bytesAt(bytes,i,"\r\n")?i:-1;}
    static WireValue.Parts decodeMultipart(JsonValue spec,byte[] bytes,String actual,ModelCodec.Context c){
        String boundary=boundary(media(actual).parameters().get("boundary"));if(!media(actual).parameters().containsKey("boundary"))throw failure("multipart-boundary-required",spec);
        c.spend(bytes.length);byte[] first=("--"+boundary).getBytes(StandardCharsets.US_ASCII),marker=("\r\n--"+boundary).getBytes(StandardCharsets.US_ASCII);int cursor=indexOf(bytes,first,0);
        while(cursor>=0&&(cursor!=0&&!bytesAt(bytes,cursor-2,"\r\n")||boundaryEnd(bytes,cursor+first.length)<0))cursor=indexOf(bytes,first,cursor+first.length);
        if(cursor<0)throw failure("invalid-multipart",spec);
        boolean positional=get(spec,"kind") instanceof JsonString s&&s.value().equals("positional");var named=new LinkedHashMap<String,List<WireValue.Part>>();var positions=new ArrayList<WireValue.Part>();int position=0;
        while(true){
            int after=cursor+first.length,end=boundaryEnd(bytes,after);if(end<0)throw failure("invalid-multipart-boundary",spec);if(bytesAt(bytes,after,"--"))break;if(!bytesAt(bytes,end,"\r\n"))throw failure("invalid-multipart",spec);
            int start=end+2,headEnd=indexOf(bytes,"\r\n\r\n".getBytes(StandardCharsets.US_ASCII),start);
            if(bytesAt(bytes,start,"\r\n"))headEnd=start-2;
            if(headEnd<start-2||headEnd-start>65536)throw failure("multipart-header-limit",spec);
            var rawHeaders=new TreeMap<String,List<String>>(String.CASE_INSENSITIVE_ORDER);
            if(headEnd>=start)for(String line:utf8(Arrays.copyOfRange(bytes,start,headEnd)).split("\r\n",-1)){int colon=line.indexOf(':');if(colon<1||!token(line.substring(0,colon)))throw failure("invalid-part-header",spec);String value=line.substring(colon+1).trim();headerValue(value);rawHeaders.computeIfAbsent(line.substring(0,colon),ignored->new ArrayList<>()).add(value);}
            int bodyStart=headEnd+4,next=indexOf(bytes,marker,bodyStart);
            while(next>=0&&boundaryEnd(bytes,next+marker.length)<0)next=indexOf(bytes,marker,next+marker.length);
            if(next<0)throw failure("unterminated-multipart",spec);
            String contentType=single(rawHeaders,"Content-Type","text/plain"),name=null,filename=null;String disposition=single(rawHeaders,"Content-Disposition",null);
            if(disposition!=null){List<String> pieces=splitQuoted(disposition,';');Map<String,String> params=new HashMap<>();for(String piece:pieces.subList(1,pieces.size())){String part=piece.trim();int equal=part.indexOf('=');if(equal<1)throw failure("invalid-part-disposition",spec);String key=part.substring(0,equal).toLowerCase(Locale.ROOT);if(!token(key)||params.putIfAbsent(key,quotedValue(part.substring(equal+1)))!=null)throw failure("invalid-part-disposition",spec);}name=params.get("name");filename=params.get("filename");if(!positional&&!pieces.getFirst().trim().equalsIgnoreCase("form-data"))throw failure("invalid-part-disposition",spec);}
            if(!positional&&name==null)throw failure("missing-part-name",spec);JsonValue part=partSpec(spec,name,position++);
            if(positional){if(position>located(spec,"max_items",Long.MAX_VALUE))throw failure("wire-cardinality",spec);}
            else {
                List<WireValue.Part> previous=named.get(name);int count=previous==null?0:previous.size();
                if(text(part,"multiplicity").equals("one")&&count!=0)throw failure("invalid-part-multiplicity",part);
                if(text(part,"multiplicity").equals("repeated-array-items")&&count>=located(part,"max_items",Long.MAX_VALUE))throw failure("wire-cardinality",part);
                if(previous==null&&named.size()>=located(get(spec,"rules"),"max_properties",Long.MAX_VALUE))throw failure("wire-cardinality",spec);
            }
            checkPartEncoding(single(rawHeaders,"Content-Transfer-Encoding",null),single(rawHeaders,"Content-Encoding",null),bytes,bodyStart,next-bodyStart,part);
            JsonValue representation=get(part,"representation");String kind=text(representation,"kind");if(kind.equals("binary")&&next-bodyStart>number(get(get(representation,"bytes"),"max_bytes")))throw failure("resource-limit",part);byte[] data=Arrays.copyOfRange(bytes,bodyStart,next);WireValue value;
            if(kind.equals("binary")){if(data.length>number(get(get(representation,"bytes"),"max_bytes")))throw failure("resource-limit",part);value=new WireValue.Binary(Bytes.owned(data));}
            else if(kind.equals("json"))value=new WireValue.Scalar(JsonRuntime.parse(data,c.jsonBudget));
            else if(kind.equals("text"))value=new WireValue.Scalar(scalarValue(utf8(data),text(representation,"scalar")));
            else value=new WireValue.Scalar(decodePartStyle(get(representation,"serialization"),name,utf8(data),c,part));
            WireValue.Part decoded=new WireValue.Part(value,contentType,filename,headers(get(part,"headers"),rawHeaders,c));validatePart(part,decoded,c);
            if(positional)positions.add(decoded);else named.computeIfAbsent(name,ignored->new ArrayList<>()).add(decoded);
            cursor=next+2;
        }
        WireValue.Parts result=new WireValue.Parts(named,positions);validateParts(spec,result,c);return result;
    }
    private static void checkPartEncoding(String transfer,String encoding,byte[] bytes,int start,int length,JsonValue at){
        if(encoding!=null&&!encoding.equalsIgnoreCase("identity"))throw failure("unexpected-part-content-encoding",at);
        if(transfer==null)return;
        if(!Set.of("7bit","8bit","binary").contains(transfer.toLowerCase(Locale.ROOT)))throw failure("unexpected-part-transfer-encoding",at);
        if(transfer.equalsIgnoreCase("7bit"))for(int i=start;i<start+length;i++)if(bytes[i]<0)throw failure("unexpected-part-transfer-encoding",at);
    }
    static String single(Map<String,List<String>> headers,String name,String fallback){List<String> values=headers.get(name);if(values==null)return fallback;if(values.size()!=1)throw new IllegalArgumentException("ambiguous repeated header");return values.getFirst();}
}
