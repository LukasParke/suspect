#include "@PACKAGE@/protocol.hpp"
#include <charconv>

namespace @NAMESPACE@::detail {
void append_bounded(std::string& out,std::string_view value,std::size_t limit,const Source& source,Context& context){
    if(out.size()>limit||value.size()>limit-out.size())http_fail(SdkError::Kind::ResourceLimit,source,"serialized value exceeds remaining byte ceiling");
    context.spend(value.size(),source,"");out.append(value);
}
static bool alnum(unsigned char c){return(c>='a'&&c<='z')||(c>='A'&&c<='Z')||(c>='0'&&c<='9');}
static bool unreserved(unsigned char c){return alnum(c)||std::string_view("-._~").find(static_cast<char>(c))!=std::string_view::npos;}
static int hex(char c){if(c>='0'&&c<='9')return c-'0';if(c>='a'&&c<='f')return c-'a'+10;if(c>='A'&&c<='F')return c-'A'+10;return -1;}
std::string encode_component(std::string_view text,Encoding encoding,Context& context,const Source& source,std::size_t limit){
    if(text.size()>limit)http_fail(SdkError::Kind::ResourceLimit,source,"encoded component exceeds byte ceiling");
    context.spend(text.size(),source,"");
    if(encoding==Encoding::None)return std::string(text);
    std::string out;static constexpr char digits[]="0123456789ABCDEF";
    for(std::size_t at=0;at<text.size();++at){
        unsigned char c=static_cast<unsigned char>(text[at]);
        bool reserved=std::string_view(":/?#[]@!$&'()*+,;=").find(static_cast<char>(c))!=std::string_view::npos;
        if(encoding==Encoding::Reserved&&c=='%'&&at+2<text.size()&&hex(text[at+1])>=0&&hex(text[at+2])>=0){append_bounded(out,text.substr(at,3),limit,source,context);at+=2;continue;}
        bool pass=encoding==Encoding::Form?(alnum(c)||std::string_view("*-._").find(static_cast<char>(c))!=std::string_view::npos):(unreserved(c)||(encoding==Encoding::Reserved&&reserved));
        if(pass)append_bounded(out,text.substr(at,1),limit,source,context);
        else if(encoding==Encoding::Form&&c==' ')append_bounded(out,"+",limit,source,context);
        else{char encoded[3]={'%',digits[c>>4],digits[c&15]};append_bounded(out,std::string_view(encoded,3),limit,source,context);}
    }return out;
}
std::string decode_component(std::string_view text,bool form,Context& context,const Source& source){
    context.spend(text.size(),source,"");std::string value;
    for(std::size_t at=0;at<text.size();++at){char c=text[at];if(c=='%'){
        if(at+2>=text.size()||hex(text[at+1])<0||hex(text[at+2])<0)http_fail(SdkError::Kind::ResponseDecoding,source,"invalid percent escape");
        value.push_back(static_cast<char>(hex(text[at+1])*16+hex(text[at+2])));at+=2;
    }else value.push_back(form&&c=='+'?' ':c);}
    return context.text(value,source,"");
}
std::string scalar_text(const JsonValue& value,Scalar scalar,const Source& source){
    if(value.is<std::string>()&&(scalar==Scalar::Any||scalar==Scalar::String))return value.as<std::string>();
    if(value.is<bool>()&&(scalar==Scalar::Any||scalar==Scalar::Boolean))return value.as<bool>()?"true":"false";
    if(value.is<JsonNumber>()&&(scalar==Scalar::Any||scalar==Scalar::Number||scalar==Scalar::Integer)){
        if(scalar==Scalar::Integer&&!value.as<JsonNumber>().is_integer())http_fail(SdkError::Kind::RequestRepresentation,source,"nonintegral wire integer");
        return value.as<JsonNumber>().token();
    }
    http_fail(SdkError::Kind::RequestRepresentation,source,"wire value is not the declared non-null scalar");
}
JsonValue parse_scalar(std::string_view text,Scalar scalar,Context& context,const Source& source){
    context.spend(text.size(),source,"");
    if(scalar==Scalar::String)return JsonValue(context.text(text,source,""));
    if(scalar==Scalar::Boolean){if(text=="true")return JsonValue(true);if(text=="false")return JsonValue(false);}
    if(scalar==Scalar::Number||scalar==Scalar::Integer){auto number=JsonNumber::parse(text);if(number&&(scalar!=Scalar::Integer||number.value().is_integer()))return JsonValue(std::move(number).value());}
    http_fail(SdkError::Kind::ResponseDecoding,source,"text does not represent the declared scalar type");
}
static void data_guard(std::string_view value,Location location,const Serialization& serialization,const Source& source){
    if(serialization.encoding==Encoding::None){
        for(unsigned char c:value)if((c<32&&!(location==Location::Header&&c=='\t'))||c==127)http_fail(SdkError::Kind::RequestRepresentation,source,"header/cookie data contains a control character");
        if(location==Location::Cookie)for(unsigned char c:value)if(c>126||std::string_view(" \t\",;\\").find(static_cast<char>(c))!=std::string_view::npos)http_fail(SdkError::Kind::RequestRepresentation,source,"cookie value requires caller-defined escaping");
    }
    auto contains=[&](std::string_view chars){return value.find_first_of(chars)!=std::string_view::npos;};
    if(serialization.encoding==Encoding::None&&serialization.shape.kind!=ShapeKind::Scalar){
        bool ambiguous=(serialization.style==Style::Simple&&contains(","))
            ||(serialization.style==Style::Form&&contains(serialization.explode?"&":","))
            ||(serialization.style==Style::DeepObject&&contains("&="))
            ||(serialization.style==Style::Cookie&&contains(serialization.explode?";":","));
        if(serialization.shape.kind==ShapeKind::Object&&serialization.explode)ambiguous|=contains("=");
        if(location==Location::Header&&!value.empty())ambiguous|=value.front()==' '||value.front()=='\t'||value.back()==' '||value.back()=='\t';
        if(ambiguous)http_fail(SdkError::Kind::RequestRepresentation,source,"unescaped composite header/part delimiter is ambiguous");
    }
    if((serialization.style==Style::SpaceDelimited&&contains(" "))||(serialization.style==Style::PipeDelimited&&contains("|"))||(serialization.style==Style::DeepObject&&contains("[]")))
        http_fail(SdkError::Kind::RequestRepresentation,source,"value contains an ambiguous style delimiter");
    if(serialization.encoding==Encoding::Reserved){
        bool hazard=location==Location::Path?contains("#[]/?"):(location==Location::Query||location==Location::Querystring)?contains("#[]&=+"):location==Location::Cookie?contains(";,"):false;
        if(serialization.shape.kind!=ShapeKind::Scalar){switch(serialization.style){case Style::Simple:case Style::Form:case Style::Cookie:hazard|=contains(",");break;case Style::Label:hazard|=contains(".,");break;case Style::Matrix:hazard|=contains(";,");break;default:break;}}
        if(hazard)http_fail(SdkError::Kind::RequestRepresentation,source,"reserved expansion needs caller-escaped URI/form delimiters");
    }
}
std::string encode_parameter(std::string_view name,Location location,const Serialization& serialization,const JsonValue& value,Context& context,const Source& source,std::size_t limit){
    if(serialization.style==Style::Content){
        auto text=serialization.content_json?write_document(value,context,source):scalar_text(value,Scalar::Any,source);
        data_guard(text,location,serialization,source);auto encoded=encode_component(text,serialization.encoding,context,source,limit);
        if(location!=Location::Query&&location!=Location::Cookie)return encoded;
        std::string result=encode_component(name,Encoding::Component,context,source,limit);append_bounded(result,"=",limit,source,context);append_bounded(result,encoded,limit,source,context);return result;
    }
    context.spend(1,source,"");const auto& shape=serialization.shape;
    auto encode=[&](std::string_view text){data_guard(text,location,serialization,source);return encode_component(text,serialization.encoding,context,source,limit);};
    auto key=encode_component(name,location==Location::Header||serialization.style==Style::Cookie?Encoding::None:Encoding::Component,context,source,limit);
    Presence<std::string> scalar;std::vector<std::string> items;std::vector<std::pair<std::string,std::string>> properties;
    std::size_t cost=0;
    auto remember=[&](std::string text){if(text.size()>limit-cost)http_fail(SdkError::Kind::ResourceLimit,source,"parameter expansion exceeds byte ceiling");cost+=text.size();return text;};
    if(shape.kind==ShapeKind::Scalar)scalar=remember(encode(scalar_text(value,shape.scalar,source)));
    else if(shape.kind==ShapeKind::Array){
        if(!value.is<JsonValue::Array>()||value.as<JsonValue::Array>().empty())http_fail(SdkError::Kind::RequestRepresentation,source,"empty or non-array composite has no parameter expansion");
        const auto& array=value.as<JsonValue::Array>();
        if((serialization.style==Style::Form||serialization.style==Style::Cookie||serialization.style==Style::Matrix)&&serialization.explode&&array.size()>limit/(key.size()+2))http_fail(SdkError::Kind::ResourceLimit,source,"repeated parameter names exceed byte ceiling");
        for(const auto& item:array){context.spend(1,source,"");items.push_back(remember(encode(scalar_text(item,shape.scalar,source))));}
    }else{
        if(!value.is<JsonValue::Object>()||value.as<JsonValue::Object>().empty())http_fail(SdkError::Kind::RequestRepresentation,source,"empty or non-object composite has no parameter expansion");
        for(const auto& [property,item]:value.as<JsonValue::Object>()){
            context.spend(property.size()+1,source,"");auto found=shape.properties.find(property);
            if(found==shape.properties.end()&&!shape.additional)http_fail(SdkError::Kind::RequestRepresentation,source,"unknown flat-object member");
            auto kind=found==shape.properties.end()?shape.extra:found->second;
            properties.emplace_back(remember(encode(property)),remember(encode(scalar_text(item,kind,source))));
        }
    }
    std::string result;
    auto add=[&](std::string_view text){append_bounded(result,text,limit,source,context);};
    auto joined_items=[&](std::string_view delimiter){for(std::size_t i=0;i<items.size();++i){if(i)add(delimiter);add(items[i]);}};
    auto joined_properties=[&](std::string_view delimiter,bool pairs){for(std::size_t i=0;i<properties.size();++i){if(i)add(delimiter);add(properties[i].first);add(pairs?"=":delimiter);add(properties[i].second);}};
    auto simple=[&](std::string_view delimiter,bool pairs){if(scalar)add(*scalar);else if(shape.kind==ShapeKind::Array)joined_items(delimiter);else joined_properties(delimiter,pairs);};
    switch(serialization.style){
        case Style::Simple:simple(",",serialization.explode);break;
        case Style::Label:add(".");simple(serialization.explode?".":",",serialization.explode);break;
        case Style::Matrix:
            if(scalar){add(";");add(key);if(!scalar->empty()){add("=");add(*scalar);}}
            else if(!serialization.explode){add(";");add(key);add("=");simple(",",false);}
            else if(shape.kind==ShapeKind::Array){for(const auto& item:items){add(";");add(key);if(!item.empty()){add("=");add(item);}}}
            else for(const auto& [name,item]:properties){add(";");add(name);if(!item.empty()){add("=");add(item);}}
            break;
        case Style::Form:case Style::Cookie:{
            auto delimiter=serialization.style==Style::Cookie?"; ":"&";
            if(scalar){add(key);add("=");add(*scalar);}
            else if(!serialization.explode){add(key);add("=");simple(",",false);}
            else if(shape.kind==ShapeKind::Array){for(std::size_t i=0;i<items.size();++i){if(i)add(delimiter);add(key);add("=");add(items[i]);}}
            else joined_properties(delimiter,true);
            break;
        }
        case Style::SpaceDelimited:case Style::PipeDelimited:add(key);add("=");simple(serialization.style==Style::SpaceDelimited?"%20":"%7C",false);break;
        case Style::DeepObject:for(std::size_t i=0;i<properties.size();++i){if(i)add("&");add(key);add("%5B");add(properties[i].first);add("%5D=");add(properties[i].second);}break;
        case Style::Content:break;
    }return result;
}
Presence<std::string> find_header(const Headers& headers,std::string_view name,const Source& source,bool list){
    Presence<std::string> found;for(const auto& [key,value]:headers)if(lower_ascii(key)==lower_ascii(name)){
        if(found){if(!list||lower_ascii(name)=="set-cookie")http_fail(SdkError::Kind::ResponseDecoding,source,"duplicate scalar response header");*found+=","+value;}else found=value;
    }
    return found;
}
JsonValue decode_header(const Headers& headers,std::string_view name,const Serialization& serialization,Context& context,const Source& source){
    if(lower_ascii(name)=="set-cookie"&&serialization.shape.kind==ShapeKind::Array&&serialization.shape.scalar==Scalar::String){
        JsonValue::Array values;for(const auto& [key,value]:headers)if(lower_ascii(key)=="set-cookie")values.push_back(parse_scalar(value,Scalar::String,context,source));return JsonValue(std::move(values));
    }
    auto text=find_header(headers,name,source,serialization.shape.kind!=ShapeKind::Scalar);if(!text)http_fail(SdkError::Kind::ResponseDecoding,source,"required header is absent");
    if(serialization.style==Style::Content)return serialization.content_json?parse_document(*text,context,source):parse_scalar(*text,serialization.shape.scalar,context,source);
    if(serialization.shape.kind==ShapeKind::Scalar)return parse_scalar(*text,serialization.shape.scalar,context,source);
    std::vector<std::string_view> parts;std::string_view rest=*text;
    for(;;){auto comma=rest.find(',');auto part=rest.substr(0,comma);while(!part.empty()&&(part.front()==' '||part.front()=='\t'))part.remove_prefix(1);while(!part.empty()&&(part.back()==' '||part.back()=='\t'))part.remove_suffix(1);parts.push_back(part);if(comma==std::string_view::npos)break;rest.remove_prefix(comma+1);context.spend(1,source,"");}
    if(serialization.shape.kind==ShapeKind::Array){JsonValue::Array values;for(auto part:parts)values.push_back(parse_scalar(part,serialization.shape.scalar,context,source));return JsonValue(std::move(values));}
    JsonValue::Object values;
    auto put=[&](std::string_view key,std::string_view value){auto found=serialization.shape.properties.find(key);if(found==serialization.shape.properties.end()&&!serialization.shape.additional)http_fail(SdkError::Kind::ResponseDecoding,source,"unknown response header object key");auto scalar=found==serialization.shape.properties.end()?serialization.shape.extra:found->second;
        if(!values.emplace(context.text(key,source,""),parse_scalar(value,scalar,context,source)).second)http_fail(SdkError::Kind::ResponseDecoding,source,"duplicate response header object key");};
    if(serialization.explode){for(auto part:parts){auto equal=part.find('=');if(equal==std::string_view::npos)http_fail(SdkError::Kind::ResponseDecoding,source,"missing header object separator");put(part.substr(0,equal),part.substr(equal+1));}}
    else{if(parts.size()%2)http_fail(SdkError::Kind::ResponseDecoding,source,"odd header object key/value count");for(std::size_t i=0;i<parts.size();i+=2)put(parts[i],parts[i+1]);}
    return JsonValue(std::move(values));
}

static std::string normalize_path(std::string_view path,const Source& source,Context& context,std::size_t limit){
    // RFC3986 removes literal dot segments only. Percent-encoded dots/slashes
    // and empty segments are data, retaining the exact source path spelling.
    std::string result;
    while(!path.empty()){
        if(path.starts_with("../")){path.remove_prefix(3);continue;}if(path.starts_with("./")){path.remove_prefix(2);continue;}
        if(path.starts_with("/./")){path.remove_prefix(2);continue;}if(path=="/."){path="/";continue;}
        if(path.starts_with("/../")||path=="/.."){
            path=path=="/.."?std::string_view("/"):path.substr(3);auto slash=result.rfind('/');result.resize(slash==std::string::npos?0:slash);continue;
        }
        if(path=="."||path==".."){path={};continue;}
        auto next=path.find('/',path.starts_with('/')?1:0);auto segment=path.substr(0,next);
        append_bounded(result,segment,limit,source,context);if(next==std::string_view::npos)break;path.remove_prefix(next);
    }
    if(result.empty())append_bounded(result,"/",limit,source,context);return result;
}
struct Url {std::string authority,path;};
static Url absolute_url(std::string_view text,const Source& source,Context& context,std::size_t limit){
    if(text.size()>limit)http_fail(SdkError::Kind::ResourceLimit,source,"URL exceeds byte ceiling");
    auto sep=text.find("://");if(sep==std::string_view::npos)http_fail(SdkError::Kind::Configuration,source,"relative server requires an HTTP document URL");
    auto scheme=lower_ascii(text.substr(0,sep));if(scheme!="https"&&scheme!="http")http_fail(SdkError::Kind::RequestRepresentation,source,"server URL requires HTTP or HTTPS");
    auto rest=text.substr(sep+3);auto slash=rest.find('/');auto host=rest.substr(0,slash);
    if(host.empty()||host.find('@')!=std::string_view::npos)http_fail(SdkError::Kind::RequestRepresentation,source,"server authority is empty or contains userinfo");
    for(unsigned char c:text)if(c<=32||c==127||c=='\\'||c=='?'||c=='#'||c=='{'||c=='}')http_fail(SdkError::Kind::RequestRepresentation,source,"server URL contains invalid delimiters");
    for(unsigned char c:host)if(c>126)http_fail(SdkError::Kind::RequestRepresentation,source,"server host must be an ASCII/IDNA authority");
    auto authority=lower_ascii(host);
    std::string_view port;
    if(authority.front()=='['){auto end=authority.find(']');if(end==std::string::npos)http_fail(SdkError::Kind::RequestRepresentation,source,"unclosed IPv6 host");if(end+1<authority.size()){if(authority[end+1]!=':')http_fail(SdkError::Kind::RequestRepresentation,source,"invalid IPv6 authority");port=std::string_view(authority).substr(end+2);}}
    else if(auto colon=authority.find(':');colon!=std::string::npos)port=std::string_view(authority).substr(colon+1);
    if(!port.empty()){unsigned n=0;auto parsed=std::from_chars(port.data(),port.data()+port.size(),n);if(parsed.ec!=std::errc{}||parsed.ptr!=port.data()+port.size()||n>65535)http_fail(SdkError::Kind::RequestRepresentation,source,"invalid server port");authority.resize(authority.size()-port.size()-1);if(!((scheme=="http"&&n==80)||(scheme=="https"&&n==443)))authority+=":"+std::to_string(n);}
    else if(authority.ends_with(':'))authority.pop_back();
    if(authority.empty()||authority.front()==':')http_fail(SdkError::Kind::RequestRepresentation,source,"empty server host");
    std::string path=slash==std::string_view::npos?"/":std::string(rest.substr(slash));
    for(unsigned char c:path)if(!alnum(c)&&std::string_view("/-._~!$&'()*+,;=:@%").find(static_cast<char>(c))==std::string_view::npos)http_fail(SdkError::Kind::RequestRepresentation,source,"server path requires an RFC3986 URI spelling");
    for(std::size_t at=0;at<path.size();++at)if(path[at]=='%'){if(at+2>=path.size()||hex(path[at+1])<0||hex(path[at+2])<0)http_fail(SdkError::Kind::RequestRepresentation,source,"invalid server percent escape");at+=2;}
    return {scheme+"://"+authority,normalize_path(path,source,context,limit)};
}
static std::string server_url(const Operation& operation,const Settings& settings,Context& context){
    if(settings.server_url){auto url=absolute_url(*settings.server_url,operation.source,context,settings.max_request_bytes);return url.authority+url.path;}
    if(settings.server_index>=operation.servers.size())http_fail(SdkError::Kind::Configuration,operation.source,"selected server is not declared");
    const auto& server=operation.servers[settings.server_index];
    for(const auto& [key,value]:settings.variables){(void)value;if(std::none_of(server.variables.begin(),server.variables.end(),[&](const auto& variable){return variable.name==key;}))http_fail(SdkError::Kind::Configuration,server.source,"unknown server variable");}
    std::string expanded;std::string_view rest=server.url;
    while(true){auto open=rest.find('{');if(open==std::string_view::npos){append_bounded(expanded,rest,settings.max_request_bytes,server.source,context);break;}
        append_bounded(expanded,rest.substr(0,open),settings.max_request_bytes,server.source,context);auto close=rest.find('}',open+1);if(close==std::string_view::npos)http_fail(SdkError::Kind::Configuration,server.source,"invalid server template");
        auto name=rest.substr(open+1,close-open-1);auto variable=std::find_if(server.variables.begin(),server.variables.end(),[&](const auto& v){return v.name==name;});if(variable==server.variables.end())http_fail(SdkError::Kind::Configuration,server.source,"undeclared server variable");
        auto override=settings.variables.find(variable->name);const auto& value=override==settings.variables.end()?variable->default_value:override->second;
        if(variable->values&&std::find(variable->values->begin(),variable->values->end(),value)==variable->values->end())http_fail(SdkError::Kind::Configuration,variable->source,"server variable is outside its enum");
        append_bounded(expanded,value,settings.max_request_bytes,variable->source,context);rest.remove_prefix(close+1);
    }
    if(expanded.find("://")!=std::string::npos){auto url=absolute_url(expanded,server.source,context,settings.max_request_bytes);return url.authority+url.path;}
    if(expanded.substr(0,expanded.find('/')).find(':')!=std::string::npos)http_fail(SdkError::Kind::RequestRepresentation,server.source,"HTTP server requires an authority or a relative URL reference");
    std::string_view document=settings.document_url?*settings.document_url:server.document_url;
    document=document.substr(0,document.find_first_of("?#"));
    auto base=absolute_url(document,server.source,context,settings.max_request_bytes);
    if(expanded.starts_with("//")){auto scheme=base.authority.substr(0,base.authority.find(':'));auto url=absolute_url(scheme+":"+expanded,server.source,context,settings.max_request_bytes);return url.authority+url.path;}
    auto path=expanded.starts_with('/')?expanded:base.path.substr(0,base.path.rfind('/')+1)+expanded;
    auto url=absolute_url(base.authority+path,server.source,context,settings.max_request_bytes);return url.authority+url.path;
}
static bool bearer(std::string_view token){if(token.size()>8192)return false;while(!token.empty()&&token.back()=='=')token.remove_suffix(1);if(token.empty())return false;for(unsigned char c:token)if(!unreserved(c)&&c!='+'&&c!='/')return false;return true;}
static std::string base64(std::string_view bytes){static constexpr char alphabet[]="ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";std::string out;
    for(std::size_t i=0;i<bytes.size();i+=3){unsigned a=static_cast<unsigned char>(bytes[i]),b=i+1<bytes.size()?static_cast<unsigned char>(bytes[i+1]):0,c=i+2<bytes.size()?static_cast<unsigned char>(bytes[i+2]):0;out.push_back(alphabet[a>>2]);out.push_back(alphabet[((a&3)<<4)|(b>>4)]);out.push_back(i+1<bytes.size()?alphabet[((b&15)<<2)|(c>>6)]:'=');out.push_back(i+2<bytes.size()?alphabet[c&63]:'=');}return out;}
HttpRequest prepare_request(const Operation& operation,const std::vector<ParameterValue>& parameters,Presence<EncodedBody> body,const CredentialValues& credentials,const Settings& settings,Context& context){
    HttpRequest request;request.method=operation.method;request.url=server_url(operation,settings,context);
    const auto effective_server=request.url;
    if(request.url.ends_with('/'))request.url.pop_back();
    auto limit=settings.max_request_bytes;std::string path;
    for(std::size_t at=0;at<operation.path.size();){auto open=operation.path.find('{',at);if(open==std::string::npos){append_bounded(path,std::string_view(operation.path).substr(at),limit,operation.source,context);break;}
        append_bounded(path,std::string_view(operation.path).substr(at,open-at),limit,operation.source,context);auto close=operation.path.find('}',open+1);if(close==std::string::npos)http_fail(SdkError::Kind::RequestRepresentation,operation.source,"unclosed path template");auto name=std::string_view(operation.path).substr(open+1,close-open-1);
        auto found=std::find_if(parameters.begin(),parameters.end(),[&](const auto& p){return p.location==Location::Path&&p.name==name;});if(found==parameters.end())http_fail(SdkError::Kind::RequestValidation,operation.source,"required path value is absent");
        auto value=encode_parameter(name,Location::Path,found->serialization,found->value,context,found->source,limit-path.size());
        if(value=="."||value=="..")http_fail(SdkError::Kind::RequestRepresentation,found->source,"path value is a dot segment");append_bounded(path,value,limit,found->source,context);at=close+1;
    }
    append_bounded(request.url,path,limit,operation.source,context);
    std::string query,cookie;bool whole_query=false;std::set<std::string> query_keys,cookie_keys;
    auto header=[&](std::string name,std::string value,const Source& source){if(!header_name(name)||!header_value(value))http_fail(SdkError::Kind::RequestRepresentation,source,"invalid request header");if(std::any_of(request.headers.begin(),request.headers.end(),[&](const auto& h){return lower_ascii(h.first)==lower_ascii(name);}))http_fail(SdkError::Kind::RequestRepresentation,source,"conflicting request header declarations or credentials");request.headers.emplace_back(std::move(name),std::move(value));};
    auto query_part=[&](std::string_view value,const Source& source){
        auto parts=value;for(;;){auto end=parts.find('&');auto pair=parts.substr(0,end);auto equal=pair.find('=');if(equal!=std::string_view::npos)query_keys.insert(decode_component(pair.substr(0,equal),false,context,source));if(end==std::string_view::npos)break;parts.remove_prefix(end+1);}
        if(!query.empty())append_bounded(query,"&",limit-request.url.size(),source,context);append_bounded(query,value,limit-request.url.size(),source,context);};
    for(const auto& parameter:parameters){if(parameter.location==Location::Path)continue;
        auto value=parameter.encoded_form?*parameter.encoded_form:encode_parameter(parameter.name,parameter.location,parameter.serialization,parameter.value,context,parameter.source,limit-request.url.size());
        switch(parameter.location){
            case Location::Query:query_keys.insert(parameter.name);query_part(value,parameter.source);break;
            case Location::Querystring:if(whole_query||!query.empty())http_fail(SdkError::Kind::RequestRepresentation,parameter.source,"whole query content conflicts with query fields");whole_query=true;query=value;break;
            case Location::Header:header(parameter.name,std::move(value),parameter.source);break;
            case Location::Cookie:cookie_keys.insert(parameter.name);if(parameter.serialization.explode&&parameter.value.is<JsonValue::Object>())for(const auto& [name,item]:parameter.value.as<JsonValue::Object>()){(void)item;cookie_keys.insert(name);}if(!cookie.empty())append_bounded(cookie,"; ",settings.transfer.max_header_bytes,parameter.source,context);append_bounded(cookie,value,settings.transfer.max_header_bytes,parameter.source,context);break;
            case Location::Path:break;
        }
    }
    if(!operation.security.empty()){
        Presence<std::size_t> selected=settings.security_alternative;
        auto available=[&](std::size_t index){return index<operation.security.size()&&std::all_of(operation.security[index].begin(),operation.security[index].end(),[&](const auto& r){return credentials.contains(r.field);});};
        if(!selected){for(std::size_t i=0;i<operation.security.size();++i)if(available(i)){selected=i;break;}}
        if(!selected||!available(*selected)){
            const Source* source=&operation.source;
            const auto candidate=selected.value_or(0);
            if(candidate<operation.security.size())for(const auto& requirement:operation.security[candidate])if(!credentials.contains(requirement.field)){source=&requirement.source;break;}
            http_fail(SdkError::Kind::RequestValidation,*source,"no selected security alternative has every required credential");
        }
        for(const auto& requirement:operation.security[*selected]){
            const auto& supplied=credentials.at(requirement.field);std::string value;
            if(requirement.kind==CredentialKind::Basic){
                const auto* ref=std::get_if<std::reference_wrapper<const BasicCredentials>>(&supplied);if(!ref)http_fail(SdkError::Kind::Configuration,requirement.source,"wrong Basic credential type");const auto& basic=ref->get();
                auto control=[](std::string_view value){return std::any_of(value.begin(),value.end(),[](unsigned char c){return c<32||c==127;});};
                if(basic.username.size()>8192||basic.password.size()>8192||basic.username.find(':')!=std::string::npos||control(basic.username)||control(basic.password))http_fail(SdkError::Kind::RequestValidation,requirement.source,"Basic credentials exceed their syntax/size policy");
                value="Basic "+base64(context.text(basic.username,requirement.source,"")+":"+context.text(basic.password,requirement.source,""));header("Authorization",std::move(value),requirement.source);
            }else if(requirement.kind==CredentialKind::Provider){
                const auto* ref=std::get_if<std::reference_wrapper<const CredentialProvider>>(&supplied);if(!ref||!ref->get())http_fail(SdkError::Kind::Configuration,requirement.source,"credential provider is absent");
                CredentialRequest call{operation.source,requirement.source,operation.id,requirement.scheme,requirement.scopes,requirement.roles,context.copy(requirement.metadata,requirement.source,""),settings.transfer.stop,settings.transfer.deadline,effective_server};
                Result<Authorization,TransportError> credential=[&](){try{return ref->get()(call);}catch(...){TransportError error;error.cause=std::current_exception();error.message="credential provider threw";return Result<Authorization,TransportError>::failure(std::move(error));}}();
                context.control.check(requirement.source);
                if(!credential){auto error=transport_error(std::move(credential).error(),requirement.source);throw HttpFailure{std::move(error)};}
                auto authorization=std::move(credential).value();if(!header_name(authorization.scheme)||authorization.value.empty()||authorization.value.size()>8192||!header_value(authorization.value))http_fail(SdkError::Kind::RequestValidation,requirement.source,"invalid provider authorization credential");
                header("Authorization",authorization.scheme+" "+authorization.value,requirement.source);
            }else{
                const auto* ref=std::get_if<std::reference_wrapper<const std::string>>(&supplied);if(!ref)http_fail(SdkError::Kind::Configuration,requirement.source,"wrong credential value type");const auto& token=ref->get();
                if(token.empty()||token.size()>8192)http_fail(SdkError::Kind::RequestValidation,requirement.source,"credential must be nonempty and at most 8192 bytes");
                switch(requirement.kind){
                    case CredentialKind::Bearer:if(!bearer(token))http_fail(SdkError::Kind::RequestValidation,requirement.source,"invalid RFC6750 bearer token");header("Authorization","Bearer "+token,requirement.source);break;
                    case CredentialKind::HeaderKey:header(requirement.wire_name,token,requirement.source);break;
                    case CredentialKind::QueryKey:if(whole_query||query_keys.contains(requirement.wire_name))http_fail(SdkError::Kind::RequestRepresentation,requirement.source,"API key conflicts with whole query or parameter");query_part(encode_component(requirement.wire_name,Encoding::Component,context,requirement.source,limit)+"="+encode_component(token,Encoding::Component,context,requirement.source,limit),requirement.source);break;
                    case CredentialKind::CookieKey:{if(!cookie_keys.insert(requirement.wire_name).second)http_fail(SdkError::Kind::RequestRepresentation,requirement.source,"API key conflicts with cookie parameter or credential");Serialization policy;policy.encoding=Encoding::None;data_guard(token,Location::Cookie,policy,requirement.source);if(!cookie.empty())append_bounded(cookie,"; ",settings.transfer.max_header_bytes,requirement.source,context);append_bounded(cookie,requirement.wire_name+"="+token,settings.transfer.max_header_bytes,requirement.source,context);break;}
                    default:break;
                }
            }
        }
    }else if(settings.security_alternative)http_fail(SdkError::Kind::Configuration,operation.source,"anonymous operation has no security alternative selector");
    if(!query.empty()){append_bounded(request.url,"?",limit,operation.source,context);append_bounded(request.url,query,limit,operation.source,context);}
    if(!cookie.empty())header("Cookie",std::move(cookie),operation.source);
    std::string accept;
    if(settings.response_media){(void)select_media(operation.accept,*settings.response_media,operation.source);accept=*settings.response_media;}
    else{for(const auto& media:operation.accept){if(!accept.empty())append_bounded(accept,", ",settings.transfer.max_header_bytes,operation.source,context);append_bounded(accept,media.declared,settings.transfer.max_header_bytes,operation.source,context);}if(accept.empty())accept="*/*";}
    header("Accept",std::move(accept),operation.source);
    if(body){if(body->bytes.size()>limit)http_fail(SdkError::Kind::ResourceLimit,operation.source,"request body exceeds byte ceiling");header("Content-Type",std::move(body->content_type),operation.source);request.body=std::move(body->bytes);}
    std::size_t header_bytes=0;for(const auto& [name,value]:request.headers){if(name.size()+value.size()+4>settings.transfer.max_header_bytes-header_bytes)http_fail(SdkError::Kind::ResourceLimit,operation.source,"request headers exceed byte ceiling");header_bytes+=name.size()+value.size()+4;}
    return request;
}
} // namespace @NAMESPACE@::detail
