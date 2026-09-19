#include <generated_sdk/sdk.hpp>
#include <cstdlib>
#include <iostream>
#include <mutex>
#include <deque>
#include <thread>
#include <filesystem>
using namespace generated_sdk;
#define CHECK(x) do { if (!(x)) { std::cerr << "failed line " << __LINE__ << ": " << #x << '\n'; std::abort(); } } while(false)
static constexpr auto widget_json = R"json({"amount":9007199254740993.000000000000000001,"id":"w1","meta":null,"payload":{"kind":"standard","text":"plain"},"child":{"label":"root","child":{"label":"leaf"}},"extra":{"n":1e999999999999999999999999}})json";
class Recording final : public Transport {
public:
    mutable std::mutex mutex;
    mutable std::vector<HttpRequest> requests;
    mutable std::deque<HttpResponse> responses;
    Result<HttpResponse, TransportError> send(const HttpRequest& request, const TransportOptions&) const override {
        std::lock_guard lock(mutex); requests.push_back(request);
        CHECK(!responses.empty()); auto response=std::move(responses.front()); responses.pop_front();
        return Result<HttpResponse,TransportError>::success(std::move(response));
    }
    void add(int status,std::string body,Headers headers={{"Content-Type","application/json"}}) {
        responses.push_back({status,std::move(headers),std::move(body)});
    }
};
static std::string header(const HttpRequest& request,std::string_view name) {
    for (const auto& [key,value]:request.headers) if(detail::lower_ascii(key)==name) return value;
    return "";
}
static void codecs() {
    auto parsed=WidgetCodec::decode(widget_json); CHECK(parsed);
    auto& widget=parsed.value(); CHECK(widget.amount.token()=="9007199254740993.000000000000000001");
    CHECK(widget.meta && std::holds_alternative<Null>(*widget.meta));
    CHECK(widget.payload.value.index()==0);
    CHECK(std::get<0>(widget.payload.value).text=="plain");
    CHECK(widget.child && widget.child->child && widget.child->child->value().label=="leaf");
    auto copy=widget; copy.child->child->value().label="changed";
    CHECK(widget.child->child->value().label=="leaf");
    auto encoded=WidgetCodec::encode(widget); CHECK(encoded);
    CHECK(encoded.value().find("9007199254740993.000000000000000001")!=std::string::npos);
    CHECK(encoded.value().find("1e999999999999999999999999")!=std::string::npos);
    widget.meta=std::nullopt; CHECK(WidgetCodec::encode(widget).value().find("\"meta\"")==std::string::npos);
    widget.meta=std::string("present"); CHECK(WidgetCodec::encode(widget).value().find("\"meta\":\"present\"")!=std::string::npos);
    WidgetInput input("alpha"); input.amount=JsonNumber::parse("1e-400").value();
    CHECK(WidgetInputCodec::encode(input).value()==R"({"amount":1e-400,"name":"alpha"})");
    input.name.clear(); auto invalid=WidgetInputCodec::encode(input); CHECK(!invalid);
    CHECK(invalid.error().kind==CodecError::Kind::Validation); CHECK(invalid.error().instance_path=="/name");
    CHECK(invalid.error().source.pointer=="/components/schemas/WidgetInput/properties/name/minLength");
    CHECK(invalid.error().source.end>invalid.error().source.begin);
    input.name="x"; input.extra.emplace("amount",JsonValue(Null{})); CHECK(!WidgetInputCodec::encode(input));
    input.extra.clear(); input.name=std::string("\xc0\xaf",2); invalid=WidgetInputCodec::encode(input);
    CHECK(!invalid && invalid.error().kind==CodecError::Kind::Model && invalid.error().instance_path=="/name");
    CHECK(!WidgetInputCodec::decode("{}")); CHECK(!WidgetInputCodec::decode(R"({"name":"x","amount":null})"));
    CHECK(StandardPayloadCodec::encode(StandardPayload("plain")).value()==R"({"kind":"standard","text":"plain"})");
    auto bad_tag=StandardPayload("plain"); bad_tag.kind=static_cast<StandardPayloadKind>(55);
    CHECK(!StandardPayloadCodec::encode(bad_tag));
    CHECK(!WidgetPayloadCodec::decode(R"({"kind":"other","text":"plain"})"));
    CHECK(!WidgetPayloadCodec::decode(R"({"kind":"secure","text":"plain"})"));
    auto node=WidgetNode("leaf"); node.child=Box<WidgetNode>(WidgetNode("child"));
    auto ownership=std::move(*node.child); CHECK(ownership.has_value());
    CHECK(!WidgetNodeCodec::encode(node));
    for (const auto* text:{"{\"x\":1,\"\\u0078\":2}","\"\\uD800\"","\"\\uDC00\"","1 2","01","NaN","[1,]","{\"x\":0,}"}) {
        auto json=parse_json(text); CHECK(!json && json.error().kind==CodecError::Kind::InvalidJson);
    }
    CHECK(!parse_json(std::string("\"\xc0\xaf\"",4)));
    auto unicode=parse_json(R"({"é":1,"e\u0301":2,"\ud83d\ude00":"😀"})");CHECK(unicode);
    CHECK(unicode.value().as<JsonValue::Object>().size()==3);
    CHECK(parse_json(write_json(unicode.value()).value()));
    CHECK(JsonInteger::parse("1.0").value().to_int64().value()==1);
    CHECK(JsonInteger::parse("1e3").value().to_int64().value()==1000);
    CHECK(JsonInteger::parse("-9223372036854775808").value().to_int64().value()==(-9223372036854775807LL-1));
    CHECK(!JsonInteger::parse("9223372036854775808").value().to_int64());
    CHECK(!JsonInteger::parse("1e100000000000000000000000").value().to_int64());
    CHECK(!JsonInteger::parse("1e-400"));
    JsonLimits tiny; tiny.max_work=3; CHECK(!parse_json("[1,2]",tiny));
    tiny=JsonLimits{}; tiny.max_depth=2; CHECK(!parse_json("[[[0]]]",tiny));
    std::string padded="1e+"+std::string(4000,'0')+"3";
    CHECK(JsonInteger::parse(padded).value().to_int64().value()==1000);
    CHECK(JsonNumber::parse("-0e999999999999999999").value()==JsonNumber(0));
}
static void operations() {
    auto transport=std::make_shared<Recording>();
    Credentials credentials; credentials.api_key="m2-key"; Client client(transport,credentials);
    transport->add(200,widget_json);
    WidgetInput body("alpha"); body.amount=JsonNumber::parse("9007199254740993.000000000000000001").value();
    auto created=client.create_widget(CreateWidgetInput(body)); CHECK(created);
    CHECK(std::get<CreateWidgetStatus200>(created.value()).data.id=="w1");
    CHECK(*transport->requests[0].body==R"({"amount":9007199254740993.000000000000000001,"name":"alpha"})");
    CHECK(transport->requests[0].method=="POST");
    CHECK(header(transport->requests[0],"authorization")=="Bearer m2-key");
    CHECK(header(transport->requests[0],"content-type")=="application/json");
    transport->add(200,widget_json);
    auto got=client.get_widget(GetWidgetInput("a/b 雪!'()*"));CHECK(got);
    CHECK(transport->requests.back().url=="https://m2.example.test/api/v1/widgets/a%2Fb%20%E9%9B%AA%21%27%28%29%2A");
    transport->add(200,widget_json);
    WidgetPatch patch; patch.amount=JsonNumber::parse("0.0000000000000000001").value();
    auto updated=client.update_widget(UpdateWidgetInput("w1",patch));CHECK(updated);
    CHECK(transport->requests.back().method=="PATCH"); CHECK(*transport->requests.back().body==R"({"amount":0.0000000000000000001})");
    transport->add(200,R"({"items":[]})");
    ListWidgetsInput list;list.tag="a/b +雪";list.tags=std::vector<std::string>{"one","a/b"};list.labels=std::vector<std::string>{"a,b","雪"};list.limit=JsonInteger(2);
    auto listed=client.list_widgets(list); CHECK(listed);
    CHECK(transport->requests.back().url=="https://m2.example.test/api/v1/widgets?tag=a%2Fb%20%2B%E9%9B%AA&tags=one&tags=a%2Fb&labels=a%2Cb,%E9%9B%AA&limit=2");
    transport->add(404,R"({"message":"missing"})");
    auto missing=client.get_widget(GetWidgetInput("missing"));CHECK(!missing);
    CHECK(std::get<GetWidgetStatus404>(missing.error()).data.message=="missing");
    body.name=""; auto rejected=client.create_widget(CreateWidgetInput(body)); CHECK(!rejected);
    CHECK(std::get<SdkError>(rejected.error()).kind==SdkError::Kind::RequestValidation);
    CHECK(transport->requests.size()==5);
    for (auto media:{"text/plain","application/json;broken","application/json;charset=utf-8;charset=ascii"}) {
        transport->add(200,widget_json,{{"Content-Type",media}});
        auto response=client.get_widget(GetWidgetInput("wrong"));CHECK(!response);
        CHECK(std::get<SdkError>(response.error()).kind==SdkError::Kind::UnexpectedResponse);
    }
    transport->add(200,"{broken");auto malformed=client.get_widget(GetWidgetInput("bad"));CHECK(!malformed);
    CHECK(std::get<SdkError>(malformed.error()).kind==SdkError::Kind::ResponseDecoding);
    transport->add(200,widget_json,{{"Content-Type","Application/JSON; charset=\"utf-8\""}});
    CHECK(client.get_widget(GetWidgetInput("good-media")));
    auto no_auth=Client(transport,Credentials{}).get_widget(GetWidgetInput("auth"));CHECK(!no_auth);
    CHECK(std::get<SdkError>(no_auth.error()).source.pointer=="/components/securitySchemes/apiKey");
    auto before=transport->requests.size();
    CallOptions tiny;tiny.max_request_bytes=64;list.tag=std::string(1000,'x');auto too_large=client.list_widgets(list,tiny);CHECK(!too_large);
    CHECK(std::get<SdkError>(too_large.error()).kind==SdkError::Kind::ResourceLimit);
    CHECK(transport->requests.size()==before);
    std::stop_token scoped;
    { Cancellation cancellation; scoped=cancellation.token(); }
    CHECK(scoped.stop_requested());
    CallOptions stopped;stopped.stop=scoped;auto cancelled=client.get_widget(GetWidgetInput("stop"),stopped);CHECK(!cancelled);
    CHECK(std::get<SdkError>(cancelled.error()).kind==SdkError::Kind::Cancelled);
    CHECK(transport->requests.size()==before);
}
template<class T,class E> const SdkError& sdk_error(const Result<T,E>& result,SdkError::Kind kind) {
    CHECK(!result);auto* error=std::get_if<SdkError>(&result.error());CHECK(error);
    if(error->kind!=kind)std::cerr<<"expected kind "<<static_cast<int>(kind)<<", received "<<static_cast<int>(error->kind)<<": "<<error->message<<" at "<<error->source.pointer<<(error->transport?" native="+std::to_string(error->transport->native_code):"")<<'\n';
    CHECK(error->kind==kind);
    CHECK(!error->operation_source.pointer.empty());CHECK(!error->operation_id.empty());return *error;
}
static bool wait_file(const std::filesystem::path& file) {
    for(int n=0;n<500;++n) {if(std::filesystem::exists(file))return true;std::this_thread::sleep_for(std::chrono::milliseconds(5));}
    return false;
}
static Client network_client(std::string base) {
    Credentials credentials;credentials.api_key="m2-key";
    ClientOptions options;options.server_url=std::move(base);
    auto client=Client::with_curl(std::move(credentials),options);
    if(!client)std::cerr<<client.error().message<<" ("<<client.error().native_code<<")\n";
    CHECK(client);return std::move(client).value();
}
static void wire(std::string base,const std::filesystem::path& directory) {
    auto client=network_client(base);
    auto response=client.get_widget(GetWidgetInput("wire-check"));CHECK(response);
    CHECK(std::get<GetWidgetStatus200>(response.value()).data.amount.token()=="9007199254740993.000000000000000001");
    WidgetInput body("alpha");body.amount=JsonNumber::parse("9007199254740993.000000000000000001").value();
    CHECK(client.create_widget(CreateWidgetInput(body)));
    WidgetPatch patch;patch.amount=JsonNumber::parse("0.0000000000000000001").value();
    CHECK(client.update_widget(UpdateWidgetInput("w1",patch)));
    ListWidgetsInput list;list.tag="a/b +雪";list.tags=std::vector<std::string>{"one","a/b"};list.labels=std::vector<std::string>{"a,b","雪"};list.limit=JsonInteger(2);
    CHECK(client.list_widgets(list));
    auto missing=client.get_widget(GetWidgetInput("missing"));CHECK(!missing);
    CHECK(std::get<GetWidgetStatus404>(missing.error()).data.message=="missing");
    for(auto path:{"wrong-media","duplicate-media","redirect"}) {
        auto error=client.get_widget(GetWidgetInput(path));const auto& sdk=sdk_error(error,SdkError::Kind::UnexpectedResponse);
        CHECK(sdk.response);CHECK(sdk.response->body_capture.size()<=32);
    }
    CHECK(client.get_widget(GetWidgetInput("cookie")));
    CHECK(client.get_widget(GetWidgetInput("cookie-check")));
    auto malformed=client.get_widget(GetWidgetInput("malformed"));CHECK(sdk_error(malformed,SdkError::Kind::ResponseDecoding).codec);
    auto declared=client.get_widget(GetWidgetInput("declared-large"));CHECK(!declared);
    const auto& api=std::get<GetWidgetStatus404>(declared.error());CHECK(api.data.message.size()==256);CHECK(api.response.body_capture.size()==32);CHECK(api.response.truncated);
    auto unknown=client.get_widget(GetWidgetInput("unknown"));const auto& unexpected=sdk_error(unknown,SdkError::Kind::UnexpectedResponse);
    CHECK(unexpected.response->body_capture.size()==32 && unexpected.response->truncated);
    CallOptions small_body;small_body.max_response_bytes=128;
    auto oversized=client.get_widget(GetWidgetInput("oversized"),small_body);const auto& size_error=sdk_error(oversized,SdkError::Kind::ResourceLimit);
    CHECK(size_error.response && size_error.response->truncated && size_error.response->body_capture.size()<=32);
    CallOptions small_headers;small_headers.max_header_bytes=256;
    auto big_header=client.get_widget(GetWidgetInput("header-large"),small_headers);sdk_error(big_header,SdkError::Kind::ResourceLimit);
    auto interim=client.get_widget(GetWidgetInput("interim"),small_headers);sdk_error(interim,SdkError::Kind::ResourceLimit);
    auto reset=client.get_widget(GetWidgetInput("reset"));sdk_error(reset,SdkError::Kind::Transport);
    auto replay=network_client(base+"/reset");
    auto post_reset=replay.create_widget(CreateWidgetInput(body));sdk_error(post_reset,SdkError::Kind::Transport);
    for(const auto* path:{"cancelled","body-cancel"}) {
        Cancellation cancellation;
        auto marker=directory/(std::string(path)+"-received");
        std::jthread stopping([&] {CHECK(wait_file(marker));cancellation.cancel();});
        CallOptions options;options.stop=cancellation.token();
        auto start=std::chrono::steady_clock::now();
        auto cancelled=client.get_widget(GetWidgetInput(path),options);const auto& cancel_error=sdk_error(cancelled,SdkError::Kind::Cancelled);
        if(std::string_view(path)=="body-cancel") CHECK(cancel_error.response && cancel_error.response->truncated && !cancel_error.response->body_capture.empty());
        CHECK(std::chrono::steady_clock::now()-start<std::chrono::seconds(2));
        CHECK(wait_file(directory/(std::string(path)+"-closed")));
    }
    CallOptions timeout;timeout.timeout=std::chrono::milliseconds(150);
    auto start=std::chrono::steady_clock::now();auto delayed=client.get_widget(GetWidgetInput("delay"),timeout);
    sdk_error(delayed,SdkError::Kind::Timeout);CHECK(std::chrono::steady_clock::now()-start<std::chrono::seconds(2));
    CHECK(wait_file(directory/"delay-closed"));
    // Last-client destruction releases the adapter; copies retain it during I/O.
    auto transport=CurlTransport::create();CHECK(transport);
    std::weak_ptr<CurlTransport> weak=transport.value();
    {
        Credentials credentials;credentials.api_key="m2-key";ClientOptions options;options.server_url=base;
        Client held(transport.value(),credentials,options);transport.value().reset();CHECK(!weak.expired());
        auto copied=held;CHECK(copied.get_widget(GetWidgetInput("lifetime")));
    }
    CHECK(weak.expired());
    std::cout<<"M2 real libcurl HTTP, limits, cancellation, timeout, cookies, proxies, redirects and lifetime passed\n";
}
static void tls(std::string base,std::string ca_file) {
    auto untrusted=network_client(base);auto rejected=untrusted.get_widget(GetWidgetInput("untrusted"));
    const auto& failure=sdk_error(rejected,SdkError::Kind::Transport);
    CHECK(failure.transport && failure.transport->kind==TransportError::Kind::Tls);
    Credentials credentials;credentials.api_key="m2-key";ClientOptions options;options.server_url=base;
    CurlOptions trust;trust.ca_bundle=std::move(ca_file);
    auto trusted=Client::with_curl(credentials,options,trust);CHECK(trusted);
    auto response=trusted.value().get_widget(GetWidgetInput("trusted"));
    if(!response)std::cerr<<std::get<SdkError>(response.error()).message<<'\n';
    CHECK(response);CHECK(std::get<GetWidgetStatus200>(response.value()).data.id=="tls");
    auto host=base.find("localhost");CHECK(host!=std::string::npos);base.replace(host,9,"127.0.0.1");options.server_url=base;
    auto wrong_host=Client::with_curl(credentials,options,trust);CHECK(wrong_host);
    auto mismatch=wrong_host.value().get_widget(GetWidgetInput("wrong-host"));
    CHECK(sdk_error(mismatch,SdkError::Kind::Transport).transport->kind==TransportError::Kind::Tls);
    std::cout<<"real TLS trust refusal, explicit CA trust and hostname validation passed\n";
}
int main(int argc,char** argv) {
    CHECK(argc>=2);
    auto mode=std::string_view(argv[1]);
    if(mode=="codecs") {codecs();operations();std::cout<<"M2 native value, operation, source, exact JSON and ownership checks passed\n";}
    else if(mode=="wire") {CHECK(argc==4);wire(argv[2],argv[3]);}
    else if(mode=="tls") {CHECK(argc==4);tls(argv[2],argv[3]);}
    else CHECK(false);
}
