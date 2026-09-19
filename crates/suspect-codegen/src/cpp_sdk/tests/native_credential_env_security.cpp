#include <env_sdk/sdk.hpp>
#include <cstdlib>
#include <iostream>
using namespace env_sdk;
#define CHECK(x) do{if(!(x)){std::cerr<<__LINE__<<" "<<#x;std::abort();}}while(false)
class Capture final:public Transport {
public:
    mutable unsigned calls=0;
    mutable HttpRequest request;
    Result<HttpResponse,TransportError> send(const HttpRequest& value,const TransportOptions&)const override{
        CHECK(value.method=="GET");CHECK(value.url.starts_with("https://env-fixture.invalid/api/"));++calls;request=value;
        return Result<HttpResponse,TransportError>::success(HttpResponse{204,{},{}});
    }
    std::string header(const char* name)const {for(const auto& [key,value]:request.headers)if(key==name)return value;return {};}
};
static void set(const char* name,const char* value){CHECK(setenv(name,value,1)==0);}
template<class R>static void unavailable(const R& result,const Capture& transport,unsigned before){CHECK(!result);const auto& error=std::get<SdkError>(result.error());CHECK(error.kind==SdkError::Kind::RequestValidation);CHECK(error.message.size()<1024);CHECK(error.message.find("fixture-")==std::string::npos);CHECK(!error.response);CHECK(transport.calls==before);}
int main(){
    for(const char* name:{"CPP_ENV_BEARER","CPP_ENV_HEADER","CPP_ENV_QUERY","CPP_ENV_COOKIE","CPP_ENV_ALIAS_ONE","CPP_ENV_ALIAS_TWO"})CHECK(unsetenv(name)==0);
    auto transport=std::make_shared<Capture>();auto missing=Client::from_env_with_transport(transport);CHECK(missing.public_call());CHECK(transport->header("Authorization").empty());
    auto before=transport->calls;unavailable(missing.protected_call(),*transport,before);CHECK(missing.optional());CHECK(transport->header("Authorization").empty());
    set("CPP_ENV_HEADER","fixture-header");auto header=Client::from_env_with_transport(transport);CHECK(header.either());CHECK(transport->header("X-Key")=="fixture-header");CHECK(transport->header("Authorization").empty());
    set("CPP_ENV_BEARER","not a bearer token");auto unusable=Client::from_env_with_transport(transport);CHECK(unusable.either());CHECK(transport->header("X-Key")=="fixture-header");
    set("CPP_ENV_BEARER","fixture-bearer");set("CPP_ENV_QUERY","A /+=");set("CPP_ENV_COOKIE","cookie%20value");auto all=Client::from_env_with_transport(transport);
    CHECK(all.together());CHECK(transport->header("Authorization")=="Bearer fixture-bearer");CHECK(transport->header("X-Key")=="fixture-header");CHECK(transport->header("Cookie")=="session=cookie%20value");CHECK(transport->request.url=="https://env-fixture.invalid/api/together?api_key=A%20%2F%2B%3D");
    CHECK(all.optional());CHECK(transport->header("Authorization").empty());CallOptions select;select.security_alternative=1;CHECK(all.optional(OptionalInput{},select));CHECK(transport->header("Authorization")=="Bearer fixture-bearer");
    Credentials explicit_partial;explicit_partial.bearer="fixture-explicit";Client partial(transport,explicit_partial);CHECK(partial.protected_call());CHECK(transport->header("Authorization")=="Bearer fixture-explicit");before=transport->calls;unavailable(partial.together(),*transport,before);
    explicit_partial.bearer="";Client empty(transport,explicit_partial);before=transport->calls;unavailable(empty.protected_call(),*transport,before);
    explicit_partial.bearer=std::nullopt;Client null_member(transport,explicit_partial);before=transport->calls;unavailable(null_member.protected_call(),*transport,before);
    Client empty_argument(transport,Credentials{});CHECK(empty_argument.public_call());before=transport->calls;unavailable(empty_argument.either(),*transport,before);
    set("CPP_ENV_BEARER","");set("CPP_ENV_HEADER","");auto empty_env=Client::from_env_with_transport(transport);CHECK(empty_env.public_call());before=transport->calls;unavailable(empty_env.either(),*transport,before);
    CHECK(all.protected_call());CHECK(transport->header("Authorization")=="Bearer fixture-bearer");
    set("CPP_ENV_ALIAS_ONE","fixture-one");set("CPP_ENV_ALIAS_TWO","fixture-two");auto aliases=Client::from_env_with_transport(transport);CHECK(aliases.aliases());CHECK(transport->header("Authorization")=="Bearer fixture-one");
    select.security_alternative=1;CHECK(aliases.aliases(AliasesInput{},select));CHECK(transport->header("Authorization")=="Bearer fixture-two");
    CHECK(unsetenv("CPP_ENV_ALIAS_ONE")==0);auto alias_two=Client::from_env_with_transport(transport);CHECK(alias_two.aliases());CHECK(transport->header("Authorization")=="Bearer fixture-two");
    set("CPP_ENV_BEARER","fixture-bearer");set("CPP_ENV_HEADER","bad\r\nheader");set("CPP_ENV_COOKIE","bad;cookie");auto bad_values=Client::from_env_with_transport(transport);CHECK(bad_values.public_call());before=transport->calls;unavailable(bad_values.together(),*transport,before);
    CHECK(aliases.from_env2());CHECK(aliases.from_env_with_transport2());
#if defined(env_sdk_HAS_CURL)
    auto configured=Client::from_env();CHECK(configured);
#endif
    std::cout<<"configured env string kinds, OR/AND/anonymous, whole explicit argument, per-declaration aliases, helper names and core-portable creation passed\n";
}
