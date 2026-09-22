#include "@PACKAGE@/stream.hpp"

namespace @NAMESPACE@::detail {
// HTML's UTF-8 decoder uses replacement, unlike the strict JSON byte parser.
static std::string replacement_utf8(std::string_view text,Context& context,const Source& source,std::size_t limit){
    std::string result;
    for(std::size_t at=0;at<text.size();){auto first=static_cast<unsigned char>(text[at]);
        if(first<128){append_bounded(result,text.substr(at++,1),limit,source,context);continue;}
        unsigned tails=first>=0xC2&&first<=0xDF?1:first>=0xE0&&first<=0xEF?2:first>=0xF0&&first<=0xF4?3:0;
        if(!tails){append_bounded(result,"\xEF\xBF\xBD",limit,source,context);++at;continue;}
        std::size_t end=at+1;bool valid=true;
        for(unsigned i=0;i<tails;++i){if(end==text.size()){valid=false;break;}auto next=static_cast<unsigned char>(text[end]);
            bool good=(next&0xC0)==0x80;
            if(i==0)good&=!(first==0xE0&&next<0xA0)&&!(first==0xED&&next>=0xA0)&&!(first==0xF0&&next<0x90)&&!(first==0xF4&&next>=0x90);
            if(!good){valid=false;break;}++end;
        }
        append_bounded(result,valid?text.substr(at,end-at):std::string_view("\xEF\xBF\xBD"),limit,source,context);
        at=end;
    }return result;
}
ItemState::ItemState(HttpExchange exchange,Settings settings,Operation operation,Source source,StreamFraming framing,std::size_t item_limit,std::unique_ptr<Context> ctx)
    :exchange_(std::move(exchange)),settings_(std::move(settings)),operation_(std::move(operation)),source_(std::move(source)),framing_(framing),item_limit_(std::min(item_limit,settings_.max_item_bytes)),polls_(settings_.transfer.max_response_bytes+4096),context(std::move(ctx)){
    response.status=exchange_.status;response.headers=exchange_.headers;response.content_type=media_type(response.headers).value_or("");
}
void ItemState::close() noexcept {if(exchange_.body)exchange_.body->close();exchange_.body.reset();chunk_.clear();line_.clear();data_.clear();closed_=true;}
SdkError ItemState::fail(CodecError error){if(error.source.document.empty())error.source=source_;auto result=codec_error(operation_,std::move(error),response);result.response->truncated=true;close();return result;}
Presence<JsonValue> ItemState::line_item(){
    auto bytes=std::move(line_);line_.clear();
    if(framing_==StreamFraming::JsonLines){if(!bytes.empty()&&bytes.back()=='\r')bytes.pop_back();frame_bytes_=0;return parse_document(bytes,*context,source_);}
    auto text=replacement_utf8(bytes,*context,source_,item_limit_);
    if(first_){first_=false;if(text.starts_with("\xEF\xBB\xBF"))text.erase(0,3);}
    if(text.empty()){
        frame_bytes_=0;
        if(!has_data_){data_.clear();event_.reset();retry_.reset();return std::nullopt;}
        JsonValue::Object object;data_.pop_back();object.emplace("data",JsonValue(std::move(data_)));data_.clear();has_data_=false;
        if(event_&&!event_->empty())object.emplace("event",JsonValue(std::move(*event_)));
        if(id_)object.emplace("id",JsonValue(*id_));
        if(retry_)object.emplace("retry",JsonValue(*retry_));
        event_.reset();retry_.reset();return JsonValue(std::move(object));
    }
    if(text.front()==':')return std::nullopt;
    std::string_view line=text;auto colon=line.find(':');auto field=line.substr(0,colon);auto value=colon==std::string_view::npos?std::string_view{}:line.substr(colon+1);if(value.starts_with(' '))value.remove_prefix(1);
    if(field=="data"){
        append_bounded(data_,value,item_limit_,source_,*context);append_bounded(data_,"\n",item_limit_,source_,*context);has_data_=true;
    }else if(field=="event")event_=context->text(value,source_,"");
    else if(field=="id"&&value.find('\0')==std::string_view::npos)id_=context->text(value,source_,"");
    else if(field=="retry"&&!value.empty()&&std::all_of(value.begin(),value.end(),[](char c){return c>='0'&&c<='9';})){
        while(value.size()>1&&value.front()=='0')value.remove_prefix(1);auto number=JsonNumber::parse(value);
        if(!number)throw Failure{std::move(number).error()};retry_=std::move(number).value();
    }
    return std::nullopt;
}
Result<Presence<JsonValue>,SdkError> ItemState::next(){
    using R=Result<Presence<JsonValue>,SdkError>;if(closed_)return R::success(std::nullopt);
    try{
        for(;;){
            context->control.check(source_);
            while(offset_<chunk_.size()){
                char byte=chunk_[offset_++];
                if(skip_lf_){skip_lf_=false;if(byte=='\n')continue;}
                if(frame_bytes_==item_limit_)http_fail(SdkError::Kind::ResourceLimit,source_,"stream frame exceeds item ceiling");++frame_bytes_;
                if(byte=='\n'||(framing_==StreamFraming::ServerSentEvents&&byte=='\r')){
                    skip_lf_=byte=='\r';auto value=line_item();if(value)return R::success(std::move(value));
                }else line_.push_back(byte);
            }
            chunk_.clear();offset_=0;if(!polls_--)http_fail(SdkError::Kind::ResourceLimit,source_,"stream polling budget exhausted");
            if(!exchange_.body){close();return R::success(std::nullopt);}
            auto next=[&]()->Result<Presence<std::string>,TransportError>{try{return exchange_.body->next();}catch(...){TransportError error;error.cause=std::current_exception();error.message="stream body threw";return Result<Presence<std::string>,TransportError>::failure(std::move(error));}}();
            if(!next){auto failure=std::move(next).error();if(failure.response)bound_metadata(*failure.response,settings_.transfer);auto error=transport_error(std::move(failure),source_);error.operation_source=operation_.source;error.operation_id=operation_.id;if(!error.response)error.response=response;error.response->truncated=true;close();return R::failure(std::move(error));}
            if(!next.value()){
                if(framing_==StreamFraming::JsonLines&&!line_.empty()){auto value=line_item();close();return R::success(std::move(value));}
                close();return R::success(std::nullopt);
            }
            auto bytes=std::move(*next.value());
            if(bytes.empty()){if(++empty_>1024)http_fail(SdkError::Kind::ResourceLimit,source_,"empty stream chunk budget exceeded");continue;}empty_=0;
            auto keep=std::min(bytes.size(),settings_.transfer.max_capture_bytes-response.body_capture.size());response.body_capture.append(bytes.data(),keep);
            if(bytes.size()>settings_.transfer.max_buffer_bytes||bytes.size()>settings_.transfer.max_response_bytes-total_)http_fail(SdkError::Kind::ResourceLimit,source_,"stream receive window or total byte ceiling exceeded");
            total_+=bytes.size();response.truncated=total_>response.body_capture.size();context->spend(bytes.size(),source_,"");chunk_=std::move(bytes);
        }
    }catch(Failure& failure){return R::failure(fail(std::move(failure.error)));}
    catch(HttpFailure& failure){failure.error.operation_source=operation_.source;failure.error.operation_id=operation_.id;failure.error.response=response;failure.error.response->truncated=true;close();return R::failure(std::move(failure.error));}
}
std::string encode_item(const JsonValue& json,StreamFraming framing,Context& context,const Source& source,std::size_t limit){
    std::string out;
    if(framing==StreamFraming::JsonLines){auto bytes=write_document(json,context,source);append_bounded(out,bytes,limit,source,context);append_bounded(out,"\n",limit,source,context);return out;}
    if(!json.is<JsonValue::Object>())http_fail(SdkError::Kind::RequestRepresentation,source,"SSE item must be an event envelope");
    const auto& object=json.as<JsonValue::Object>();for(const auto& [name,value]:object){(void)value;if(name!="data"&&name!="event"&&name!="id"&&name!="retry")http_fail(SdkError::Kind::RequestRepresentation,source,"SSE fields discarded by framing have no faithful request representation");}
    for(const auto* name:{"event","id","retry"})if(auto it=object.find(name);it!=object.end()){
        std::string value;
        if(std::string_view(name)=="retry"){if(!it->second.is<JsonNumber>())http_fail(SdkError::Kind::RequestRepresentation,source,"retry must be a nonnegative integer");value=integer_digits(it->second.as<JsonNumber>(),limit,source);}
        else value=scalar_text(it->second,Scalar::String,source);
        if(value.find_first_of("\r\n")!=std::string::npos||(std::string_view(name)=="id"&&value.find('\0')!=std::string::npos))http_fail(SdkError::Kind::RequestRepresentation,source,"SSE field contains a framing delimiter");
        append_bounded(out,name,limit,source,context);append_bounded(out,": ",limit,source,context);append_bounded(out,value,limit,source,context);append_bounded(out,"\n",limit,source,context);
    }
    auto it=object.find("data");if(it==object.end())http_fail(SdkError::Kind::RequestRepresentation,source,"SSE request item requires data for dispatch");auto data=scalar_text(it->second,Scalar::String,source);
    if(data.find('\r')!=std::string::npos)http_fail(SdkError::Kind::RequestRepresentation,source,"SSE data must use LF line endings");
    std::string_view rest=data;for(;;){auto lf=rest.find('\n');append_bounded(out,"data: ",limit,source,context);append_bounded(out,rest.substr(0,lf),limit,source,context);append_bounded(out,"\n",limit,source,context);if(lf==std::string_view::npos)break;rest.remove_prefix(lf+1);}
    append_bounded(out,"\n",limit,source,context);return out;
}
} // namespace @NAMESPACE@::detail
