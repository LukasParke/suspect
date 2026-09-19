#pragma once
/** @file stream.hpp Move-only, single-consumer native item streams. */
#include "@PACKAGE@/protocol.hpp"
#include <iterator>

namespace @NAMESPACE@ {
enum class StreamFraming { ServerSentEvents, JsonLines };
namespace detail {
class ItemState {
    HttpExchange exchange_;
    Settings settings_;
    Operation operation_;
    Source source_;
    StreamFraming framing_;
    std::size_t item_limit_,total_=0,polls_,empty_=0,offset_=0,frame_bytes_=0;
    std::string chunk_,line_,data_;
    Presence<std::string> event_,id_;
    Presence<JsonNumber> retry_;
    bool has_data_=false,skip_lf_=false,first_=true,closed_=false;
    Presence<JsonValue> line_item();
public:
    std::unique_ptr<Context> context;
    ResponseMetadata response;
    ItemState(HttpExchange,Settings,Operation,Source,StreamFraming,std::size_t,std::unique_ptr<Context>);
    ~ItemState(){close();}
    Result<Presence<JsonValue>,SdkError> next();
    SdkError fail(CodecError);
    void close() noexcept;
};
std::string encode_item(const JsonValue&,StreamFraming,Context&,const Source&,std::size_t);
}

/** Pull one validated item with next(), or use a move-only range cursor. Starting
 * range iteration transfers the stream lease to that cursor; break/destruction
 * closes the underlying socket even if the original response is retained. */
template<class T> class ItemStream {
    using Decode=T(*)(const JsonValue&,detail::Context&,const std::string&);
    std::unique_ptr<detail::ItemState> state_;
    Decode decode_;
    std::size_t root_;
    static Result<Presence<T>,SdkError> pull(std::unique_ptr<detail::ItemState>& state,Decode decode,std::size_t root){
        using R=Result<Presence<T>,SdkError>;
        if(!state)return R::success(std::nullopt);
        auto value=state->next();
        if(!value){auto error=std::move(value).error();state.reset();return R::failure(std::move(error));}
        if(!value.value()){state.reset();return R::success(std::nullopt);}
        try{
            state->context->validation.require(root,*value.value(),"");
            auto result=decode(*value.value(),*state->context,"");
            state->context->control.check();
            return R::success(std::move(result));
        }catch(detail::Failure& failure){auto error=state->fail(std::move(failure.error));state.reset();return R::failure(std::move(error));}
    }
public:
    ItemStream(std::unique_ptr<detail::ItemState> state,Decode decode,std::size_t root):state_(std::move(state)),decode_(decode),root_(root){}
    ItemStream(const ItemStream&)=delete;
    ItemStream& operator=(const ItemStream&)=delete;
    ItemStream(ItemStream&&) noexcept=default;
    ItemStream& operator=(ItemStream&&) noexcept=default;
    ~ItemStream()=default;
    [[nodiscard]] Result<Presence<T>,SdkError> next(){return pull(state_,decode_,root_);}
    void close() noexcept {state_.reset();}
    [[nodiscard]] bool is_closed() const noexcept {return !state_;}
    class Cursor {
        std::unique_ptr<detail::ItemState> state_;
        Decode decode_;
        std::size_t root_;
        Presence<Result<T,SdkError>> current_;
        void advance(){
            auto value=pull(state_,decode_,root_);
            if(!value)current_=Result<T,SdkError>::failure(std::move(value).error());
            else if(value.value())current_=Result<T,SdkError>::success(std::move(*value.value()));
            else current_.reset();
        }
    public:
        Cursor(std::unique_ptr<detail::ItemState> state,Decode decode,std::size_t root):state_(std::move(state)),decode_(decode),root_(root){advance();}
        Cursor(Cursor&&) noexcept=default;
        Cursor& operator=(Cursor&&) noexcept=default;
        Cursor(const Cursor&)=delete;
        Cursor& operator=(const Cursor&)=delete;
        const Result<T,SdkError>& operator*() const {return *current_;}
        Cursor& operator++(){advance();return *this;}
        void operator++(int){advance();}
        bool operator==(std::default_sentinel_t) const noexcept {return !current_;}
    };
    Cursor begin(){return Cursor(std::move(state_),decode_,root_);}
    std::default_sentinel_t end() const noexcept {return {};}
};
} // namespace @NAMESPACE@
