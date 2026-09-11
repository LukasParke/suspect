#include <generated_sdk/sdk.hpp>
#include <cstdlib>
#include <iostream>
using namespace generated_sdk;
#define CHECK(x) do{if(!(x)){std::cerr<<__LINE__<<" "<<#x;std::abort();}}while(false)
static JsonValue object(int value){return JsonValue(JsonValue::Object{{"data",JsonValue(JsonInteger(value))}});}
int main(int argc,char** argv){
    CHECK(argc==2);ClientOptions options;options.server_url=argv[1];auto ready=Client::with_curl({},options);CHECK(ready);auto& client=ready.value();
    Tree tree(JsonInteger(1));tree.children=std::vector<JsonValue>{object(2)};
    auto strict=StrictCodec::encode(tree);CHECK(strict);CHECK(client.strict(StrictInput("ok",tree)));
    tree.children->at(0).as<JsonValue::Object>().emplace("unexpected",JsonValue(true));
    CHECK(!StrictCodec::encode(tree));auto bad=client.strict(StrictInput("ok",tree));CHECK(!bad);
    const auto& request_error=std::get<SdkError>(bad.error());CHECK(request_error.kind==SdkError::Kind::RequestValidation);CHECK(request_error.codec->source.pointer=="/components/schemas/Strict/unevaluatedProperties");CHECK(request_error.codec->instance_path=="/children/0/unexpected");
    // Strict is an indexed candidate but never entered by the Tree operation.
    auto ordinary=client.tree(TreeInput(tree));CHECK(ordinary);CHECK(std::get<TreeStatus200>(ordinary.value()).data.children->at(0).as<JsonValue::Object>().contains("unexpected"));
    tree.children=std::vector<JsonValue>{object(2)};auto bad_response=client.strict(StrictInput("bad",tree));CHECK(!bad_response);const auto& response_error=std::get<SdkError>(bad_response.error());CHECK(response_error.kind==SdkError::Kind::ResponseDecoding);CHECK(response_error.codec->source.pointer=="/components/schemas/Strict/unevaluatedProperties");
    Switch switched(JsonValue(JsonInteger(7)));CHECK(client.switch_value(SwitchValueInput(switched)));
    switched.value=JsonValue("initial target is not selected");CHECK(!SwitchCodec::encode(switched));
    auto detached=JsonValue(JsonInteger(9));CHECK(client.detached(DetachedInput(detached)));
    CHECK(!DetachedDefsStartCodec::encode(JsonValue("not an integer")));
    auto items=client.items();CHECK(items);auto& stream=std::get<ItemsStatus200>(items.value()).data;
    auto first=stream.next();CHECK(first&&first.value()&&first.value()->data.token()=="1");auto second=stream.next();CHECK(!second&&second.error().kind==SdkError::Kind::ResponseDecoding);CHECK(second.error().codec->instance_path=="/children/0/unexpected");CHECK(stream.is_closed());
    std::cout<<"resource/dynamic SDK: checked carriers, active vs unentered overrides, detached entry, mutable codecs, native HTTP and item scopes passed\n";
}
