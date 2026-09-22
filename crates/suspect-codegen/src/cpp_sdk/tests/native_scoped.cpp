#include <generated_sdk/sdk.hpp>
#include <cstdlib>
#include <iostream>
#include <type_traits>
using namespace generated_sdk;
#define CHECK(x) do{if(!(x)){std::cerr<<__LINE__<<" "<<#x;std::abort();}}while(false)
template<class R>const SdkError& error(const R& result,SdkError::Kind kind){CHECK(!result);const auto* e=std::get_if<SdkError>(&result.error());CHECK(e);if(e->kind!=kind)std::cerr<<e->message;CHECK(e->kind==kind);return *e;}
static JsonValue number(int n){return JsonValue(JsonInteger(n));}
int main(int argc,char** argv){
    CHECK(argc==2);ClientOptions options;options.server_url=argv[1];auto connected=Client::with_curl({},options);CHECK(connected);auto& client=connected.value();
    Record record(JsonInteger(3),RecordMode::Num);record.extra.emplace("x-positive",number(7));
    auto parsed=RecordCodec::decode("{\"base\":3,\"mode\":\"num\",\"x-positive\":7}");CHECK(parsed);CHECK(parsed.value().extra.at("x-positive").as<JsonNumber>().token()=="7");
    auto result=client.apply_record(ApplyRecordInput("ok",record));CHECK(result);CHECK(std::get<ApplyRecordStatus200>(result.value()).data.extra.at("x-positive").as<JsonNumber>().token()=="7");
    auto retained=RecordCodec::encode(record);CHECK(retained);record.extra.at("x-positive")=number(0);
    const auto overlap=error(client.apply_record(ApplyRecordInput("ok",record)),SdkError::Kind::RequestValidation);CHECK(overlap.codec->source.pointer.ends_with("/patternProperties/-positive$/minimum"));CHECK(overlap.codec->instance_path=="/x-positive");
    record.extra.at("x-positive")=number(7);record.note=Null{};
    const auto dependent=error(client.apply_record(ApplyRecordInput("ok",record)),SdkError::Kind::RequestValidation);CHECK(dependent.codec->source.pointer.ends_with("/dependentRequired/note"));record.note.reset();
    record.base=JsonInteger(1);error(client.apply_record(ApplyRecordInput("ok",record)),SdkError::Kind::RequestValidation);record.base=JsonInteger(3);
    record.extra.emplace("unknown",number(1));CHECK(!RecordCodec::encode(record));record.extra.erase("unknown");
    auto invalid_response=client.apply_record(ApplyRecordInput("bad-response",record));CHECK(error(invalid_response,SdkError::Kind::ResponseDecoding).codec->source.pointer.ends_with("/patternProperties/-positive$/minimum"));
    Ledger ledger(JsonInteger(1));ledger.extra.emplace("s-label",JsonValue("yes"));ledger.extra.emplace("other",number(2));
    auto led=client.ledger(LedgerInput(ledger));CHECK(led);CHECK(std::get<LedgerStatus200>(led.value()).data.extra.at("s-label").as<std::string>()=="yes");
    ledger.extra["other"]=JsonValue("not an integer");CHECK(!LedgerCodec::encode(ledger));ledger.extra["other"]=number(2);
    ledger.count=JsonInteger(0);CHECK(!LedgerCodec::encode(ledger));ledger.count=JsonInteger(1);
    auto choice=ChoiceCodec::decode("{\"a\":2}");CHECK(choice);CHECK(client.choice(ChoiceInput(choice.value())));
    choice.value().as<JsonValue::Object>().emplace("extra",number(1));CHECK(!ChoiceCodec::encode(choice.value()));
    auto sequence=SequenceCodec::decode("[\"header\",1]");CHECK(sequence);CHECK(client.sequence(SequenceInput(sequence.value())));
    sequence.value().as<JsonValue::Array>().push_back(JsonValue(false));CHECK(!SequenceCodec::encode(sequence.value()));
    Node node;node.extra.emplace("x-one",number(1));Node child;child.extra.emplace("x-two",number(2));node.next=Box<Node>(child);
    auto copy=node;copy.next->value().extra["x-two"]=number(3);CHECK(node.next->value().extra.at("x-two").as<JsonNumber>().token()=="2");
    CHECK(client.node(NodeInput(node)));auto moved=std::move(*copy.next);CHECK(!NodeCodec::encode(copy));CHECK(moved.has_value());
    node.extra.emplace("not-evaluated",number(2));CHECK(!NodeCodec::encode(node));
    auto items=client.items();CHECK(items);auto& stream=std::get<ItemsStatus200>(items.value()).data;
    auto first=stream.next();CHECK(first&&first.value());CHECK(first.value()->extra.at("s-label").as<std::string>()=="yes");
    auto second=stream.next();CHECK(!second&&second.error().kind==SdkError::Kind::ResponseDecoding);CHECK(second.error().codec->source.pointer.ends_with("/additionalProperties/type"));CHECK(stream.is_closed());
    std::cout<<"scoped models, overlapping extras, conditional/dependency rules, checked carriers, recursive ownership, mutable HTTP and item codecs passed\n";
}
