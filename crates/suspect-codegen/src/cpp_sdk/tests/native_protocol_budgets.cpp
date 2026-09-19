#include <generated_sdk/sdk.hpp>
#include <cstdlib>
#include <iostream>
using namespace generated_sdk;
#define CHECK(x) do {if(!(x)){std::cerr<<__LINE__<<" "<<#x;std::abort();}}while(false)
struct Counts {std::size_t polls=0,closed=0;};
class RepeatingBody final:public ResponseBody {
    std::shared_ptr<Counts> counts_;
    bool closed_=false;
public:
    explicit RepeatingBody(std::shared_ptr<Counts> counts):counts_(std::move(counts)){}
    Result<Presence<std::string>,TransportError> next()override{
        ++counts_->polls;return Result<Presence<std::string>,TransportError>::success(counts_->polls<=100?Presence<std::string>{"1\n"}:std::nullopt);
    }
    void close()noexcept override{if(!closed_){++counts_->closed;closed_=true;}}
};
class Fixture final:public Transport {
    std::shared_ptr<Counts> counts_;
public:
    explicit Fixture(std::shared_ptr<Counts> counts):counts_(std::move(counts)){}
    Result<HttpResponse,TransportError> send(const HttpRequest&,const TransportOptions&)const override{CHECK(false);return Result<HttpResponse,TransportError>::failure(TransportError{});}
    Result<HttpExchange,TransportError> open(const HttpRequest&,const TransportOptions&)const override{
        return Result<HttpExchange,TransportError>::success(HttpExchange{200,{{"Content-Type","application/x-ndjson"}},std::make_unique<RepeatingBody>(counts_)});
    }
};
template<class T> static std::size_t consume(T& stream,const std::shared_ptr<Counts>& counts){
    std::size_t items=0;
    for(;;){auto item=stream.next();if(!item){CHECK(item.error().kind==SdkError::Kind::ResourceLimit);CHECK(item.error().codec);CHECK(item.error().response->truncated);CHECK(!item.error().source.pointer.empty());break;}
        CHECK(item.value());CHECK(item.value()->token()=="1");CHECK(++items<100);}
    CHECK(items>0&&counts->polls==items+1&&counts->closed==1);CHECK(!stream.next().value());return items;
}
int main(){
    auto cold=std::make_shared<Counts>();Client first(std::make_shared<Fixture>(cold));auto cold_response=first.collect();CHECK(cold_response);
    const auto cold_count=consume(std::get<CollectStatus200>(cold_response.value()).data,cold);
    auto warm=std::make_shared<Counts>();Client second(std::make_shared<Fixture>(warm));
    auto warm_response=second.warm(WarmInput(std::vector<JsonInteger>(4,JsonInteger(1))));CHECK(warm_response);
    const auto warm_count=consume(std::get<WarmStatus200>(warm_response.value()).data,warm);
    CHECK(warm_count<cold_count);
    std::cout<<"one finite call budget: fresh="<<cold_count<<", after request validation/encoding="<<warm_count<<"; every item is individually schema-valid\n";
}
