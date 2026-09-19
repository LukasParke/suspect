#include "@PACKAGE@/protocol.hpp"
#include <curl/curl.h>
#include <atomic>
#include <charconv>
#include <limits>
#include <mutex>

namespace @NAMESPACE@ {
namespace {
std::mutex& global_mutex(){static std::mutex mutex;return mutex;}
TransportError fault(TransportError::Kind kind,const char* message,int code=0){TransportError error;error.kind=kind;error.message=message;error.native_code=code;return error;}
TransportError::Kind curl_kind(CURLcode code){switch(code){case CURLE_OPERATION_TIMEDOUT:return TransportError::Kind::Timeout;case CURLE_PEER_FAILED_VERIFICATION:case CURLE_SSL_CONNECT_ERROR:case CURLE_SSL_CERTPROBLEM:case CURLE_SSL_CACERT_BADFILE:return TransportError::Kind::Tls;case CURLE_UNSUPPORTED_PROTOCOL:case CURLE_URL_MALFORMAT:return TransportError::Kind::Configuration;default:return TransportError::Kind::Network;}}
}
struct CurlTransport::State {
    CurlOptions options;
    bool initialized=false;
    explicit State(CurlOptions value):options(std::move(value)){}
    ~State(){if(initialized){std::lock_guard lock(global_mutex());curl_global_cleanup();}}
};
struct CurlTransport::Transfer final : ResponseBody {
    std::shared_ptr<const State> state;
    HttpRequest request;
    TransportOptions options;
    std::unique_ptr<CURL,decltype(&curl_easy_cleanup)> easy{nullptr,curl_easy_cleanup};
    std::unique_ptr<CURLM,decltype(&curl_multi_cleanup)> multi{nullptr,curl_multi_cleanup};
    std::unique_ptr<curl_slist,decltype(&curl_slist_free_all)> native_headers{nullptr,curl_slist_free_all};
    HttpResponse response;
    std::string buffer,capture;
    std::size_t received=0,header_bytes=0,header_fields=0;
    bool attached=false,headers_ready=false,paused=false,finished=false,closed=false,error_delivered=false;
    Presence<TransportError> failure;
    std::atomic<bool> cancelled{false};
    mutable std::mutex mutex;
    std::unique_ptr<std::stop_callback<std::function<void()>>> on_stop;
    Transfer(std::shared_ptr<const State> owner,HttpRequest input,TransportOptions policy):state(std::move(owner)),request(std::move(input)),options(std::move(policy)){}
    ~Transfer() override {on_stop.reset();close();}
    bool active() noexcept {
        if(failure)return false;
        if(cancelled.load(std::memory_order_relaxed)||options.stop.stop_requested()){failure=fault(TransportError::Kind::Cancelled,"exchange cancelled");return false;}
        if(std::chrono::steady_clock::now()>=options.deadline){failure=fault(TransportError::Kind::Timeout,"exchange deadline exceeded");return false;}
        return !closed;
    }
    void release() noexcept {
        if(attached){curl_multi_remove_handle(multi.get(),easy.get());attached=false;}
        easy.reset();multi.reset();native_headers.reset();closed=true;
    }
    void close() noexcept override {std::lock_guard lock(mutex);release();buffer.clear();}
    void cancel() noexcept {
        cancelled.store(true,std::memory_order_relaxed);
        // A waiting read observes the flag within one bounded poll. Taking the
        // lock afterwards also closes the transfer if cancellation races with
        // the last read, so no further next() call is needed to release I/O.
        std::lock_guard lock(mutex);
        if(!finished&&!failure)failure=fault(TransportError::Kind::Cancelled,"exchange cancelled");release();buffer.clear();
    }
    TransportError error() {
        auto error=failure.value_or(fault(TransportError::Kind::Network,"libcurl exchange failed"));
        if(response.status){auto meta=detail::metadata(response,0,true);meta.body_capture=capture;error.response=std::move(meta);}return error;
    }
    static std::size_t write(char* bytes,std::size_t size,std::size_t count,void* opaque) noexcept {
        auto& self=*static_cast<Transfer*>(opaque);
        try {
            if(!self.active())return 0;
            if(size&&count>std::numeric_limits<std::size_t>::max()/size){self.failure=fault(TransportError::Kind::ResourceLimit,"chunk size overflow");return 0;}
            auto length=size*count;
            if(length&&detail::forbidden_body(self.request.method,self.response.status)){self.failure=fault(TransportError::Kind::Protocol,"HTTP status/method forbids response content");return 0;}
            if(length>self.options.max_response_bytes-self.received){
                auto take=std::min(length,self.options.max_capture_bytes-self.capture.size());self.capture.append(bytes,take);
                self.failure=fault(TransportError::Kind::ResourceLimit,"response body exceeds byte ceiling");return 0;
            }
            if(length>self.options.max_buffer_bytes-self.buffer.size()){self.paused=true;return CURL_WRITEFUNC_PAUSE;}
            self.received+=length;self.capture.append(bytes,std::min(length,self.options.max_capture_bytes-self.capture.size()));
            self.buffer.append(bytes,length);return length;
        }catch(...){self.failure=fault(TransportError::Kind::Network,"body callback failed");self.failure->cause=std::current_exception();return 0;}
    }
    static std::size_t header(char* bytes,std::size_t size,std::size_t count,void* opaque) noexcept {
        auto& self=*static_cast<Transfer*>(opaque);
        try {
            if(!self.active())return 0;
            if(size&&count>std::numeric_limits<std::size_t>::max()/size){self.failure=fault(TransportError::Kind::ResourceLimit,"header size overflow");return 0;}
            auto length=size*count;if(length>self.options.max_header_bytes-self.header_bytes){self.failure=fault(TransportError::Kind::ResourceLimit,"cumulative response headers exceed ceiling");return 0;}self.header_bytes+=length;
            std::string_view line(bytes,length);if(!line.ends_with("\r\n")){self.failure=fault(TransportError::Kind::Protocol,"malformed response header line");return 0;}line.remove_suffix(2);
            if(line.starts_with("HTTP/")){
                auto at=line.find(' ');if(at==std::string_view::npos||at+4>line.size()){self.failure=fault(TransportError::Kind::Protocol,"malformed HTTP status");return 0;}
                int status=0;auto parsed=std::from_chars(line.data()+at+1,line.data()+at+4,status);
                if(parsed.ec!=std::errc{}||parsed.ptr!=line.data()+at+4||status<100||status>599){self.failure=fault(TransportError::Kind::Protocol,"invalid HTTP status");return 0;}
                self.response.status=status;self.response.headers.clear();self.headers_ready=false;return length;
            }
            if(line.empty()){if(self.response.status>=200)self.headers_ready=true;return length;}
            if(++self.header_fields>256){self.failure=fault(TransportError::Kind::ResourceLimit,"cumulative header count exceeded");return 0;}
            auto colon=line.find(':');if(colon==std::string_view::npos){self.failure=fault(TransportError::Kind::Protocol,"header colon missing");return 0;}
            auto name=line.substr(0,colon),value=line.substr(colon+1);while(!value.empty()&&(value.front()==' '||value.front()=='\t'))value.remove_prefix(1);while(!value.empty()&&(value.back()==' '||value.back()=='\t'))value.remove_suffix(1);
            if(!detail::header_name(name)||!detail::header_value(value)){self.failure=fault(TransportError::Kind::Protocol,"invalid response header");return 0;}
            if(self.headers_ready){auto key=detail::lower_ascii(name);if(key=="content-type"||key=="content-encoding"||key=="content-length"||key=="transfer-encoding"||key=="host"||key=="trailer"){self.failure=fault(TransportError::Kind::Protocol,"trailer cannot change response representation/framing");return 0;}}
            self.response.headers.emplace_back(name,value);
            if(detail::lower_ascii(name)=="content-length"&&!detail::forbidden_body(self.request.method,self.response.status)){
                std::size_t expected=0;auto parsed=std::from_chars(value.data(),value.data()+value.size(),expected);
                if(parsed.ec==std::errc::result_out_of_range||(parsed.ec==std::errc{}&&expected>self.options.max_response_bytes)){self.failure=fault(TransportError::Kind::ResourceLimit,"declared response length exceeds ceiling");return 0;}
                if(parsed.ec!=std::errc{}||parsed.ptr!=value.data()+value.size()){self.failure=fault(TransportError::Kind::Protocol,"invalid content length");return 0;}
            }
            return length;
        }catch(...){self.failure=fault(TransportError::Kind::Network,"header callback failed");self.failure->cause=std::current_exception();return 0;}
    }
    static int progress(void* opaque,curl_off_t,curl_off_t,curl_off_t,curl_off_t) noexcept {return static_cast<Transfer*>(opaque)->active()?0:1;}
    bool start() {
        if(!active())return false;
        easy.reset(curl_easy_init());multi.reset(curl_multi_init());
        if(!easy||!multi){failure=fault(TransportError::Kind::Network,"libcurl handle allocation failed");return false;}
        for(const auto& [key,value]:request.headers){if(!detail::header_name(key)||!detail::header_value(value)){failure=fault(TransportError::Kind::Configuration,"invalid prepared header");return false;}auto* next=curl_slist_append(native_headers.get(),(key+": "+value).c_str());if(!next){failure=fault(TransportError::Kind::Network,"header allocation failed");return false;}native_headers.release();native_headers.reset(next);}
        auto* next=curl_slist_append(native_headers.get(),"Expect:");if(!next){failure=fault(TransportError::Kind::Network,"header allocation failed");return false;}native_headers.release();native_headers.reset(next);
        CURLcode configured=CURLE_OK;auto set=[&](CURLoption option,auto value){auto code=curl_easy_setopt(easy.get(),option,value);if(configured==CURLE_OK)configured=code;};
        auto remaining=std::clamp<long long>(std::chrono::duration_cast<std::chrono::milliseconds>(options.deadline-std::chrono::steady_clock::now()).count(),1,86'400'000);
        set(CURLOPT_URL,request.url.c_str());set(CURLOPT_PROTOCOLS_STR,"http,https");set(CURLOPT_REDIR_PROTOCOLS_STR,"https");
        set(CURLOPT_PATH_AS_IS,1L);set(CURLOPT_HTTP_VERSION,static_cast<long>(CURL_HTTP_VERSION_1_1));
        set(CURLOPT_FRESH_CONNECT,1L);set(CURLOPT_FORBID_REUSE,1L);set(CURLOPT_FOLLOWLOCATION,0L);set(CURLOPT_MAXREDIRS,0L);
        set(CURLOPT_UNRESTRICTED_AUTH,0L);set(CURLOPT_AUTOREFERER,0L);set(CURLOPT_PROXY,"");set(CURLOPT_NOPROXY,"*");
        set(CURLOPT_NETRC,static_cast<long>(CURL_NETRC_IGNORED));set(CURLOPT_HTTPAUTH,static_cast<long>(CURLAUTH_NONE));set(CURLOPT_PROXYAUTH,static_cast<long>(CURLAUTH_NONE));
        set(CURLOPT_COOKIEFILE,static_cast<const char*>(nullptr));set(CURLOPT_COOKIEJAR,static_cast<const char*>(nullptr));
        set(CURLOPT_ACCEPT_ENCODING,static_cast<const char*>(nullptr));set(CURLOPT_HTTP_CONTENT_DECODING,0L);
        set(CURLOPT_SSL_VERIFYPEER,1L);set(CURLOPT_SSL_VERIFYHOST,2L);set(CURLOPT_SSLVERSION,static_cast<long>(CURL_SSLVERSION_TLSv1_2));
        if(state->options.ca_bundle)set(CURLOPT_CAINFO,state->options.ca_bundle->c_str());if(state->options.ca_directory)set(CURLOPT_CAPATH,state->options.ca_directory->c_str());
        set(CURLOPT_NOSIGNAL,1L);set(CURLOPT_TIMEOUT_MS,static_cast<long>(remaining));set(CURLOPT_CONNECTTIMEOUT_MS,static_cast<long>(std::min<long long>(remaining,state->options.connect_timeout.count())));
        set(CURLOPT_HTTPHEADER,native_headers.get());
        if(request.body){set(CURLOPT_POSTFIELDS,request.body->data());set(CURLOPT_POSTFIELDSIZE_LARGE,static_cast<curl_off_t>(request.body->size()));}
        if(request.method=="HEAD")set(CURLOPT_NOBODY,1L);
        set(CURLOPT_CUSTOMREQUEST,request.method.c_str());set(CURLOPT_WRITEFUNCTION,write);set(CURLOPT_WRITEDATA,this);
        set(CURLOPT_HEADERFUNCTION,header);set(CURLOPT_HEADERDATA,this);set(CURLOPT_NOPROGRESS,0L);set(CURLOPT_XFERINFOFUNCTION,progress);set(CURLOPT_XFERINFODATA,this);
        if(configured!=CURLE_OK){failure=fault(TransportError::Kind::Configuration,"libcurl option setup failed",static_cast<int>(configured));return false;}
        auto added=curl_multi_add_handle(multi.get(),easy.get());if(added!=CURLM_OK){failure=fault(TransportError::Kind::Network,"libcurl attach failed",static_cast<int>(added));return false;}attached=true;
        return active();
    }
    bool pump(bool head) {
        while(active()&&!finished&&(head?!headers_ready:buffer.empty())){
            if(paused){paused=false;auto code=curl_easy_pause(easy.get(),CURLPAUSE_CONT);if(code!=CURLE_OK){failure=fault(curl_kind(code),"libcurl resume failed",static_cast<int>(code));break;}}
            int running=0;auto code=curl_multi_perform(multi.get(),&running);if(code!=CURLM_OK){failure=fault(TransportError::Kind::Network,"libcurl perform failed",static_cast<int>(code));break;}
            if(!running){
                finished=true;CURLcode complete=CURLE_FAILED_INIT;int pending=0;while(auto* message=curl_multi_info_read(multi.get(),&pending))if(message->msg==CURLMSG_DONE&&message->easy_handle==easy.get())complete=message->data.result;
                if(complete!=CURLE_OK&&!failure)failure=fault(curl_kind(complete),"libcurl transfer failed",static_cast<int>(complete));
                char* effective=nullptr;if(!failure&&(curl_easy_getinfo(easy.get(),CURLINFO_EFFECTIVE_URL,&effective)!=CURLE_OK||!effective||request.url!=effective))failure=fault(TransportError::Kind::Protocol,"effective URL differs from prepared URL");
                release();break;
            }
            if((head&&headers_ready)||(!head&&!buffer.empty()))break;
            code=curl_multi_poll(multi.get(),nullptr,0,25,nullptr);if(code!=CURLM_OK){failure=fault(TransportError::Kind::Network,"libcurl poll failed",static_cast<int>(code));break;}
        }
        if(failure)release();return !failure;
    }
    Result<Presence<std::string>,TransportError> next() override {
        using R=Result<Presence<std::string>,TransportError>;std::lock_guard lock(mutex);
        if(cancelled.load(std::memory_order_relaxed)&&!failure)failure=fault(TransportError::Kind::Cancelled,"exchange cancelled");
        if(!failure)(void)active();
        if(!failure&&!buffer.empty()){auto bytes=std::move(buffer);buffer.clear();return R::success(std::move(bytes));}
        if(!failure&&!finished&&!closed)pump(false);
        if(failure){if(error_delivered)return R::success(std::nullopt);error_delivered=true;auto failed=error();release();buffer.clear();return R::failure(std::move(failed));}
        if(!buffer.empty()){auto bytes=std::move(buffer);buffer.clear();return R::success(std::move(bytes));}
        return R::success(std::nullopt);
    }
    Headers final_headers() const override {std::lock_guard lock(mutex);return response.headers;}
};
Result<std::shared_ptr<CurlTransport>,TransportError> CurlTransport::create(CurlOptions options){
    using R=Result<std::shared_ptr<CurlTransport>,TransportError>;
    if(options.connect_timeout.count()<=0||options.connect_timeout.count()>86'400'000)return R::failure(fault(TransportError::Kind::Configuration,"invalid connect timeout"));
    for(const auto* path:{&options.ca_bundle,&options.ca_directory})if(*path&&((*path)->empty()||(*path)->size()>8192||(*path)->find('\0')!=std::string::npos))return R::failure(fault(TransportError::Kind::Configuration,"invalid explicit CA path"));
    auto state=std::make_shared<State>(std::move(options));
    {std::lock_guard lock(global_mutex());auto code=curl_global_init(CURL_GLOBAL_DEFAULT);if(code!=CURLE_OK)return R::failure(fault(TransportError::Kind::Configuration,"libcurl global init failed",static_cast<int>(code)));state->initialized=true;
        const auto* version=curl_version_info(CURLVERSION_NOW);if(!version||version->version_num<0x075500||!(version->features&CURL_VERSION_SSL)||!(version->features&CURL_VERSION_ASYNCHDNS)||!(version->features&CURL_VERSION_THREADSAFE)){state->initialized=false;curl_global_cleanup();return R::failure(fault(TransportError::Kind::Configuration,"libcurl >=7.85 with TLS, async DNS and thread-safe init is required"));}}
    return R::success(std::shared_ptr<CurlTransport>(new CurlTransport(std::move(state))));
}
Result<HttpExchange,TransportError> CurlTransport::open(const HttpRequest& request,const TransportOptions& options) const {
    using R=Result<HttpExchange,TransportError>;
    if(!state_||!options.max_response_bytes||options.max_response_bytes>@RESPONSE_BYTES@||!options.max_header_bytes||options.max_header_bytes>@HEADER_BYTES@||options.max_capture_bytes>@CAPTURE_BYTES@||options.max_buffer_bytes<16384||options.max_buffer_bytes>@STREAM_BUFFER_BYTES@)
        return R::failure(fault(TransportError::Kind::Configuration,"invalid transport resource policy"));
    if(request.url.size()>@REQUEST_BYTES@||(request.body&&request.body->size()>@REQUEST_BYTES@)||request.url.find('\0')!=std::string::npos||!detail::header_name(request.method))return R::failure(fault(TransportError::Kind::Configuration,"invalid prepared request"));
    auto transfer=std::make_unique<Transfer>(state_,request,options);
    // Registration can invoke the callback immediately for a stopped token.
    // Do it before acquiring the transfer lock; try_lock on a mutex already
    // owned by this thread is not a portable synchronization operation.
    transfer->on_stop=std::make_unique<std::stop_callback<std::function<void()>>>(options.stop,[value=transfer.get()]{value->cancel();});
    {std::lock_guard lock(transfer->mutex);if(!transfer->start()||!transfer->pump(true)){auto error=transfer->error();transfer->release();return R::failure(std::move(error));}}
    HttpExchange result;result.status=transfer->response.status;result.headers=transfer->response.headers;result.body=std::move(transfer);return R::success(std::move(result));
}
Result<HttpResponse,TransportError> CurlTransport::send(const HttpRequest& request,const TransportOptions& options) const {
    auto opened=open(request,options);if(!opened)return Result<HttpResponse,TransportError>::failure(std::move(opened).error());
    detail::Settings settings;settings.transfer=options;auto response=detail::collect(std::move(opened).value(),settings,{});
    if(!response){auto sdk=std::move(response).error();auto error=sdk.transport.value_or(fault(sdk.kind==SdkError::Kind::ResourceLimit?TransportError::Kind::ResourceLimit:sdk.kind==SdkError::Kind::Cancelled?TransportError::Kind::Cancelled:sdk.kind==SdkError::Kind::Timeout?TransportError::Kind::Timeout:TransportError::Kind::Network,"HTTP response collection failed"));error.response=std::move(sdk.response);return Result<HttpResponse,TransportError>::failure(std::move(error));}
    return Result<HttpResponse,TransportError>::success(std::move(response).value());
}
} // namespace @NAMESPACE@
