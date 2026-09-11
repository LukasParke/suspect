#include <openrouter/sdk.hpp>
#include <cstdlib>
#include <iostream>
using namespace openrouter;
#define CHECK(x) do{if(!(x)){std::cerr<<__LINE__<<" "<<#x;std::abort();}}while(false)
class Capture final:public Transport {
public:
    mutable unsigned calls=0;
    mutable std::string authorization;
    Result<HttpResponse,TransportError> send(const HttpRequest& request,const TransportOptions& options)const override{
        CHECK(request.method=="GET");CHECK(!request.body);CHECK(!options.stop.stop_requested());CHECK(std::chrono::steady_clock::now()<options.deadline);
        CHECK(request.url=="https://openrouter.ai/api/v1/key"||request.url=="https://openrouter.ai/api/v1/credits");
        ++calls;authorization.clear();for(const auto& [name,value]:request.headers)if(name=="Authorization")authorization=value;
        return Result<HttpResponse,TransportError>::success(HttpResponse{200,{{"Content-Type","application/json"}},request.url.ends_with("/key")?__KEY_RESPONSE__:__CREDITS_RESPONSE__});
    }
};
static void set(const char* value){CHECK(setenv("OPENROUTER_API_KEY",value,1)==0);}
static void accepted(Client& client,const std::shared_ptr<Capture>& transport,const char* token){auto result=client.get_current_key();CHECK(result);CHECK(std::get<GetCurrentKeyStatus200>(result.value()).response.status==200);CHECK(!std::get<GetCurrentKeyStatus200>(result.value()).data.data.label.empty());CHECK(transport->authorization==std::string("Bearer ")+token);}
static void missing(Client& client,const std::shared_ptr<Capture>& transport){const auto before=transport->calls;auto result=client.get_current_key();CHECK(!result);const auto& error=std::get<SdkError>(result.error());CHECK(error.kind==SdkError::Kind::RequestValidation);CHECK(error.source.pointer=="/components/securitySchemes/apiKey");CHECK(error.message.size()<1024);CHECK(error.message.find("env-token")==std::string::npos);CHECK(!error.response);CHECK(transport->calls==before);}
int main(){
    auto transport=std::make_shared<Capture>();
    set("env-token-A");auto first=Client::from_env_with_transport(transport);set("env-token-B");accepted(first,transport,"env-token-A");
    auto second=Client::from_env_with_transport(transport);accepted(second,transport,"env-token-B");
    auto copied=first;auto moved=std::move(copied);CHECK(unsetenv("OPENROUTER_API_KEY")==0);accepted(moved,transport,"env-token-A");
    auto absent=Client::from_env_with_transport(transport);missing(absent,transport);
    set("");auto empty_environment=Client::from_env_with_transport(transport);missing(empty_environment,transport);
    set("env-token-present");Credentials explicit_value;explicit_value.api_key="explicit-token";Client explicit_client(transport,explicit_value);accepted(explicit_client,transport,"explicit-token");
    explicit_value.api_key="";Client explicit_empty(transport,explicit_value);missing(explicit_empty,transport);
    explicit_value.api_key=std::nullopt;Client explicit_null(transport,explicit_value);missing(explicit_null,transport);
    Client explicit_missing_member(transport,Credentials{});missing(explicit_missing_member,transport);
    Client ordinary_constructor(transport);missing(ordinary_constructor,transport);
    std::string oversized(8193,'x');set(oversized.c_str());auto too_large=Client::from_env_with_transport(transport);missing(too_large,transport);
    set("invalid\r\nheader");auto invalid=Client::from_env_with_transport(transport);missing(invalid,transport);
    set("env-token-C");auto credits=Client::from_env_with_transport(transport);auto balance=credits.get_credits();CHECK(balance);CHECK(std::get<GetCreditsStatus200>(balance.value()).response.status==200);CHECK(transport->authorization=="Bearer env-token-C");
    Cancellation cancel;cancel.cancel();CallOptions options;options.stop=cancel.token();const auto before=transport->calls;auto stopped=first.get_current_key(GetCurrentKeyInput{},options);CHECK(!stopped&&std::get<SdkError>(stopped.error()).kind==SdkError::Kind::Cancelled);CHECK(transport->calls==before);
#if defined(openrouter_HAS_CURL)
    // The live helper is compiled/created, but no real account request is made.
    auto native=Client::from_env();CHECK(native);auto native_stop=native.value().get_current_key(GetCurrentKeyInput{},options);CHECK(!native_stop&&std::get<SdkError>(native_stop.error()).kind==SdkError::Kind::Cancelled);
    CHECK(unsetenv("OPENROUTER_API_KEY")==0);auto native_missing=Client::from_env();CHECK(native_missing);auto rejected=native_missing.value().get_current_key();CHECK(!rejected&&std::get<SdkError>(rejected.error()).kind==SdkError::Kind::RequestValidation);
    CurlOptions bad_curl;bad_curl.connect_timeout=std::chrono::milliseconds(0);auto configuration=Client::from_env(ClientOptions{},bad_curl);CHECK(!configuration&&configuration.error().kind==TransportError::Kind::Configuration);
#endif
    std::cout<<"openrouter::Client::from_env / from_env_with_transport: construction snapshots, explicit precedence, bounded missing values, source-default HTTPS GET /key and GET /credits passed (controlled transport)\n";
}
