#include <generated_sdk/sdk.hpp>
#include <cstdlib>
#include <iostream>
using namespace generated_sdk;
using CreateBody = __CREATE_BODY__;
using UpdateBody = __UPDATE_BODY__;
#define CHECK(x) do { if (!(x)) { std::cerr << "failed line " << __LINE__ << ": " << #x << '\n'; std::abort(); } } while(false)
int main(int argc,char** argv) {
    CHECK(argc==2);
    Credentials credentials;credentials.api_key="management-fixture-token";
    ClientOptions options;options.server_url=argv[1];
    auto ready=Client::with_curl(credentials,options);CHECK(ready);
    auto client=std::move(ready).value();
    auto credits=client.get_credits(GetCreditsInput());CHECK(credits);
    const auto& balance=std::get<GetCreditsStatus200>(credits.value());
    CHECK(balance.response.status==200);
    CHECK(balance.data.data.total_credits.token()=="100.50000000000000001");
    CHECK(balance.data.data.total_usage.token()=="25.75");
    auto denied=client.get_credits(GetCreditsInput());CHECK(!denied);
    const auto& unauthorized=std::get<GetCreditsStatus401>(denied.error());
    CHECK(unauthorized.response.status==401);
    CHECK(unauthorized.data.error.code.token()=="401");
    CHECK(unauthorized.data.error.message=="Missing Authentication header");
    CHECK(!unauthorized.data.user_id);
    CreateBody create("Native Test Key");
    create.limit=JsonNumber::parse("50.25").value();create.limit_reset=Null{};
    auto made=client.create_keys(CreateKeysInput(create));CHECK(made);
    const auto& key=std::get<CreateKeysStatus201>(made.value());
    CHECK(key.response.status==201);CHECK(key.data.key=="fixture-secret");
    CHECK(key.data.data.hash=="fixture-hash");
    CHECK(std::get<JsonNumber>(key.data.data.limit).token()=="50.250");
    CHECK(std::holds_alternative<Null>(key.data.data.updated_at));
    CHECK(std::holds_alternative<Null>(key.data.data.external_user));
    UpdateBody update;
    update.disabled=true;update.limit=JsonNumber::parse("75.50").value();
    update.limit_reset=Null{};update.name="Updated Native Key";
    __UPDATE_CONSTRUCTION__
    auto changed=client.update_keys(update_input);CHECK(changed);
    const auto& updated=std::get<UpdateKeysStatus200>(changed.value()).data.data;
    CHECK(updated.name=="Updated Native Key");CHECK(std::get<JsonNumber>(updated.limit_remaining).token()=="49.5");
    auto got=client.get_container_file(GetContainerFileInput("sess_abc123","cfile_a/b 雪!'()*"));CHECK(got);
    const auto& file=std::get<GetContainerFileStatus200>(got.value()).data;
    CHECK(file.id=="cfile-1");CHECK(file.container_id=="sess_abc123");CHECK(file.path=="out/report.csv");
    CHECK(file.bytes.token()=="123");CHECK(file.created_at.token()=="1755640000");
    ListContainerFilesInput list("sess_abc123");list.limit=JsonInteger(2);list.after="a/b +雪";
    auto listed=client.list_container_files(list);CHECK(listed);
    const auto& page=std::get<ListContainerFilesStatus200>(listed.value()).data;
    CHECK(page.data.size()==1);CHECK(!page.has_more);
    CHECK(std::get<std::string>(page.first_id)=="cfile-1");
    CHECK(page.data[0].bytes.token()=="123");
    // Mutable request revalidation prevents the seventh HTTP exchange.
    create.name.clear();auto invalid=client.create_keys(CreateKeysInput(create));CHECK(!invalid);
    CHECK(std::get<SdkError>(invalid.error()).kind==SdkError::Kind::RequestValidation);
    CHECK(std::get<SdkError>(invalid.error()).codec->instance_path=="/name");
    std::cout<<"five actual OpenRouter operations, six real HTTP exchanges and exact typed failures passed\n";
}
