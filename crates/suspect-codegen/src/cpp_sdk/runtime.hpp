#pragma once
/** @file runtime.hpp Exact JSON, value ownership, and explicit codec outcomes. */

#include <algorithm>
#include <chrono>
#include <cstddef>
#include <cstdint>
#include <map>
#include <memory>
#include <optional>
#include <set>
#include <stop_token>
#include <stdexcept>
#include <string>
#include <string_view>
#include <type_traits>
#include <tuple>
#include <utility>
#include <variant>
#include <vector>

namespace @NAMESPACE@ {

/** A successful operation with no payload. */
struct Unit {};
/** JSON null; distinct from an absent optional member. */
struct Null { friend constexpr bool operator==(Null, Null) = default; };
/** Uninhabited false-schema value. An optional Never can only be absent. */
struct Never { Never() = delete; };
/** Absence is std::nullopt. Presence<Nullable<T>> has three distinct states. */
template<class T> using Presence = std::optional<T>;
/** Required nullable value: Null{} or a T. */
template<class T> using Nullable = std::variant<Null, T>;

/** Explicit operational outcomes. value()/error() require the matching arm;
 * misuse has std::variant's normal bad_variant_access behavior. */
template<class T, class E> class [[nodiscard]] Result {
    std::variant<T, E> storage_;
    template<std::size_t I, class V>
    explicit Result(std::in_place_index_t<I> tag, V&& value) : storage_(tag, std::forward<V>(value)) {}
public:
    static Result success(T value) { return Result(std::in_place_index<0>, std::move(value)); }
    static Result failure(E error) { return Result(std::in_place_index<1>, std::move(error)); }
    [[nodiscard]] bool ok() const noexcept { return storage_.index() == 0; }
    explicit operator bool() const noexcept { return ok(); }
    T& value() & { return std::get<0>(storage_); }
    const T& value() const & { return std::get<0>(storage_); }
    T&& value() && { return std::get<0>(std::move(storage_)); }
    E& error() & { return std::get<1>(storage_); }
    const E& error() const & { return std::get<1>(storage_); }
    E&& error() && { return std::get<1>(std::move(storage_)); }
};

/** Deep-copy, unique recursive ownership. A moved-from Box is empty and is
 * rejected by codecs. It is never interpreted as JSON null or absence. */
template<class T> class Box {
    std::unique_ptr<T> value_;
public:
    template<class U> requires std::is_same_v<std::remove_cvref_t<U>, T>
    explicit Box(U&& value) : value_(std::make_unique<T>(std::forward<U>(value))) {}
    Box(const Box& other) : value_(other.value_ ? std::make_unique<T>(*other.value_) : nullptr) {}
    Box& operator=(const Box& other) { if (this != &other) { Box copy(other); value_.swap(copy.value_); } return *this; }
    Box(Box&&) noexcept = default;
    Box& operator=(Box&&) noexcept = default;
    ~Box() = default;
    [[nodiscard]] bool has_value() const noexcept { return bool(value_); }
    T& value() { if (!value_) throw std::logic_error("empty recursive Box"); return *value_; }
    const T& value() const { if (!value_) throw std::logic_error("empty recursive Box"); return *value_; }
};

/** Stable original source identity and byte range, independent of code layout. */
struct Source {
    std::string document;
    std::string pointer;
    std::size_t begin = 0;
    std::size_t end = 0;
};

/** Completed invalidity is distinct from incomplete evaluation/resource work. */
struct CodecError {
    enum class Kind { InvalidJson, Validation, EvaluationFailure, Model, ResourceLimit, Cancelled, Timeout };
    Kind kind = Kind::Model;
    Source source;
    std::string instance_path;
    std::string message;
    std::optional<std::size_t> byte_offset;
};

/** Exact decimal token. The coefficient and signed decimal exponent remain
 * symbolic; no floating-point conversion or exponent-sized allocation occurs. */
class JsonNumber {
    std::shared_ptr<const std::string> token_ = std::make_shared<const std::string>("0");
    explicit JsonNumber(std::string token) : token_(std::make_shared<const std::string>(std::move(token))) {}
public:
    JsonNumber() = default;
    template<class T> requires (std::is_integral_v<T> && !std::is_same_v<T, bool> && sizeof(T) <= 8)
    explicit JsonNumber(T value) : JsonNumber(std::to_string(value)) {}
    template<class T> requires std::is_floating_point_v<T> JsonNumber(T) = delete;
    JsonNumber(const JsonNumber&) = default;
    JsonNumber& operator=(const JsonNumber&) = default;
    // Immutable tokens may be shared; moved-from numbers remain valid values.
    JsonNumber(JsonNumber&& other) noexcept : token_(other.token_) {}
    JsonNumber& operator=(JsonNumber&& other) noexcept { token_ = other.token_; return *this; }
    static Result<JsonNumber, CodecError> parse(std::string_view token, std::size_t max_bytes = 65536);
    [[nodiscard]] const std::string& token() const noexcept { return *token_; }
    [[nodiscard]] bool is_integer() const;
    [[nodiscard]] int compare(const JsonNumber& other) const;
    friend bool operator==(const JsonNumber& a, const JsonNumber& b) { return a.compare(b) == 0; }
};

/** A mathematically integral exact number, including 1.0 and 1e1000000000000. */
class JsonInteger {
    JsonNumber number_;
    explicit JsonInteger(JsonNumber number) : number_(std::move(number)) {}
public:
    JsonInteger() = default;
    template<class T> requires (std::is_integral_v<T> && !std::is_same_v<T, bool> && sizeof(T) <= 8)
    explicit JsonInteger(T value) : number_(value) {}
    template<class T> requires std::is_floating_point_v<T> JsonInteger(T) = delete;
    static Result<JsonInteger, CodecError> parse(std::string_view token);
    static Result<JsonInteger, CodecError> from_number(const JsonNumber& number);
    [[nodiscard]] const std::string& token() const noexcept { return number_.token(); }
    [[nodiscard]] const JsonNumber& number() const noexcept { return number_; }
    /// Checked mathematical conversion, independent of the token's spelling.
    [[nodiscard]] Result<std::int64_t, CodecError> to_int64() const;
    friend bool operator==(const JsonInteger& a, const JsonInteger& b) { return a.number_ == b.number_; }
};

/** Legitimately unconstrained JSON and open-object extras. Numbers are exact.
 * Mutable strings are checked for valid Unicode again by the writer. */
struct JsonValue {
    using Array = std::vector<JsonValue>;
    using Object = std::map<std::string, JsonValue, std::less<>>;
    using Storage = std::variant<Null, bool, JsonNumber, std::string, Array, Object>;
    Storage value;
    JsonValue() : value(Null{}) {}
    explicit JsonValue(Null v) : value(v) {}
    explicit JsonValue(bool v) : value(v) {}
    explicit JsonValue(JsonNumber v) : value(std::move(v)) {}
    explicit JsonValue(JsonInteger v) : value(v.number()) {}
    explicit JsonValue(std::string v) : value(std::move(v)) {}
    explicit JsonValue(const char* v) : value(std::string(v)) {}
    explicit JsonValue(Array v) : value(std::move(v)) {}
    explicit JsonValue(Object v) : value(std::move(v)) {}
    template<class T> requires (std::is_arithmetic_v<T> && !std::is_same_v<T, bool>) JsonValue(T) = delete;
    template<class T> [[nodiscard]] bool is() const noexcept { return std::holds_alternative<T>(value); }
    template<class T> const T& as() const { return std::get<T>(value); }
    template<class T> T& as() { return std::get<T>(value); }
};

/** Bounded strict UTF-8 JSON: duplicate decoded keys, non-JSON numbers,
 * unpaired surrogates and trailing input are errors. */
struct JsonLimits {
    std::size_t max_bytes = @JSON_BYTES@;
    std::size_t max_depth = @JSON_DEPTH@;
    std::size_t max_work = @JSON_WORK@;
    std::size_t max_number_bytes = 65536;
};
[[nodiscard]] Result<JsonValue, CodecError> parse_json(std::string_view bytes, JsonLimits limits = {});
[[nodiscard]] Result<std::string, CodecError> write_json(const JsonValue& value, JsonLimits limits = {});

/** Scope-owned cancellation. Destruction requests cancellation; callers may
 * also pass a std::jthread/stop_source token directly to CallOptions. */
class Cancellation {
    std::stop_source source_;
public:
    Cancellation() = default;
    Cancellation(const Cancellation&) = delete;
    Cancellation& operator=(const Cancellation&) = delete;
    Cancellation(Cancellation&&) noexcept = default;
    Cancellation& operator=(Cancellation&& other) noexcept {
        if (this != &other) { source_.request_stop(); source_ = std::move(other.source_); } return *this;
    }
    ~Cancellation() { source_.request_stop(); }
    [[nodiscard]] std::stop_token token() const noexcept { return source_.get_token(); }
    void cancel() noexcept { source_.request_stop(); }
};

/// Implementation details are exposed solely to support generated source files.
namespace detail {
struct Failure { CodecError error; };
[[noreturn]] void fail(CodecError::Kind kind, const Source& source, std::string_view path, std::string message);
[[nodiscard]] std::string child_path(std::string_view parent, std::string_view token);
[[nodiscard]] std::size_t unicode_length(std::string_view text);
std::uint32_t unicode_scalar(std::string_view text, std::size_t& offset);

struct Control {
    std::stop_token stop;
    std::optional<std::chrono::steady_clock::time_point> deadline;
    void check(const Source& source = {}, std::string_view path = "") const;
};

enum class Op { Always, Type, Ref, Properties, AdditionalProperties, Required, Items, PrefixItems,
    AllOf, AnyOf, OneOf, Not, Bound, MultipleOf, Count, Enum, Const, UniqueItems, Pattern,
    If, DependentRequired, DependentSchemas, Contains, PatternProperties,
    AdditionalPropertiesWithPatterns, PropertyNames, UnevaluatedProperties, UnevaluatedItems, DynamicRef };
struct PatternState {
    enum class Kind { Match, Char, Split, Jump, Start, End };
    Kind kind = Kind::Match;
    std::size_t first = 0;
    std::size_t second = 0;
    std::vector<std::pair<std::uint32_t, std::uint32_t>> ranges;
};
struct PatternProgram { std::size_t start = 0; std::vector<PatternState> states; };
struct Property { std::string name; std::size_t target; };
struct PropertyPattern { std::string text; PatternProgram pattern; std::size_t target; };
struct Check {
    Source source;
    Op op = Op::Always;
    bool flag = false;
    bool exclusive = false;
    std::size_t target = 0;
    std::size_t start = 0;
    std::vector<std::size_t> targets;
    std::vector<std::string> names;
    std::vector<Property> properties;
    std::string operand;
    std::vector<JsonValue> literals;
    PatternProgram pattern;
    Presence<std::size_t> then_target, else_target;
    Presence<std::string> minimum, maximum;
    Presence<Source> minimum_source, maximum_source;
    std::vector<std::pair<std::string,std::vector<std::string>>> dependencies;
    std::vector<Source> dependency_sources;
    std::vector<PropertyPattern> patterns;
    std::size_t initial_resource = 0;
    Presence<std::string> dynamic_anchor;
};
struct Node { Source source; std::vector<Check> checks; };
struct DynamicBinding { std::string name; Source source; std::size_t target; };
struct SchemaResource {
    Source source;
    std::string kind, canonical_uri, base_uri;
    std::vector<std::string> aliases;
    Presence<Source> declaration_source;
    std::vector<DynamicBinding> dynamic_anchors;
};
struct NodeResourceScope { std::size_t resource; Source schema_root; std::string canonical_address; };
struct ProgramResourceContext { std::vector<SchemaResource> resources; std::vector<NodeResourceScope> node_scopes; };
struct Program {
    std::string version = "suspect.validation.experimental.v1";
    std::string profile = "oas31-jsonschema202012-static-subset";
    std::vector<Node> nodes;
    std::set<std::size_t> roots;
    std::size_t max_depth = @SCHEMA_DEPTH@;
    std::size_t max_number_bytes = @NUMBER_BYTES@;
    std::size_t max_equality_steps = @EQUALITY_WORK@;
    std::size_t max_evaluation_steps = @SCHEMA_WORK@;
    Presence<ProgramResourceContext> resource_context;
};
const Program& validation_program();
JsonValue literal(std::string_view token);

/** Every logical trial shares budgets. Evaluation failures throw Failure
 * internally and cannot be suppressed as an invalid union branch. */
class Validation {
    const Program& program_;
    Control control_;
    std::size_t steps_;
    std::size_t equality_;
    std::size_t numeric_;
    std::set<std::pair<std::size_t, const JsonValue*>> active_;
    bool scoped_ = false;
    bool resources_ = false;
    std::vector<std::size_t> resource_stack_;
    std::set<std::size_t> entered_resources_;
    std::size_t resource_context_ = 0;
    std::map<std::pair<std::size_t,std::size_t>,std::size_t> resource_contexts_;
    std::set<std::tuple<std::size_t,const JsonValue*,std::size_t>> resource_active_;
    struct ResourceRestore {
        Validation& validation; std::size_t size, context;
        ~ResourceRestore();
    };
    void check_resources();
    void enter_resource(std::size_t node,const Source&,const std::string&);
    std::size_t dynamic_target(const Check&,const std::string&);
    std::set<const JsonValue*> numeric_checked_;
    struct Annotations { std::set<std::string,std::less<>> properties; std::set<std::size_t> items; };
    struct Evaluated { Presence<CodecError> failure; Annotations annotations; };
    Evaluated evaluate_v2(std::size_t, const JsonValue&, const std::string&, std::size_t);
    Evaluated apply_v2(const Node&, const Check&, const JsonValue&, const std::string&, std::size_t, Annotations&);
    Evaluated object_v2(const Node&, const Check&, const JsonValue::Object&, const std::string&, std::size_t, const Annotations&);
    Evaluated array_v2(const Node&, const Check&, const JsonValue::Array&, const std::string&, std::size_t, Annotations&);
    Evaluated same_instance_v2(const Check&, const JsonValue&, const std::string&, std::size_t, Annotations&);
    Evaluated dependent_v2(const Check&, const JsonValue&, const std::string&, std::size_t);
    Evaluated scalar_v2(const Check&, const JsonValue&, const std::string&, std::size_t);
    void merge(Annotations&, Annotations, const Source&, const std::string&);
    bool pattern_v2(const PatternProgram&, std::string_view, const Source&, const std::string&);
    void numeric_instance_v2(const JsonValue&, const Source&, const std::string&);
    std::optional<CodecError> evaluate(std::size_t index, const JsonValue& value, const std::string& path, std::size_t depth);
    std::optional<CodecError> instruction(const Check& check, const JsonValue& value, const std::string& path, std::size_t depth);
    bool equal(const JsonValue& a, const JsonValue& b, const Source& source, const std::string& path, std::size_t depth);
    bool pattern(const PatternProgram& program, std::string_view text, const Source& source, const std::string& path);
    void spend(std::size_t& budget, std::size_t cost, const Source& source, const std::string& path);
public:
    explicit Validation(Control control = {});
    explicit Validation(const Program& checked_program, Control control = {});
    bool matches(std::size_t root, const JsonValue& value, const std::string& path);
    void require(std::size_t root, const JsonValue& value, const std::string& path);
};

struct Context {
    Control control;
    Validation validation;
    JsonLimits limits;
    std::size_t remaining;
    std::size_t depth = 0;
    explicit Context(Control c = {}, JsonLimits l = {}) : control(c), validation(c), limits(l), remaining(l.max_work) {}
    Context(const Context&) = delete;
    Context& operator=(const Context&) = delete;
    struct Guard { Context& context; ~Guard() { --context.depth; } };
    Guard enter(const Source& source, const std::string& path);
    void spend(std::size_t amount, const Source& source, const std::string& path);
    std::string text(std::string_view value, const Source& source, const std::string& path);
    JsonValue copy(const JsonValue& value, const Source& source, const std::string& path);
};
JsonValue parse_document(std::string_view bytes, Context& context, const Source& source);
std::string write_document(const JsonValue& value, Context& context, const Source& source);

template<class T> const T& as(const JsonValue& value, const Source& source, const std::string& path) {
    if (auto p = std::get_if<T>(&value.value)) return *p;
    fail(CodecError::Kind::Model, source, path, "JSON/native representation mismatch");
}
const JsonValue& member(const JsonValue::Object& object, std::string_view key, const Source& source, const std::string& path);

template<class T, class Decode>
Result<T, CodecError> decode_model(std::size_t root, std::string_view bytes, Decode decode) {
    try { Context context; auto parsed = parse_document(bytes, context, validation_program().nodes.at(root).source);
        context.validation.require(root, parsed, "");
        return Result<T, CodecError>::success(decode(parsed, context, std::string{})); }
    catch (Failure& failure) { return Result<T, CodecError>::failure(std::move(failure.error)); }
}
template<class T, class Encode>
Result<JsonValue, CodecError> encode_model(std::size_t root, const T& value, Encode encode) {
    try { Context context; auto json = encode(value, context, std::string{});
        context.validation.require(root, json, "");
        return Result<JsonValue, CodecError>::success(std::move(json)); }
    catch (Failure& failure) { return Result<JsonValue, CodecError>::failure(std::move(failure.error)); }
}
template<class T, class Encode>
Result<std::string, CodecError> encode_bytes(std::size_t root, const T& value, Encode encode) {
    try { Context context; auto json = encode(value, context, std::string{});
        context.validation.require(root, json, "");
        return Result<std::string, CodecError>::success(write_document(json, context, validation_program().nodes.at(root).source)); }
    catch (Failure& failure) { return Result<std::string, CodecError>::failure(std::move(failure.error)); }
}
} // namespace detail
} // namespace @NAMESPACE@
