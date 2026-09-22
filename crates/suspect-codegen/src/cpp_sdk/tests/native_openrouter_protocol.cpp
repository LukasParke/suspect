#include <generated_sdk/sdk.hpp>
#include <cstdlib>
#include <iostream>
using namespace generated_sdk;
#define CHECK(x) do {if(!(x)){std::cerr<<__LINE__<<" "<<#x;std::abort();}}while(false)
int main(int argc,char** argv){
    CHECK(argc==2);Credentials credentials;credentials.api_key="openrouter-fixture-token";
    ClientOptions options;options.server_url=argv[1];auto ready=Client::with_curl(credentials,options);CHECK(ready);auto client=std::move(ready).value();
    auto container=client.download_container_file_content(DownloadContainerFileContentInput("sess_abc123","cfile_a/b 雪"));CHECK(container);
    CHECK(std::get<DownloadContainerFileContentStatus200>(container.value()).data==Bytes({0,255,1,2,13,10}));
    DownloadFileContentInput request("file_1");request.workspace_id="workspace 1";
    auto file=client.download_file_content(request);CHECK(file);CHECK(std::get<DownloadFileContentStatus200>(file.value()).data==Bytes({0,255,1,2,13,10}));
    auto absent=client.create_coinbase_charge();CHECK(absent);CHECK(std::get<CreateCoinbaseChargeStatus200>(absent.value()).data.empty());
    auto gone=client.create_coinbase_charge();CHECK(!gone);CHECK(std::get<CreateCoinbaseChargeStatus410>(gone.error()).data.error.code.token()=="410");
    CHECK(std::get<CreateCoinbaseChargeStatus410>(gone.error()).data.error.message=="removed");
    std::cout<<"three additional actual OpenRouter operations preserve bytes, anonymous security and declared empty-content semantics\n";
}
