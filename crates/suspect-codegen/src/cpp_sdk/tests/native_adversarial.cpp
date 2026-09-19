#include <generated_sdk/sdk.hpp>
#include <cstdlib>
#include <iostream>
using namespace generated_sdk;
using Root = __Value__;
using RootCodec = __ValueCodec__;
#define CHECK(x) do { if (!(x)) { std::cerr << "failed line " << __LINE__ << ": " << #x << '\n'; std::abort(); } } while(false)
class CountTransport final : public Transport {
public:
    mutable unsigned count=0;
    mutable HttpRequest request;
    Result<HttpResponse,TransportError> send(const HttpRequest& value,const TransportOptions&) const override {
        ++count;request=value;
        return Result<HttpResponse,TransportError>::success({200,{{"Content-Type","application/json"}},R"({"required_nullable":null,"required_value":"yes"})"});
    }
};
class Throwing final : public Transport {
    Result<HttpResponse,TransportError> send(const HttpRequest&,const TransportOptions&) const override {throw std::runtime_error("original adapter cause");}
};
int main() {
    Root root(Null{},"required");
    auto encoded=RootCodec::encode(root);CHECK(encoded);
    CHECK(encoded.value()==R"({"required_nullable":null,"required_value":"required"})");
    CHECK(!RootCodec::decode(R"({"required_value":"x"})"));
    CHECK(!RootCodec::decode(R"({"required_nullable":null})"));
    CHECK(!RootCodec::decode(R"({"required_nullable":null,"required_value":"x","optional_value":null})"));
    root.optional_nullable=Null{};CHECK(RootCodec::encode(root).value().find("\"optional_nullable\":null")!=std::string::npos);
    root.optional_nullable=std::string("present");CHECK(RootCodec::encode(root).value().find("\"optional_nullable\":\"present\"")!=std::string::npos);
    root.optional_nullable=std::nullopt;CHECK(RootCodec::encode(root).value().find("optional_nullable")==std::string::npos);
    // A parent can accept via another arm, but the explicitly selected arm must still pass.
    auto wrong_arm=__Selected__::alternative_0(JsonNumber(10));
    auto rejected=__SelectedCodec__::encode(wrong_arm);CHECK(!rejected && rejected.error().kind==CodecError::Kind::Validation);
    CHECK(rejected.error().source.pointer=="/components/schemas/Selected/anyOf/0/maximum");
    root.selected=wrong_arm;CHECK(!RootCodec::encode(root));root.selected=std::nullopt;
    CHECK(!__SelectedCodec__::encode(__Selected__::alternative_0(JsonNumber(1))));
    CHECK(!__SelectedCodec__::decode("1"));CHECK(__SelectedCodec__::encode(__Selected__::alternative_0(JsonNumber(3))));
    CHECK(__SelectedCodec__::encode(__Selected__::alternative_1(JsonNumber(10))));
    CHECK(!__ExclusiveCodec__::encode(__Exclusive__::alternative_0(JsonInteger(1))));
    CHECK(__ExclusiveCodec__::encode(__Exclusive__::alternative_1(JsonNumber::parse("1.5").value())));
    auto metadata=__MetadataCodec__::decode(R"({"one":1.0,"huge":1e99999999999999999999})");CHECK(metadata);
    CHECK(metadata.value().extra.at("one").token()=="1.0");
    CHECK(__MetadataCodec__::encode(metadata.value()).value().find("1e99999999999999999999")!=std::string::npos);
    metadata.value().extra.emplace("known",JsonInteger(1));auto collision=__MetadataCodec__::encode(metadata.value());CHECK(!collision);
    CHECK(collision.error().kind==CodecError::Kind::Model && collision.error().instance_path=="/known");
    CHECK(!__MetadataCodec__::decode(R"({"other":1.5})"));
    CHECK(__ClosedCodec__::decode("{}"));CHECK(!__ClosedCodec__::decode(R"({"forbidden":null})"));CHECK(!__ClosedCodec__::decode(R"({"other":null})"));
    CHECK(__FalseListCodec__::decode("[]"));CHECK(!__FalseListCodec__::decode("[null]"));
    auto literal=__LiteralCodec__::decode(R"({"a":[1.0,null,true]})");CHECK(literal);
    CHECK(__LiteralCodec__::encode(literal.value()).value()==R"({"a":[1.0,null,true]})");
    CHECK(!__LiteralCodec__::decode(R"({"a":[true,null,true]})"));
    auto long_literal=__LongLiteralCodec__::decode("\""+std::string(70000,'x')+"\"");CHECK(long_literal);
    CHECK(__LongLiteralCodec__::encode(long_literal.value()).value()=="\""+std::string(70000,'x')+"\"");
    auto recursive=__RecursiveCodec__::decode(R"(["leaf",["nested"]])");CHECK(recursive);
    CHECK(__RecursiveCodec__::encode(recursive.value()).value()==R"(["leaf",["nested"]])");
    auto copied=recursive.value();std::get<0>(std::get<1>(copied.value)[0].value().value)="changed";
    CHECK(__RecursiveCodec__::encode(recursive.value()).value()==R"(["leaf",["nested"]])");
    auto nullable=__NullableNodeCodec__::decode(R"({"label":"root","child":null})");CHECK(nullable);
    CHECK(__NullableNodeCodec__::encode(nullable.value()).value()==R"({"child":null,"label":"root"})");
    auto hostile=RootCodec::decode(R"({"required_nullable":null,"required_value":"x","a/b~\u0000":"exact","a-b":"other","class":"keyword","extra":"named","\n#error injected\n\\":"tag\u0000\n*/\\\n#error injected"})");CHECK(hostile);
    auto hostile_json=parse_json(RootCodec::encode(hostile.value()).value());CHECK(hostile_json);
    CHECK(hostile_json.value().as<JsonValue::Object>().at(std::string("a/b~\0",5)).as<std::string>()=="exact");
    CHECK(hostile_json.value().as<JsonValue::Object>().at("a-b").as<std::string>()=="other");
    CHECK(hostile_json.value().as<JsonValue::Object>().at("extra").as<std::string>()=="named");
    root.free=JsonValue(Null{});CHECK(RootCodec::encode(root));
    auto transport=std::make_shared<CountTransport>();Credentials credentials;credentials.api_key="fixture-token";Client client(transport,credentials);
    EchoValueInput input(root);input.enabled=true;input.amount=JsonNumber::parse("1e+400").value();
    CHECK(client.echo_value(input));CHECK(transport->request.url=="https://example.test/api/v1/values?enabled=true&amount=1e%2B400");
    input.value=std::vector<std::string>(10000,"x");CallOptions bounded;bounded.max_request_bytes=8192;
    auto large=client.echo_value(input,bounded);CHECK(!large);
    CHECK(std::get<SdkError>(large.error()).kind==SdkError::Kind::ResourceLimit);CHECK(transport->count==1);
    input.value=std::nullopt;input.lead=std::string(200,' ');bounded.max_request_bytes=512;
    auto expansion=client.echo_value(input,bounded);CHECK(!expansion);CHECK(transport->count==1);
    // Adapter exceptions retain their exact cause and do not become typed API errors.
    auto thrown=Client(std::make_shared<Throwing>(),credentials).echo_value(EchoValueInput(root));CHECK(!thrown);
    const auto& error=std::get<SdkError>(thrown.error());CHECK(error.kind==SdkError::Kind::Transport && error.cause);
    try {std::rethrow_exception(error.cause);}catch(const std::runtime_error& cause){CHECK(std::string(cause.what())=="original adapter cause");}
    // Conversion and JSON emission share one budget rather than receiving fresh allowances.
    JsonLimits limits;limits.max_work=7;detail::Context context({},limits);auto json=context.copy(JsonValue("abc"),{},"");
    bool exhausted=false;try{(void)detail::write_document(json,context,{});}catch(const detail::Failure& failure){exhausted=failure.error.kind==CodecError::Kind::ResourceLimit;}CHECK(exhausted);
    std::cout<<"adversarial native models, selected arms, parent constraints, recursive ownership, presence, names and shared budgets passed\n";
}
