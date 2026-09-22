#include <generated_sdk/sdk.hpp>
#include <cstdlib>
#include <filesystem>
#include <iostream>
#include <thread>
using namespace generated_sdk;
__ADVANCED_ALIASES__
#define CHECK(x) do {if(!(x)){std::cerr<<__LINE__<<" "<<#x;std::abort();}}while(false)
static bool closed(const std::filesystem::path& path){for(unsigned i=0;i<400;++i){if(std::filesystem::exists(path))return true;std::this_thread::sleep_for(std::chrono::milliseconds(5));}return false;}
static void tls_failure(const Result<EventsSuccess,EventsError>& result){CHECK(!result);const auto& error=std::get<SdkError>(result.error());CHECK(error.kind==SdkError::Kind::Transport&&error.transport->kind==TransportError::Kind::Tls);}
int main(int argc,char** argv){
    CHECK(argc==3);std::string port=argv[1];std::filesystem::path root=argv[2];
    ClientOptions options;options.server_url="https://localhost:"+port;options.max_capture_bytes=16;
    auto untrusted=Client::with_curl({},options);CHECK(untrusted);tls_failure(untrusted.value().events(EventsInput("untrusted")));
    CurlOptions curl;curl.ca_bundle=(root/"server.crt").string();
    options.server_url="https://127.0.0.1:"+port;auto wrong_name=Client::with_curl({},options,curl);CHECK(wrong_name);tls_failure(wrong_name.value().events(EventsInput("wrong-name")));
    options.server_url="https://localhost:"+port;
    auto response=[&]{auto ready=Client::with_curl({},options,curl);CHECK(ready);return ready.value().events(EventsInput("lease"));}();CHECK(response);
    auto& stream=std::get<EventsStatus200>(response.value()).data;CHECK(stream.next().value()->data=="verified TLS");stream.close();CHECK(closed(root/"lease-closed"));
    auto ready=Client::with_curl({},options,curl);CHECK(ready);auto& client=ready.value();
    Cancellation cancellation;CallOptions stop;stop.stop=cancellation.token();auto cancel=client.events(EventsInput("cancel"),stop);CHECK(cancel);cancellation.cancel();
    CHECK(closed(root/"cancel-closed"));auto cancelled=std::get<EventsStatus200>(cancel.value()).data.next();CHECK(!cancelled&&cancelled.error().kind==SdkError::Kind::Cancelled);
    CallOptions limit;limit.timeout=std::chrono::milliseconds(180);auto timeout=client.events(EventsInput("timeout"),limit);CHECK(timeout);
    auto item=std::get<EventsStatus200>(timeout.value()).data.next();CHECK(!item&&item.error().kind==SdkError::Kind::Timeout);CHECK(closed(root/"timeout-closed"));
    CHECK(client.cookie(CookieInput(Colors("green","red"))));
    CHECK(client.array_cookie(ArrayCookieInput(std::vector<std::string>{"one","two"})));
    auto invalid=client.array_cookie(ArrayCookieInput(std::vector<std::string>{"has;separator"}));CHECK(!invalid&&std::get<SdkError>(invalid.error()).kind==SdkError::Kind::RequestRepresentation);
    Line first(JsonInteger::parse("1.0").value()),second(JsonInteger::parse("1e100000000000000000000").value());
    CHECK(client.emit_lines(EmitLinesInput(std::vector<Line>{first,second})));
    TestExtraForm form("known");form.extra.emplace("count",JsonInteger::parse("9007199254740993").value());
    auto form_response=client.extra_form(ExtraFormInput(form));CHECK(form_response);CHECK(std::get<ExtraFormStatus200>(form_response.value()).data.extra.at("count").token()=="9007199254740993");
    TestExtraMultipart multipart(TestKnownPart("known"));multipart.extra.emplace("雪",TestExtraPart(JsonInteger(2)));
    auto multipart_response=client.extra_multipart(ExtraMultipartInput(multipart));CHECK(multipart_response);CHECK(std::get<ExtraMultipartStatus200>(multipart_response.value()).data.extra.at("雪").data.token()=="2");
    multipart.extra.emplace("name",TestExtraPart(JsonInteger(3)));auto collision=client.extra_multipart(ExtraMultipartInput(multipart));CHECK(!collision&&std::get<SdkError>(collision.error()).kind==SdkError::Kind::RequestRepresentation);
    std::cout<<"verified TLS stream leases/cancellation/deadlines, standard cookies, JSON-line requests and typed aggregate extras passed\n";
}
