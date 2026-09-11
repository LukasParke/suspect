#include "@PACKAGE@/protocol.hpp"
#include "number.hpp"
#include <atomic>
#include <charconv>

namespace @NAMESPACE@::detail {
std::string request_bytes(const Bytes& bytes,Context& context,const Source& source,std::size_t limit){
    if(bytes.size()>limit)http_fail(SdkError::Kind::ResourceLimit,source,"binary input exceeds byte policy");context.spend(bytes.size(),source,"");
    return bytes.empty()?std::string{}:std::string(reinterpret_cast<const char*>(bytes.data()),bytes.size());
}
Bytes response_bytes(std::string_view bytes,Context& context,const Source& source,std::size_t limit){
    if(bytes.size()>limit)http_fail(SdkError::Kind::ResourceLimit,source,"binary response/part exceeds byte policy");context.spend(bytes.size(),source,"");
    Bytes result;result.reserve(bytes.size());for(unsigned char byte:bytes)result.push_back(byte);return result;
}
void aggregate_rules(const ObjectRules& rules,const std::set<std::string>& names){
    for(const auto& name:rules.required)if(!names.contains(name))http_fail(SdkError::Kind::RequestValidation,rules.source,"required form/multipart property is absent");
    if((rules.minimum&&names.size()<*rules.minimum)||(rules.maximum&&names.size()>*rules.maximum))http_fail(SdkError::Kind::RequestValidation,rules.source,"aggregate property count violates source bounds");
}
void part_count(const PartRules& rules,std::size_t count){
    if(!count||(rules.minimum&&count<*rules.minimum)||(rules.maximum&&count>*rules.maximum))http_fail(SdkError::Kind::RequestRepresentation,rules.source,"present repeated part has no faithful representation or violates cardinality");
}
std::string integer_digits(const JsonNumber& number,std::size_t limit,const Source& source){
    auto decimal=Decimal::parse(number.token());if(!decimal.integral()||decimal.sign<0)http_fail(SdkError::Kind::RequestRepresentation,source,"framing requires a nonnegative integer");
    if(!decimal.sign)return "0";
    auto length=decimal.exponent.plus(SignedDecimal::size(decimal.digits.size()));
    if(length.compare(SignedDecimal::size(limit))>0)http_fail(SdkError::Kind::ResourceLimit,source,"integer expansion exceeds framing byte ceiling");
    std::size_t padding=0;for(char c:decimal.exponent.digits)padding=padding*10+static_cast<unsigned>(c-'0');
    return decimal.digits+std::string(padding,'0');
}
std::string part_json_bytes(const PartRules& rules,const JsonValue& json,Context& context,std::size_t limit){
    const auto previous=context.limits.max_bytes;
    struct Restore {Context& context;std::size_t previous;~Restore(){context.limits.max_bytes=previous;}} restore{context,previous};
    context.limits.max_bytes=std::min(previous,limit);
    std::string bytes;
    switch(rules.encoding){
        case PartEncoding::Json:bytes=write_document(json,context,rules.source);break;
        case PartEncoding::Text:bytes=scalar_text(json,rules.scalar,rules.source);context.spend(bytes.size(),rules.source,"");break;
        case PartEncoding::Style:bytes=encode_parameter(rules.name,Location::Query,rules.serialization,json,context,rules.source,limit);break;
        case PartEncoding::Binary:http_fail(SdkError::Kind::RequestRepresentation,rules.source,"binary parts require Bytes, not JSON");
    }
    if(bytes.size()>limit)http_fail(SdkError::Kind::ResourceLimit,rules.source,"part exceeds byte ceiling");return bytes;
}
JsonValue part_json_value(const PartRules& rules,std::string_view bytes,Context& context){
    if(bytes.size()>rules.max_bytes)http_fail(SdkError::Kind::ResourceLimit,rules.source,"part exceeds its byte policy");
    if(rules.encoding==PartEncoding::Json)return parse_document(bytes,context,rules.source);
    if(rules.encoding==PartEncoding::Text)return parse_scalar(bytes,rules.scalar,context,rules.source);
    if(rules.encoding==PartEncoding::Style)return styled_part_value(rules,bytes,context);
    http_fail(SdkError::Kind::ResponseDecoding,rules.source,"binary part cannot enter a JSON codec");
}
std::string select_part_media(const PartRules& rules,const Presence<std::string>& supplied){
    std::string actual;
    if(supplied)actual=*supplied;
    else if(rules.content_types.size()==1&&rules.content_types[0].type!="*"&&rules.content_types[0].subtype!="*")actual=rules.content_types[0].declared;
    else http_fail(SdkError::Kind::RequestRepresentation,rules.source,"part requires an explicit concrete content type");
    (void)select_media(rules.content_types,actual,rules.source);return actual;
}
static std::string quote(std::string_view text,const Source& source){
    try{(void)unicode_length(text);}catch(Failure&){http_fail(SdkError::Kind::RequestRepresentation,source,"MIME name/filename is not UTF-8");}
    std::string out="\"";for(unsigned char c:text){if(c<32||c==127)http_fail(SdkError::Kind::RequestRepresentation,source,"MIME name/filename contains a control character");if(c=='"'||c=='\\')out.push_back('\\');out.push_back(static_cast<char>(c));}out.push_back('"');return out;
}
std::string disposition(std::string_view name,const Presence<std::string>& filename,const Source& source){
    auto value="form-data; name="+quote(name,source);if(filename)value+="; filename="+quote(*filename,source);return value;
}
static Presence<Media> disposition_fields(const RawPart& part,const Source& source){
    auto header=find_header(part.headers,"content-disposition",source,false);if(!header)return std::nullopt;
    return parse_media("x/"+*header);
}
Presence<std::string> part_filename(const RawPart& part,const Source& source){
    auto fields=disposition_fields(part,source);if(!fields)http_fail(SdkError::Kind::ResponseDecoding,source,"invalid part Content-Disposition");
    auto value=fields->parameters.find("filename");return value==fields->parameters.end()?Presence<std::string>{}:value->second;
}
static std::string boundary_value(const Media& media,const Source& source){
    auto it=media.parameters.find("boundary");if(it==media.parameters.end()||it->second.empty()||it->second.size()>70)http_fail(SdkError::Kind::ResponseDecoding,source,"multipart boundary is missing or invalid");
    for(unsigned char c:it->second)if(!((c>='0'&&c<='9')||(c>='a'&&c<='z')||(c>='A'&&c<='Z'))&&std::string_view("'()+_,-./:=? ").find(static_cast<char>(c))==std::string_view::npos)http_fail(SdkError::Kind::ResponseDecoding,source,"invalid MIME boundary character");
    if(it->second.ends_with(' '))http_fail(SdkError::Kind::ResponseDecoding,source,"MIME boundary ends in space");return it->second;
}
EncodedBody encode_multipart(std::vector<RawPart> parts,std::string content_type,Context& context,const Settings& settings,const Source& source){
    if(parts.size()>settings.max_parts)http_fail(SdkError::Kind::ResourceLimit,source,"multipart part count ceiling exceeded");
    auto media=parse_media(content_type);if(!media||media->type!="multipart")http_fail(SdkError::Kind::RequestRepresentation,source,"invalid multipart content type");
    std::string boundary;bool fixed=media->parameters.contains("boundary");
    if(fixed)boundary=boundary_value(*media,source);
    else{static std::atomic<std::uint64_t> sequence{0};boundary="suspect_cpp_"+std::to_string(sequence.fetch_add(1,std::memory_order_relaxed));}
    auto collision=[&]{return std::any_of(parts.begin(),parts.end(),[&](const auto& part){context.spend(part.bytes.size()+1,source,"");return part.bytes.starts_with("--"+boundary)||part.bytes.find("\r\n--"+boundary)!=std::string::npos;});};
    for(unsigned attempt=0;collision();++attempt){if(fixed||attempt==63||boundary.size()==70)http_fail(SdkError::Kind::RequestRepresentation,source,"multipart boundary occurs in part data");boundary+="x";}
    if(!fixed)content_type+="; boundary="+boundary;
    std::string bytes;std::size_t headers=0;
    for(const auto& part:parts){
        if(part.bytes.size()>settings.max_part_bytes)http_fail(SdkError::Kind::ResourceLimit,source,"multipart part exceeds byte ceiling");
        append_bounded(bytes,"--"+boundary+"\r\n",settings.max_request_bytes,source,context);
        std::set<std::string> names;
        for(const auto& [name,value]:part.headers){
            if(!header_name(name)||!header_value(value)||!names.insert(lower_ascii(name)).second)http_fail(SdkError::Kind::RequestRepresentation,source,"invalid or duplicate MIME header");
            if(name.size()+value.size()+4>settings.transfer.max_header_bytes-headers)http_fail(SdkError::Kind::ResourceLimit,source,"multipart header budget exceeded");headers+=name.size()+value.size()+4;
            append_bounded(bytes,name+": "+value+"\r\n",settings.max_request_bytes,source,context);
        }
        append_bounded(bytes,"\r\n",settings.max_request_bytes,source,context);append_bounded(bytes,part.bytes,settings.max_request_bytes,source,context);append_bounded(bytes,"\r\n",settings.max_request_bytes,source,context);
    }
    append_bounded(bytes,"--"+boundary+"--\r\n",settings.max_request_bytes,source,context);return {std::move(bytes),std::move(content_type)};
}
std::vector<RawPart> decode_multipart(std::string_view bytes,std::string_view content_type,Context& context,const Settings& settings,const Source& source){
    context.spend(bytes.size(),source,"");auto media=parse_media(content_type);if(!media||media->type!="multipart")http_fail(SdkError::Kind::ResponseDecoding,source,"invalid multipart Content-Type");
    auto boundary=boundary_value(*media,source);auto marker="--"+boundary;std::vector<RawPart> parts;std::size_t header_bytes=0,header_count=0;
    struct Delimiter {std::size_t at,after;bool closing;};
    auto find=[&](std::size_t from)->Presence<Delimiter>{for(;;){auto found=bytes.find(marker,from);if(found==std::string_view::npos)return std::nullopt;from=found+marker.size();
        if(found!=0&&(found<2||bytes.substr(found-2,2)!="\r\n"))continue;
        auto after=from;bool closing=bytes.substr(after,2)=="--";if(closing)after+=2;
        while(after<bytes.size()&&(bytes[after]==' '||bytes[after]=='\t'))++after;
        if(closing&&after==bytes.size())return Delimiter{found,after,true};
        if(bytes.substr(after,2)=="\r\n")return Delimiter{found,after+2,closing};
    }};
    auto delimiter=find(0);if(!delimiter)http_fail(SdkError::Kind::ResponseDecoding,source,"multipart boundary not found");
    for(;;){
        if(delimiter->closing)break;auto at=delimiter->after;
        if(parts.size()==settings.max_parts)http_fail(SdkError::Kind::ResourceLimit,source,"multipart part count exceeded");
        auto head_end=bytes.find("\r\n\r\n",at);if(head_end==std::string_view::npos)http_fail(SdkError::Kind::ResponseDecoding,source,"incomplete MIME headers");
        if(head_end+4-at>settings.transfer.max_header_bytes-header_bytes)http_fail(SdkError::Kind::ResourceLimit,source,"multipart header budget exceeded");header_bytes+=head_end+4-at;
        RawPart part;
        while(at<head_end){auto end=bytes.find("\r\n",at);auto line=bytes.substr(at,end-at);auto colon=line.find(':');if(colon==std::string_view::npos)http_fail(SdkError::Kind::ResponseDecoding,source,"malformed MIME header");auto name=line.substr(0,colon);auto value=line.substr(colon+1);while(!value.empty()&&(value.front()==' '||value.front()=='\t'))value.remove_prefix(1);while(!value.empty()&&(value.back()==' '||value.back()=='\t'))value.remove_suffix(1);
            if(++header_count>256)http_fail(SdkError::Kind::ResourceLimit,source,"MIME header count ceiling exceeded");
            if(!header_name(name)||!header_value(value))http_fail(SdkError::Kind::ResponseDecoding,source,"invalid MIME header");
            if(lower_ascii(name)=="content-transfer-encoding")http_fail(SdkError::Kind::ResponseDecoding,source,"MIME transfer encoding requires a separate codec profile");
            part.headers.emplace_back(name,value);at=end+2;
        }
        at=head_end+4;auto next=find(at);if(!next||next->at<at+2)http_fail(SdkError::Kind::ResponseDecoding,source,"multipart closing boundary is absent");
        auto data=bytes.substr(at,next->at-at-2);if(data.size()>settings.max_part_bytes)http_fail(SdkError::Kind::ResourceLimit,source,"multipart part exceeds byte ceiling");part.bytes=std::string(data);
        auto fields=disposition_fields(part,source);if(!fields||fields->subtype!="form-data"||!fields->parameters.contains("name"))http_fail(SdkError::Kind::ResponseDecoding,source,"named multipart requires a form-data name");
        part.name=context.text(fields->parameters.at("name"),source,"");parts.push_back(std::move(part));delimiter=next;
    }return parts;
}
std::vector<RawPart> decode_form(std::string_view bytes,Context& context,const Settings& settings,const Source& source){
    std::vector<RawPart> fields;if(bytes.empty())return fields;
    for(;;){if(fields.size()==settings.max_parts)http_fail(SdkError::Kind::ResourceLimit,source,"form field count exceeded");auto end=bytes.find('&');auto pair=bytes.substr(0,end);auto equal=pair.find('=');if(equal==std::string_view::npos)http_fail(SdkError::Kind::ResponseDecoding,source,"form pair requires name=value");
        auto value=pair.substr(equal+1);if(value.size()>settings.max_part_bytes)http_fail(SdkError::Kind::ResourceLimit,source,"form value exceeds part ceiling");fields.push_back({decode_component(pair.substr(0,equal),true,context,source),{},std::string(value)});
        if(end==std::string_view::npos)break;bytes.remove_prefix(end+1);
    }return fields;
}
std::string encode_form_field(const PartRules& rules,const JsonValue& json,std::string_view name,Context& context,std::size_t limit){
    if(rules.encoding==PartEncoding::Style)return encode_parameter(name,Location::Query,rules.serialization,json,context,rules.source,limit);
    auto bytes=part_json_bytes(rules,json,context,std::min(limit,rules.max_bytes));auto key=encode_component(name,Encoding::Form,context,rules.source,limit);append_bounded(key,"=",limit,rules.source,context);auto value=encode_component(bytes,rules.outer,context,rules.source,limit-key.size());append_bounded(key,value,limit,rules.source,context);return key;
}
JsonValue form_value(const PartRules& rules,const std::vector<RawPart>& parts,Context& context){
    if(parts.empty())http_fail(SdkError::Kind::ResponseDecoding,rules.source,"form field is absent");
    if(rules.encoding!=PartEncoding::Style){if(parts.size()!=1)http_fail(SdkError::Kind::ResponseDecoding,rules.source,"duplicate scalar form field");return part_json_value(rules,decode_component(parts[0].bytes,rules.outer==Encoding::Form,context,rules.source),context);}
    const auto& shape=rules.serialization.shape;
    if(shape.kind==ShapeKind::Scalar){if(parts.size()!=1)http_fail(SdkError::Kind::ResponseDecoding,rules.source,"duplicate scalar form field");return parse_scalar(decode_component(parts[0].bytes,true,context,rules.source),shape.scalar,context,rules.source);}
    if(shape.kind==ShapeKind::Array){
        JsonValue::Array result;
        if(rules.serialization.explode){for(const auto& part:parts)result.push_back(parse_scalar(decode_component(part.bytes,true,context,rules.source),shape.scalar,context,rules.source));}
        else{if(parts.size()!=1)http_fail(SdkError::Kind::ResponseDecoding,rules.source,"duplicate non-exploded array field");std::string_view rest=parts[0].bytes;const auto delimiter=rules.serialization.style==Style::SpaceDelimited?"%20":rules.serialization.style==Style::PipeDelimited?"%7C":",";for(;;){auto next=rest.find(delimiter);result.push_back(parse_scalar(decode_component(rest.substr(0,next),true,context,rules.source),shape.scalar,context,rules.source));if(next==std::string_view::npos)break;rest.remove_prefix(next+std::string_view(delimiter).size());}}
        return JsonValue(std::move(result));
    }
    if(parts.size()!=1||rules.serialization.explode)http_fail(SdkError::Kind::ResponseDecoding,rules.source,"ambiguous exploded object form decoding");
    JsonValue::Object result;std::string_view rest=parts[0].bytes;
    const auto delimiter=rules.serialization.style==Style::SpaceDelimited?"%20":rules.serialization.style==Style::PipeDelimited?"%7C":",";
    std::vector<std::string> pieces;
    for(;;){auto end=rest.find(delimiter);pieces.push_back(decode_component(rest.substr(0,end),true,context,rules.source));if(end==std::string_view::npos)break;rest.remove_prefix(end+std::string_view(delimiter).size());}
    if(pieces.size()%2)http_fail(SdkError::Kind::ResponseDecoding,rules.source,"odd packed form-object key/value count");
    for(std::size_t i=0;i<pieces.size();i+=2){auto key=shape.properties.find(pieces[i]);if(key==shape.properties.end()&&!shape.additional)http_fail(SdkError::Kind::ResponseDecoding,rules.source,"unknown packed form-object key");
        auto scalar=key==shape.properties.end()?shape.extra:key->second;
        if(!result.emplace(pieces[i],parse_scalar(pieces[i+1],scalar,context,rules.source)).second)http_fail(SdkError::Kind::ResponseDecoding,rules.source,"duplicate packed form-object key");}
    return JsonValue(std::move(result));
}

JsonValue styled_part_value(const PartRules& rules,std::string_view bytes,Context& context){
    const auto& shape=rules.serialization.shape;
    const auto style=rules.serialization.style;
    auto prefix=encode_component(rules.name,Encoding::Component,context,rules.source,rules.max_bytes)+"=";
    auto value_text=[&](std::string_view value){return rules.serialization.encoding==Encoding::None?context.text(value,rules.source,""):decode_component(value,false,context,rules.source);};
    auto scalar=[&](std::string_view value,Scalar kind){return parse_scalar(value_text(value),kind,context,rules.source);};
    auto split=[&](std::string_view value,std::string_view delimiter){std::vector<std::string_view> result;for(;;){context.spend(1,rules.source,"");auto next=value.find(delimiter);result.push_back(value.substr(0,next));if(next==std::string_view::npos)break;value.remove_prefix(next+delimiter.size());}return result;};
    if(shape.kind==ShapeKind::Scalar){if(!bytes.starts_with(prefix))http_fail(SdkError::Kind::ResponseDecoding,rules.source,"styled part name is absent");return scalar(bytes.substr(prefix.size()),shape.scalar);}
    const auto delimiter=style==Style::SpaceDelimited?"%20":style==Style::PipeDelimited?"%7C":",";
    if(shape.kind==ShapeKind::Array){
        JsonValue::Array values;
        if(rules.serialization.explode){for(auto item:split(bytes,"&")){if(!item.starts_with(prefix))http_fail(SdkError::Kind::ResponseDecoding,rules.source,"styled array part name differs");values.push_back(scalar(item.substr(prefix.size()),shape.scalar));}}
        else{if(!bytes.starts_with(prefix))http_fail(SdkError::Kind::ResponseDecoding,rules.source,"styled array part name absent");for(auto item:split(bytes.substr(prefix.size()),delimiter))values.push_back(scalar(item,shape.scalar));}
        return JsonValue(std::move(values));
    }
    JsonValue::Object values;
    auto insert=[&](std::string_view raw_key,std::string_view value){auto key=value_text(raw_key);auto type=shape.properties.find(key);if(type==shape.properties.end()&&!shape.additional)http_fail(SdkError::Kind::ResponseDecoding,rules.source,"unknown styled object key");
        if(!values.emplace(key,scalar(value,type==shape.properties.end()?shape.extra:type->second)).second)http_fail(SdkError::Kind::ResponseDecoding,rules.source,"duplicate styled object key");};
    if(rules.serialization.explode||style==Style::DeepObject){for(auto pair:split(bytes,"&")){auto equal=pair.find('=');if(equal==std::string_view::npos)http_fail(SdkError::Kind::ResponseDecoding,rules.source,"styled object key/value separator missing");auto key=pair.substr(0,equal);
            if(style==Style::DeepObject){auto start=prefix.substr(0,prefix.size()-1)+"%5B";if(!key.starts_with(start)||!key.ends_with("%5D"))http_fail(SdkError::Kind::ResponseDecoding,rules.source,"invalid deep-object part key");key=key.substr(start.size(),key.size()-start.size()-3);}
            insert(key,pair.substr(equal+1));}}
    else{if(!bytes.starts_with(prefix))http_fail(SdkError::Kind::ResponseDecoding,rules.source,"styled object part name absent");auto parts=split(bytes.substr(prefix.size()),delimiter);if(parts.size()%2)http_fail(SdkError::Kind::ResponseDecoding,rules.source,"odd styled object key/value count");for(std::size_t i=0;i<parts.size();i+=2)insert(parts[i],parts[i+1]);}
    return JsonValue(std::move(values));
}
} // namespace @NAMESPACE@::detail
