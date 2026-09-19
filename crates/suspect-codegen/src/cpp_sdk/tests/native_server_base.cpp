#include <generated_sdk/sdk.hpp>
#include <cstdlib>
#include <iostream>
using namespace generated_sdk;
#define CHECK(x) do{if(!(x)){std::cerr<<__LINE__<<" "<<#x;std::abort();}}while(false)
int main(int argc,char** argv){
    CHECK(argc==3);std::string entry=argv[1],fragment=argv[2];unsigned oauth_calls=0,oidc_calls=0;
    Credentials credentials;
    credentials.oauth=CredentialProvider([&](const CredentialRequest& request){
        CHECK(request.scheme_source.document==fragment+"/files/parts.json");CHECK(request.operation_source.document==entry+"/docs/v1/api.json");
        CHECK(request.metadata_url_base=="effective-server");CHECK(request.effective_server_url==(oauth_calls==0?fragment+"/Protected/%2e/Api":entry+"/Override/%2e%2E/v2"));
        auto metadata=write_json(request.metadata);CHECK(metadata);CHECK(metadata.value().find("\"tokens\"")!=std::string::npos);CHECK(metadata.value().find("\"../authorize\"")!=std::string::npos);
        ++oauth_calls;return Result<Authorization,TransportError>::success(Authorization("Bearer","provider-token"));
    });
    credentials.oidc=CredentialProvider([&](const CredentialRequest& request){
        CHECK(request.scheme_source.document==fragment+"/files/parts.json");CHECK(request.effective_server_url==entry+"/");CHECK(request.metadata_url_base=="effective-server");
        auto metadata=write_json(request.metadata);CHECK(metadata&&metadata.value().find("\".well-known/openid-configuration\"")!=std::string::npos);
        ++oidc_calls;return Result<Authorization,TransportError>::success(Authorization("Bearer","oidc-token"));
    });
    auto ready=Client::with_curl(credentials);CHECK(ready);auto& client=ready.value();
    CHECK(client.inherited());CHECK(client.empty());CHECK(client.declared());
    ClientOptions document;document.document_url=entry+"/caller/spec/root.json";
    auto overridden=Client::with_curl(credentials,document);CHECK(overridden);CHECK(overridden.value().declared());
    CHECK(client.variable());CallOptions variables;variables.server_variables=std::map<std::string,std::string,std::less<>>{{"endpoint","../Relative/%2Ekeep"}};
    CHECK(client.variable(VariableInput{},variables));
    ClientOptions full;full.server_url=entry+"/Override/%2e%2E/v2";auto full_client=Client::with_curl(credentials,full);CHECK(full_client);CHECK(full_client.value().inherited());
    CHECK(client.oauth_call());CHECK(full_client.value().oauth_call());CHECK(client.oidc_call());CHECK(oauth_calls==2&&oidc_calls==1);
    auto bytes=client.bytes();CHECK(bytes);CHECK(std::get<BytesStatus200>(bytes.value()).data==Bytes({0,255,1}));
    variables.server_variables=std::map<std::string,std::string,std::less<>>{{"segment","not-in-enum"}};
    auto bad=client.declared(DeclaredInput{},variables);CHECK(!bad);const auto& error=std::get<SdkError>(bad.error());
    CHECK(error.kind==SdkError::Kind::Configuration);CHECK(error.source.document==fragment+"/files/parts.json");
    CHECK(error.source.pointer=="/components/pathItems/Declared/get/servers/0/variables/segment");CHECK(error.source.end>error.source.begin);
    std::cout<<"physical document bases, redirected source ownership, encoded paths, explicit overrides and provider URL context passed\n";
}
