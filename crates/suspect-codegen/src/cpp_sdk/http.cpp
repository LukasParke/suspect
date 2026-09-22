#include "@PACKAGE@/protocol.hpp"

namespace @NAMESPACE@ {
namespace {
class MemoryBody final : public ResponseBody {
    std::string bytes_;
    Headers headers_;
    TransportOptions options_;
    std::size_t offset_ = 0;
public:
    MemoryBody(std::string bytes, Headers headers, TransportOptions options)
        : bytes_(std::move(bytes)), headers_(std::move(headers)), options_(std::move(options)) {}
    Result<Presence<std::string>,TransportError> next() override {
        using R=Result<Presence<std::string>,TransportError>;
        if(options_.stop.stop_requested() || std::chrono::steady_clock::now()>=options_.deadline){
            TransportError error;error.kind=options_.stop.stop_requested()?TransportError::Kind::Cancelled:TransportError::Kind::Timeout;
            error.message="response body interrupted";close();return R::failure(std::move(error));
        }
        if(offset_==bytes_.size()){close();return R::success(std::nullopt);}
        auto size=std::min(options_.max_buffer_bytes,bytes_.size()-offset_);
        auto chunk=bytes_.substr(offset_,size);offset_+=size;return R::success(std::move(chunk));
    }
    void close() noexcept override {bytes_.clear();offset_=0;}
    Headers final_headers() const override {return headers_;}
};
}
Result<HttpExchange,TransportError> Transport::open(const HttpRequest& request,const TransportOptions& options) const {
    auto response=send(request,options);
    if(!response)return Result<HttpExchange,TransportError>::failure(std::move(response).error());
    auto value=std::move(response).value();
    if(value.body.size()>options.max_response_bytes || (detail::forbidden_body(request.method,value.status)&&!value.body.empty())){
        TransportError error;error.kind=detail::forbidden_body(request.method,value.status)?TransportError::Kind::Protocol:TransportError::Kind::ResourceLimit;
        error.message="complete transport body violates its HTTP/size policy";
        error.response=detail::metadata(value,options.max_capture_bytes,true);
        return Result<HttpExchange,TransportError>::failure(std::move(error));
    }
    HttpExchange result;result.status=value.status;result.headers=value.headers;
    result.body=std::make_unique<MemoryBody>(std::move(value.body),std::move(value.headers),options);
    return Result<HttpExchange,TransportError>::success(std::move(result));
}
namespace detail {
[[noreturn]] void http_fail(SdkError::Kind kind,const Source& source,std::string message){
    SdkError error;error.kind=kind;error.source=source;error.message=std::move(message);throw HttpFailure{std::move(error)};
}
Settings settings(const ClientOptions& client,const CallOptions& call,const Source& source){
    Settings result;
    auto limit=[&](std::size_t maximum,std::size_t wide,Presence<std::size_t> local,bool zero=false){
        auto value=local.value_or(wide);if(wide>maximum||value>wide||(!zero&&(!wide||!value)))http_fail(SdkError::Kind::Configuration,source,"caller limit exceeds its enclosing finite ceiling");return value;
    };
    result.max_request_bytes=limit(@REQUEST_BYTES@,client.max_request_bytes,call.max_request_bytes);
    result.transfer.max_response_bytes=limit(@RESPONSE_BYTES@,client.max_response_bytes,call.max_response_bytes);
    result.transfer.max_header_bytes=limit(@HEADER_BYTES@,client.max_header_bytes,call.max_header_bytes);
    result.transfer.max_capture_bytes=std::min(result.transfer.max_response_bytes,limit(@CAPTURE_BYTES@,client.max_capture_bytes,call.max_capture_bytes,true));
    result.transfer.max_buffer_bytes=limit(@STREAM_BUFFER_BYTES@,client.max_stream_buffer_bytes,call.max_stream_buffer_bytes);
    if(result.transfer.max_buffer_bytes<16384)http_fail(SdkError::Kind::Configuration,source,"receive window must hold a libcurl write chunk (16384 bytes)");
    result.max_item_bytes=limit(@STREAM_ITEM_BYTES@,client.max_stream_item_bytes,call.max_stream_item_bytes);
    result.max_parts=limit(@PARTS@,client.max_parts,call.max_parts);
    result.max_part_bytes=limit(@PART_BYTES@,client.max_part_bytes,call.max_part_bytes,true);
    auto timeout=call.timeout.value_or(client.timeout);
    if(client.timeout.count()<=0||client.timeout.count()>86'400'000||timeout.count()<=0||timeout>client.timeout)
        http_fail(SdkError::Kind::Configuration,source,"timeout must be positive, at most a day and no larger than client timeout");
    result.transfer.stop=call.stop;result.transfer.deadline=std::chrono::steady_clock::now()+timeout;
    result.control().check(source);
    if(client.server_url && client.server_url->size()>result.max_request_bytes)http_fail(SdkError::Kind::ResourceLimit,source,"server URL exceeds request ceiling");
    if(client.document_url && client.document_url->size()>result.max_request_bytes)http_fail(SdkError::Kind::ResourceLimit,source,"document URL exceeds request ceiling");
    result.server_url=client.server_url;result.document_url=client.document_url;result.server_index=call.server_index.value_or(client.server_index);
    const auto& variables=call.server_variables?*call.server_variables:client.server_variables;
    std::size_t bytes=0;for(const auto& [key,value]:variables){if(key.size()>result.max_request_bytes-bytes)http_fail(SdkError::Kind::ResourceLimit,source,"server variable names exceed request ceiling");bytes+=key.size();if(value.size()>result.max_request_bytes-bytes)http_fail(SdkError::Kind::ResourceLimit,source,"server variables exceed request ceiling");bytes+=value.size();}
    result.variables=variables;result.security_alternative=call.security_alternative?call.security_alternative:client.security_alternative;
    result.user_agent=client.user_agent;result.application_id=client.application_id;
    result.response_media=call.response_media;return result;
}
std::string lower_ascii(std::string_view text){std::string value(text);for(char& c:value)if(c>='A'&&c<='Z')c=static_cast<char>(c-'A'+'a');return value;}
bool header_name(std::string_view value){
    if(value.empty())return false;
    for(unsigned char c:value)if(!((c>='0'&&c<='9')||(c>='a'&&c<='z')||(c>='A'&&c<='Z'))&&std::string_view("!#$%&'*+-.^_`|~").find(static_cast<char>(c))==std::string_view::npos)return false;
    return true;
}
bool header_value(std::string_view value){for(unsigned char c:value)if((c<32&&c!='\t')||c==127)return false;return true;}
static std::string_view trim(std::string_view value){while(!value.empty()&&(value.front()==' '||value.front()=='\t'))value.remove_prefix(1);while(!value.empty()&&(value.back()==' '||value.back()=='\t'))value.remove_suffix(1);return value;}
Presence<Media> parse_media(std::string_view text,bool ranges){
    text=trim(text);if(!header_value(text))return std::nullopt;std::size_t at=0;
    auto token=[&]()->Presence<std::string>{auto start=at;while(at<text.size()&&header_name(text.substr(at,1)))++at;if(start==at)return std::nullopt;return std::string(text.substr(start,at-start));};
    auto type=token();if(!type||at==text.size()||text[at++]!='/')return std::nullopt;auto subtype=token();if(!subtype)return std::nullopt;
    Media result;result.declared=std::string(text);result.type=lower_ascii(*type);result.subtype=lower_ascii(*subtype);
    if(result.type.find('*')!=std::string::npos||result.subtype.find('*')!=std::string::npos){
        if(!ranges||result.subtype!="*"||(result.type.find('*')!=std::string::npos&&result.type!="*"))return std::nullopt;
    }
    for(;;){
        while(at<text.size()&&(text[at]==' '||text[at]=='\t'))++at;
        if(at==text.size())return result;if(text[at++]!=';')return std::nullopt;
        while(at<text.size()&&(text[at]==' '||text[at]=='\t'))++at;
        auto key=token();if(!key||at==text.size()||text[at++]!='=')return std::nullopt;
        std::string value;
        if(at<text.size()&&text[at]=='"'){
            ++at;bool closed=false;
            while(at<text.size()){char c=text[at++];if(c=='"'){closed=true;break;}if(c=='\\'){if(at==text.size())return std::nullopt;c=text[at++];}value.push_back(c);}
            if(!closed||!header_value(value))return std::nullopt;
        }else{auto v=token();if(!v)return std::nullopt;value=std::move(*v);}
        if(!result.parameters.emplace(lower_ascii(*key),std::move(value)).second)return std::nullopt;
    }
}
Presence<std::string> content_type(const Headers& headers){
    Presence<std::string> result;for(const auto& [key,value]:headers)if(lower_ascii(key)=="content-type"){if(result)return std::nullopt;result=value;}return result;
}
Presence<std::string> media_type(const Headers& headers){auto text=content_type(headers);if(!text)return std::nullopt;auto media=parse_media(*text);if(!media)return std::nullopt;return media->type+"/"+media->subtype;}
bool matches_media(const Media& declared,const Media& actual){
    if(declared.type!="*"&&declared.type!=actual.type)return false;if(declared.subtype!="*"&&declared.subtype!=actual.subtype)return false;
    for(const auto& [name,value]:declared.parameters){auto at=actual.parameters.find(name);if(at==actual.parameters.end())return false;if(name=="charset"?lower_ascii(value)!=lower_ascii(at->second):value!=at->second)return false;}return true;
}
std::size_t select_media(const std::vector<Media>& media,std::string_view actual,const Source& source){
    auto parsed=parse_media(actual);if(!parsed)http_fail(SdkError::Kind::RequestRepresentation,source,"invalid concrete Content-Type");
    Presence<std::size_t> result;std::pair<unsigned,std::size_t> rank{};
    for(std::size_t i=0;i<media.size();++i)if(matches_media(media[i],*parsed)){
        auto next=std::pair{media[i].type=="*"?0u:media[i].subtype=="*"?1u:2u,media[i].parameters.size()};
        if(!result||next>rank){result=i;rank=next;}
    }
    if(!result)http_fail(SdkError::Kind::RequestRepresentation,source,"Content-Type is not declared");
    if(media[*result].utf8){auto charset=parsed->parameters.find("charset");if(charset!=parsed->parameters.end()&&lower_ascii(charset->second)!="utf-8")http_fail(SdkError::Kind::RequestRepresentation,source,"representation requires UTF-8 charset");}
    return *result;
}
bool forbidden_body(std::string_view method,int status){return method=="HEAD"||(status>=100&&status<200)||status==204||status==205||status==304;}
ResponseMetadata metadata(const HttpResponse& response,std::size_t capture,bool interrupted){
    ResponseMetadata result;result.status=response.status;result.headers=response.headers;result.content_type=media_type(response.headers).value_or("");
    result.body_capture=response.body.substr(0,capture);result.truncated=interrupted||result.body_capture.size()!=response.body.size();return result;
}
SdkError transport_error(TransportError error,const Source& source){
    SdkError result;result.kind=SdkError::Kind::Transport;result.source=source;result.message="HTTP transport failed";
    switch(error.kind){case TransportError::Kind::Cancelled:result.kind=SdkError::Kind::Cancelled;break;case TransportError::Kind::Timeout:result.kind=SdkError::Kind::Timeout;break;case TransportError::Kind::ResourceLimit:result.kind=SdkError::Kind::ResourceLimit;break;case TransportError::Kind::Configuration:result.kind=SdkError::Kind::Configuration;break;default:break;}
    result.response=error.response;result.cause=error.cause;result.transport=std::move(error);return result;
}
void bound_metadata(ResponseMetadata& value,const TransportOptions& options){
    if(value.body_capture.size()>options.max_capture_bytes){value.body_capture.resize(options.max_capture_bytes);value.truncated=true;}
    std::size_t count=0,bytes=0;
    for(const auto& [key,v]:value.headers){if(count==256||key.size()>options.max_header_bytes-bytes)break;bytes+=key.size();if(v.size()>options.max_header_bytes-bytes)break;bytes+=v.size();++count;}
    value.headers.resize(count);
    if(value.content_type.size()>options.max_header_bytes)value.content_type.resize(options.max_header_bytes);
}
Result<HttpExchange,SdkError> open_exchange(const std::shared_ptr<const Transport>& transport,const HttpRequest& request,const Settings& settings,const Source& source){
    using R=Result<HttpExchange,SdkError>;
    if(!transport)http_fail(SdkError::Kind::Configuration,source,"client transport is absent");settings.control().check(source);
    auto opened=[&]()->Result<HttpExchange,TransportError>{try{return transport->open(request,settings.transfer);}catch(...){TransportError error;error.message="injected transport threw";error.cause=std::current_exception();return Result<HttpExchange,TransportError>::failure(std::move(error));}}();
    if(!opened){auto error=std::move(opened).error();if(error.response)bound_metadata(*error.response,settings.transfer);return R::failure(transport_error(std::move(error),source));}
    auto result=std::move(opened).value();
    auto fail=[&](SdkError::Kind kind,const char* message){SdkError error;error.kind=kind;error.source=source;error.message=message;ResponseMetadata retained;retained.status=result.status;retained.headers=std::move(result.headers);bound_metadata(retained,settings.transfer);error.response=std::move(retained);if(result.body)result.body->close();return R::failure(std::move(error));};
    if(settings.transfer.stop.stop_requested())return fail(SdkError::Kind::Cancelled,"operation cancelled");
    if(std::chrono::steady_clock::now()>=settings.transfer.deadline)return fail(SdkError::Kind::Timeout,"operation deadline exceeded");
    if(result.status<200||result.status>599)return fail(SdkError::Kind::Transport,"invalid final HTTP status");
    if(result.headers.size()>256)return fail(SdkError::Kind::ResourceLimit,"response header count exceeded");
    std::size_t bytes=0;for(const auto& [key,value]:result.headers){
        if(key.size()>settings.transfer.max_header_bytes-bytes)return fail(SdkError::Kind::ResourceLimit,"response headers exceed ceiling");bytes+=key.size();
        if(value.size()>settings.transfer.max_header_bytes-bytes)return fail(SdkError::Kind::ResourceLimit,"response headers exceed ceiling");bytes+=value.size();
        if(4>settings.transfer.max_header_bytes-bytes)return fail(SdkError::Kind::ResourceLimit,"response headers exceed ceiling");bytes+=4;
        if(!header_name(key)||!header_value(value))return fail(SdkError::Kind::Transport,"malformed response header");
    }
    if(forbidden_body(request.method,result.status)){if(result.body)result.body->close();result.body.reset();}
    return R::success(std::move(result));
}
Result<HttpResponse,SdkError> collect(HttpExchange response,const Settings& settings,const Source& source){
    using R=Result<HttpResponse,SdkError>;HttpResponse result{response.status,std::move(response.headers),{}};
    std::size_t polls=settings.transfer.max_response_bytes+4096,empty=0;
    while(response.body){
        auto failure=[&](SdkError::Kind kind,const char* message){SdkError error;error.kind=kind;error.source=source;error.message=message;error.response=metadata(result,settings.transfer.max_capture_bytes,true);return R::failure(std::move(error));};
        if(settings.transfer.stop.stop_requested())return failure(SdkError::Kind::Cancelled,"operation cancelled");
        if(std::chrono::steady_clock::now()>=settings.transfer.deadline)return failure(SdkError::Kind::Timeout,"operation deadline exceeded");
        if(!polls--)return failure(SdkError::Kind::ResourceLimit,"response polling budget exceeded");
        auto next=[&]()->Result<Presence<std::string>,TransportError>{try{return response.body->next();}catch(...){TransportError error;error.cause=std::current_exception();error.message="body reader threw";return Result<Presence<std::string>,TransportError>::failure(std::move(error));}}();
        if(!next){auto failure=std::move(next).error();if(failure.response)bound_metadata(*failure.response,settings.transfer);auto error=transport_error(std::move(failure),source);if(!error.response)error.response=metadata(result,settings.transfer.max_capture_bytes,true);return R::failure(std::move(error));}
        if(!next.value())break;
        auto& chunk=*next.value();if(chunk.empty()){if(++empty>1024)return failure(SdkError::Kind::ResourceLimit,"too many empty response chunks");continue;}empty=0;
        if(chunk.size()>settings.transfer.max_buffer_bytes||chunk.size()>settings.transfer.max_response_bytes-result.body.size()){
            auto keep=std::min(chunk.size(),settings.transfer.max_capture_bytes>result.body.size()?settings.transfer.max_capture_bytes-result.body.size():0);
            result.body.append(chunk.data(),keep);return failure(SdkError::Kind::ResourceLimit,"response chunk/body ceiling exceeded");
        }
        result.body+=chunk;
    }
    if(response.body){
        Headers headers;
        try{headers=response.body->final_headers();}catch(...){auto error=transport_error(TransportError{},source);error.cause=std::current_exception();error.response=metadata(result,settings.transfer.max_capture_bytes,true);return R::failure(std::move(error));}
        std::size_t bytes=0;bool valid=headers.size()<=256;
        for(const auto& [name,value]:headers){
            if(name.size()>settings.transfer.max_header_bytes-bytes){valid=false;break;}bytes+=name.size();
            if(value.size()>settings.transfer.max_header_bytes-bytes){valid=false;break;}bytes+=value.size();
            if(4>settings.transfer.max_header_bytes-bytes){valid=false;break;}bytes+=4;
            if(!header_name(name)||!header_value(value)){valid=false;break;}
        }
        if(!valid){SdkError error;error.kind=SdkError::Kind::ResourceLimit;error.source=source;error.message="final response headers violate bounded transport policy";error.response=metadata(result,settings.transfer.max_capture_bytes,true);return R::failure(std::move(error));}
        if(!headers.empty())result.headers=std::move(headers);
    }
    return R::success(std::move(result));
}
Result<HttpResponse,SdkError> exchange(const std::shared_ptr<const Transport>& transport,const HttpRequest& request,const Settings& settings,const Source& source){auto opened=open_exchange(transport,request,settings,source);if(!opened)return Result<HttpResponse,SdkError>::failure(std::move(opened).error());return collect(std::move(opened).value(),settings,source);}
SdkError codec_error(const Operation& operation,CodecError error,Presence<ResponseMetadata> response){
    SdkError result;result.kind=response?SdkError::Kind::ResponseDecoding:SdkError::Kind::RequestRepresentation;
    if(!response&&error.kind==CodecError::Kind::Validation)result.kind=SdkError::Kind::RequestValidation;
    if(error.kind==CodecError::Kind::ResourceLimit||error.kind==CodecError::Kind::EvaluationFailure)result.kind=SdkError::Kind::ResourceLimit;
    if(error.kind==CodecError::Kind::Cancelled)result.kind=SdkError::Kind::Cancelled;
    if(error.kind==CodecError::Kind::Timeout)result.kind=SdkError::Kind::Timeout;
    result.source=error.source;result.operation_source=operation.source;result.operation_id=operation.id;result.message=response?"response codec failed":"request codec failed";
    result.codec=std::move(error);result.response=std::move(response);return result;
}
} // namespace detail
} // namespace @NAMESPACE@
