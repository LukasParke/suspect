// Included by the installed rich consumer after its source-allocated aliases.
#include <stdexcept>
struct Script {
    int status=200;
    Headers headers;
    std::vector<std::string> chunks;
    Presence<TransportError> failure;
    Headers final;
    bool throws=false,closed=false;
    std::size_t opens=0,polls=0,closes=0,index=0;
    HttpRequest request;
};
class ScriptBody final:public ResponseBody {
    std::shared_ptr<Script> script_;
public:
    explicit ScriptBody(std::shared_ptr<Script> script):script_(std::move(script)){}
    Result<Presence<std::string>,TransportError> next() override {
        ++script_->polls;
        if(script_->throws)throw std::runtime_error("reader exception witness");
        if(script_->failure)return Result<Presence<std::string>,TransportError>::failure(*script_->failure);
        if(script_->index==script_->chunks.size())return Result<Presence<std::string>,TransportError>::success(std::nullopt);
        return Result<Presence<std::string>,TransportError>::success(script_->chunks[script_->index++]);
    }
    void close() noexcept override {if(!script_->closed){script_->closed=true;++script_->closes;}}
    Headers final_headers() const override {return script_->final;}
    // HttpExchange must close on every exit, even when an injected reader's
    // destructor relies on its owner to perform that release.
};
class ScriptTransport final:public Transport {
    std::shared_ptr<Script> script_;
public:
    explicit ScriptTransport(std::shared_ptr<Script> script):script_(std::move(script)){}
    Result<HttpResponse,TransportError> send(const HttpRequest&,const TransportOptions&) const override {
        CHECK(false);return Result<HttpResponse,TransportError>::failure(TransportError{});
    }
    Result<HttpExchange,TransportError> open(const HttpRequest& request,const TransportOptions&) const override {
        ++script_->opens;script_->request=request;
        return Result<HttpExchange,TransportError>::success(HttpExchange(script_->status,script_->headers,std::make_unique<ScriptBody>(script_)));
    }
};
static std::shared_ptr<Script> script(std::string media,std::string bytes={}){
    auto result=std::make_shared<Script>();result->headers={{"Content-Type",std::move(media)}};
    if(!bytes.empty())result->chunks.push_back(std::move(bytes));return result;
}
static Client scripted(const std::shared_ptr<Script>& script,ClientOptions options={},Credentials credentials={}){
    if(!options.server_url&&!options.document_url)options.server_url="https://fixture.invalid/base";
    options.max_capture_bytes=16;
    return Client(std::make_shared<ScriptTransport>(script),std::move(credentials),std::move(options));
}
static TestUpload upload_value(){
    TestFilePart file(Bytes{1},TestPartHeaders("part-1"));file.content_type="image/png";
    return TestUpload(std::move(file),TestMetadataPart(TestMetadata("native")));
}
static void edge_cases(){
    {
        std::string bytes="\xef\xbb\xbf: comment\revent: discarded\rid: 7\r\rdata\r\rdata: x\ndata: ";
        bytes.push_back(static_cast<char>(0xff));bytes+="\nid: bad";bytes.push_back('\0');
        bytes+="id\nretry: +1\nunknown: ignored\n\nid\n\nevent: tick\ndata: [DONE]\nretry: 0005\n\ndata: pending\n";
        auto input=script("text/event-stream");for(char c:bytes)input->chunks.emplace_back(1,c);
        auto response=scripted(input).events(EventsInput("chunks"));CHECK(response&&input->polls==0);
        auto& stream=std::get<EventsStatus200>(response.value()).data;
        auto first=stream.next();CHECK(first&&first.value()&&first.value()->data.empty());CHECK(first.value()->id=="7"&&!first.value()->event);
        auto second=stream.next();CHECK(second&&second.value());CHECK(second.value()->data=="x\n\xef\xbf\xbd");CHECK(second.value()->id=="7"&&!second.value()->retry);
        auto third=stream.next();CHECK(third&&third.value());CHECK(third.value()->data=="[DONE]"&&third.value()->event=="tick");CHECK(third.value()->id==""&&third.value()->retry->token()=="5");
        CHECK(!stream.next().value());CHECK(stream.is_closed()&&input->closes==1);
    }
    {
        auto input=script("text/event-stream","data: first\n\ndata: second\n\n");Cancellation cancellation;CallOptions options;options.stop=cancellation.token();
        auto response=scripted(input).events(EventsInput("cached"),options);CHECK(response);
        auto& source=std::get<EventsStatus200>(response.value()).data;auto moved=std::move(source);CHECK(source.is_closed());
        CHECK(moved.next().value()->data=="first");cancellation.cancel();
        auto next=moved.next();CHECK(!next&&next.error().kind==SdkError::Kind::Cancelled);CHECK(input->polls==1&&input->closes==1);
    }
    {
        auto input=script("text/event-stream","data: first\n\n");
        {auto response=scripted(input).events(EventsInput("scope"));CHECK(response);}
        CHECK(input->polls==0&&input->closes==1);
    }
    {
        auto input=script("text/event-stream");input->throws=true;
        auto response=scripted(input).events(EventsInput("throw"));CHECK(response);unsigned items=0;
        for(const auto& item:std::get<EventsStatus200>(response.value()).data){++items;CHECK(!item&&item.error().kind==SdkError::Kind::Transport);CHECK(item.error().cause);CHECK(item.error().response->status==200);}
        CHECK(items==1&&input->polls==1&&input->closes==1);
    }
    {
        auto input=script("text/event-stream");TransportError failure;failure.message="bounded retained error";
        failure.response=ResponseMetadata{};failure.response->status=200;failure.response->body_capture=std::string(100000,'x');
        failure.response->headers=Headers(300,{"X-Failure",std::string(1000,'x')});input->failure=std::move(failure);
        auto response=scripted(input).events(EventsInput("failure"));CHECK(response);auto next=std::get<EventsStatus200>(response.value()).data.next();
        CHECK(!next&&next.error().response->body_capture.size()==16&&next.error().response->truncated);
        CHECK(next.error().transport->response->body_capture.size()==16);CHECK(next.error().response->headers.size()<256);CHECK(input->closes==1);
    }
    for(bool window:{false,true}){
        auto input=script("text/event-stream",std::string(window?32769:129,'x'));CallOptions options;options.max_response_bytes=window?100000:128;options.max_stream_buffer_bytes=16384;
        auto response=scripted(input).events(EventsInput("ceiling"),options);CHECK(response);auto next=std::get<EventsStatus200>(response.value()).data.next();
        CHECK(!next&&next.error().kind==SdkError::Kind::ResourceLimit);CHECK(next.error().operation_id=="events"&&!next.error().source.pointer.empty());
        CHECK(next.error().response->body_capture.size()==16&&input->polls==1&&input->closes==1);
    }
    {
        auto input=script("text/event-stream");input->chunks=std::vector<std::string>(1025);
        auto response=scripted(input).events(EventsInput("empty-chunks"));CHECK(response);auto next=std::get<EventsStatus200>(response.value()).data.next();
        CHECK(!next&&next.error().kind==SdkError::Kind::ResourceLimit);CHECK(input->polls==1025&&input->closes==1);
    }
    {
        auto input=script("text/event-stream",": "+std::string(40,static_cast<char>(0xff))+"\n\n");CallOptions options;options.max_stream_item_bytes=64;
        auto response=scripted(input).events(EventsInput("replacement-ceiling"),options);CHECK(response);auto next=std::get<EventsStatus200>(response.value()).data.next();
        CHECK(!next&&next.error().kind==SdkError::Kind::ResourceLimit&&input->closes==1);
    }
    for(const auto& bytes:std::vector<std::string>{"\n","null\n","{\"n\":1,\"n\":2}\n",std::string("{\"n\":\"")+static_cast<char>(0xff)+"\"}\n"}){
        auto input=script("application/x-ndjson",bytes);auto response=scripted(input).lines(LinesInput("invalid"));CHECK(response);
        auto next=std::get<LinesStatus200>(response.value()).data.next();CHECK(!next&&next.error().kind==SdkError::Kind::ResponseDecoding);CHECK(input->closes==1);
    }
    {
        auto input=script("application/json","{\"message\":\"unused\"}");
        error(scripted(input).headers(),SdkError::Kind::ResponseDecoding);CHECK(input->polls==0&&input->closes==1);
    }
    {
        auto input=script("text/plain","must not be read");input->status=205;
        auto response=scripted(input).precedence(PrecedenceInput("205"));CHECK(response);CHECK(std::get<TestRangeNone>(response.value()).response.status==205);
        CHECK(input->polls==0&&input->closes==1);
    }
    {
        auto input=script("application/json","{\"message\":\"headers\"}");input->headers.insert(input->headers.end(),{{"X-Rate","1.0"},{"X-Flags","a,b"},{"x-flags","c"}});
        auto response=scripted(input).headers();CHECK(response);CHECK(*std::get<HeadersStatus200>(response.value()).headers.x_flags==std::vector<std::string>({"a","b","c"}));CHECK(input->closes==1);
    }
    for(const auto& bytes:std::vector<std::string>{"x--b--\r\n","--b\r\nContent-Disposition: form-data; name=\"file\"\r\nContent-Type: image/png\r\n\r\nx\r\n--b--not-a-terminator\r\n","--b\r\nContent-Transfer-Encoding: base64\r\nContent-Disposition: form-data; name=\"file\"\r\n\r\neA==\r\n--b--\r\n"}){
        auto input=script("multipart/form-data; boundary=b",bytes);
        error(scripted(input).upload(UploadInput(upload_value())),SdkError::Kind::ResponseDecoding);CHECK(input->closes==1);
    }
    {
        auto file_data=std::string("\0\377",2)+"\r\n--b b--not-a-terminator";
        auto bytes=std::string("preamble\r\n--b b \t\r\nContent-Disposition: form-data; name=\"file\"\r\nContent-Type: image/png\r\nX-Part-Id: part-1\r\n\r\n")+file_data+
            "\r\n--b b\r\nContent-Disposition: form-data; name=\"metadata\"\r\nContent-Type: application/json\r\n\r\n{\"title\":\"native\"}\r\n--b b--\r\nepilogue";
        auto input=script("multipart/form-data; boundary=\"b b\"",bytes);auto response=scripted(input).upload(UploadInput(upload_value()));CHECK(response);
        const auto& data=std::get<TestUploadResponse>(response.value()).data;CHECK(std::string(data.file.data.begin(),data.file.data.end())==file_data);CHECK(input->closes==1);
    }
    {
        auto input=script("text/plain","ok");CallOptions options;options.max_parts=1;
        error(scripted(input).upload(UploadInput(upload_value()),options),SdkError::Kind::ResourceLimit);CHECK(input->opens==0);
        TestForm form("x");form.tags=std::vector<std::string>{"a","b"};
        error(scripted(input).form(FormInput(form),options),SdkError::Kind::ResourceLimit);CHECK(input->opens==0);
    }
    {
        auto input=script("text/plain","ok");ClientOptions options;options.document_url="http://Example.TEST:00080/a/spec.json?revision=1";options.server_index=1;
        CHECK(scripted(input,options).servers());CHECK(input->request.url=="http://example.test/api/servers");
        auto encoded=script("text/plain","ok");options.server_url="http://example.test/a/%2e%2e/%E9%9B%AA//keep/";
        CHECK(scripted(encoded,options).public_call());CHECK(encoded->request.url=="http://example.test/a/%2e%2e/%E9%9B%AA//keep/public");
    }
    {
        auto input=script("text/plain","unused");Credentials credentials;credentials.oauth=CredentialProvider([](const CredentialRequest&)->Result<Authorization,TransportError>{throw std::runtime_error("provider failed");});
        CallOptions options;options.security_alternative=3;auto response=scripted(input,{},credentials).secure(SecureInput{},options);
        CHECK(error(response,SdkError::Kind::Transport).cause);CHECK(input->opens==0);
    }
    std::cout<<"C++ split-framing, metadata, MIME, source-control and owning resource edge cases passed\n";
}
