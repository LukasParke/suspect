#include "@PACKAGE@/runtime.hpp"
#include "number.hpp"
#include <charconv>

namespace @NAMESPACE@ {
namespace detail {
[[noreturn]] void fail(CodecError::Kind kind, const Source& source, std::string_view path, std::string message) {
    throw Failure{CodecError{kind, source, std::string(path), std::move(message), std::nullopt}};
}
void Control::check(const Source& source, std::string_view path) const {
    if (stop.stop_requested()) fail(CodecError::Kind::Cancelled, source, path, "operation cancelled");
    if (deadline && std::chrono::steady_clock::now() >= *deadline) fail(CodecError::Kind::Timeout, source, path, "operation deadline exceeded");
}
std::string child_path(std::string_view parent, std::string_view token) {
    std::string path(parent); path.push_back('/');
    for (char c : token) { if (c == '~') path += "~0"; else if (c == '/') path += "~1"; else path.push_back(c); }
    return path;
}

// A scalar decoder shared by the parser, writer and Unicode cardinalities.
std::uint32_t unicode_scalar(std::string_view text, std::size_t& at) {
    if (at >= text.size()) fail(CodecError::Kind::InvalidJson, {}, "", "incomplete UTF-8");
    auto first = static_cast<unsigned char>(text[at++]);
    if (first < 0x80) return first;
    unsigned tail = 0; std::uint32_t point = 0; std::uint32_t minimum = 0;
    if (first >= 0xC2 && first <= 0xDF) { tail = 1; point = first & 0x1F; minimum = 0x80; }
    else if (first >= 0xE0 && first <= 0xEF) { tail = 2; point = first & 0x0F; minimum = 0x800; }
    else if (first >= 0xF0 && first <= 0xF4) { tail = 3; point = first & 0x07; minimum = 0x10000; }
    else fail(CodecError::Kind::InvalidJson, {}, "", "invalid UTF-8 leading byte");
    for (unsigned i = 0; i < tail; ++i) {
        if (at >= text.size()) fail(CodecError::Kind::InvalidJson, {}, "", "incomplete UTF-8 scalar");
        auto next = static_cast<unsigned char>(text[at++]);
        if ((next & 0xC0) != 0x80) fail(CodecError::Kind::InvalidJson, {}, "", "invalid UTF-8 continuation");
        point = (point << 6) | (next & 0x3F);
    }
    if (point < minimum || point > 0x10FFFF || (point >= 0xD800 && point <= 0xDFFF))
        fail(CodecError::Kind::InvalidJson, {}, "", "invalid Unicode scalar");
    return point;
}
std::size_t unicode_length(std::string_view text) {
    std::size_t count = 0; std::size_t at = 0;
    while (at < text.size()) { unicode_scalar(text, at); ++count; }
    return count;
}
static void utf8(std::string& text, std::uint32_t point) {
    if (point <= 0x7F) text.push_back(static_cast<char>(point));
    else if (point <= 0x7FF) {
        text.push_back(static_cast<char>(0xC0 | (point >> 6)));
        text.push_back(static_cast<char>(0x80 | (point & 0x3F)));
    } else if (point <= 0xFFFF) {
        text.push_back(static_cast<char>(0xE0 | (point >> 12)));
        text.push_back(static_cast<char>(0x80 | ((point >> 6) & 0x3F)));
        text.push_back(static_cast<char>(0x80 | (point & 0x3F)));
    } else {
        text.push_back(static_cast<char>(0xF0 | (point >> 18)));
        text.push_back(static_cast<char>(0x80 | ((point >> 12) & 0x3F)));
        text.push_back(static_cast<char>(0x80 | ((point >> 6) & 0x3F)));
        text.push_back(static_cast<char>(0x80 | (point & 0x3F)));
    }
}
static bool digit(char c) { return c >= '0' && c <= '9'; }
static bool number_grammar(std::string_view token) {
    std::size_t at = 0;
    if (at < token.size() && token[at] == '-') ++at;
    if (at == token.size()) return false;
    if (token[at] == '0') ++at;
    else if (token[at] >= '1' && token[at] <= '9') { while (at < token.size() && digit(token[at])) ++at; }
    else return false;
    if (at < token.size() && token[at] == '.') {
        ++at; auto start = at;
        while (at < token.size() && digit(token[at])) ++at;
        if (at == start) return false;
    }
    if (at < token.size() && (token[at] == 'e' || token[at] == 'E')) {
        ++at;
        if (at < token.size() && (token[at] == '+' || token[at] == '-')) ++at;
        auto start = at;
        while (at < token.size() && digit(token[at])) ++at;
        if (at == start) return false;
    }
    return at == token.size();
}

class Parser {
    std::string_view bytes_;
    JsonLimits limits_;
    std::size_t& work_;
    Control control_;
    std::size_t at_ = 0;
    [[noreturn]] void bad(std::string message, CodecError::Kind kind = CodecError::Kind::InvalidJson) {
        throw Failure{CodecError{kind, {}, "", std::move(message), at_}};
    }
    void spend(std::size_t cost = 1) {
        if (cost > work_) bad("JSON parsing work limit", CodecError::Kind::ResourceLimit);
        work_ -= cost;
        if ((at_ & 1023) == 0) control_.check();
    }
    char take() { spend(); if (at_ == bytes_.size()) bad("unexpected end of JSON"); return bytes_[at_++]; }
    void space() { while (at_ < bytes_.size() && (bytes_[at_] == ' ' || bytes_[at_] == '\r' || bytes_[at_] == '\n' || bytes_[at_] == '\t')) take(); }
    std::uint32_t hex4() {
        std::uint32_t point = 0;
        for (int i = 0; i < 4; ++i) {
            char c = take(); unsigned n;
            if (c >= '0' && c <= '9') n = static_cast<unsigned>(c - '0');
            else if (c >= 'a' && c <= 'f') n = static_cast<unsigned>(c - 'a' + 10);
            else if (c >= 'A' && c <= 'F') n = static_cast<unsigned>(c - 'A' + 10);
            else bad("invalid JSON Unicode escape");
            point = point * 16 + n;
        }
        return point;
    }
    std::string string() {
        if (take() != '"') bad("expected JSON string");
        std::string out;
        while (at_ < bytes_.size()) {
            char c = take();
            if (c == '"') return out;
            if (static_cast<unsigned char>(c) < 0x20) bad("unescaped control in JSON string");
            if (c == '\\') {
                switch (take()) {
                    case '"': out.push_back('"'); break;
                    case '\\': out.push_back('\\'); break;
                    case '/': out.push_back('/'); break;
                    case 'b': out.push_back('\b'); break;
                    case 'f': out.push_back('\f'); break;
                    case 'n': out.push_back('\n'); break;
                    case 'r': out.push_back('\r'); break;
                    case 't': out.push_back('\t'); break;
                    case 'u': {
                        auto point = hex4();
                        if (point >= 0xD800 && point <= 0xDBFF) {
                            if (take() != '\\' || take() != 'u') bad("unpaired high surrogate");
                            auto low = hex4();
                            if (low < 0xDC00 || low > 0xDFFF) bad("invalid low surrogate");
                            point = 0x10000 + (point - 0xD800) * 0x400 + low - 0xDC00;
                        } else if (point >= 0xDC00 && point <= 0xDFFF) bad("unpaired low surrogate");
                        utf8(out, point); break;
                    }
                    default: bad("unknown JSON string escape");
                }
            } else if (static_cast<unsigned char>(c) >= 0x80) {
                auto start = at_ - 1; auto end = start;
                unicode_scalar(bytes_, end); spend(end - at_); at_ = end; out.append(bytes_.substr(start, end - start));
            } else out.push_back(c);
        }
        bad("unterminated JSON string");
    }
    JsonValue value(std::size_t depth) {
        spend();
        if (depth > limits_.max_depth) bad("JSON nesting limit", CodecError::Kind::ResourceLimit);
        space();
        if (at_ == bytes_.size()) bad("missing JSON value");
        char c = bytes_[at_];
        if (c == '"') return JsonValue(string());
        if (c == '[') {
            take(); space(); JsonValue::Array out;
            if (at_ < bytes_.size() && bytes_[at_] == ']') { take(); return JsonValue(std::move(out)); }
            for (;;) {
                out.push_back(value(depth + 1)); space();
                char end = take(); if (end == ']') break; if (end != ',') bad("expected array comma");
            }
            return JsonValue(std::move(out));
        }
        if (c == '{') {
            take(); space(); JsonValue::Object out;
            if (at_ < bytes_.size() && bytes_[at_] == '}') { take(); return JsonValue(std::move(out)); }
            for (;;) {
                space(); auto key = string(); space(); if (take() != ':') bad("expected object colon");
                auto child = value(depth + 1);
                if (!out.emplace(std::move(key), std::move(child)).second) bad("duplicate decoded object key");
                space(); char end = take(); if (end == '}') break; if (end != ',') bad("expected object comma");
            }
            return JsonValue(std::move(out));
        }
        for (auto [word, kind] : {std::pair<std::string_view, int>{"true", 1}, {"false", 2}, {"null", 3}}) {
            if (bytes_.substr(at_, word.size()) == word) {
                spend(word.size()); at_ += word.size();
                return kind == 3 ? JsonValue(Null{}) : JsonValue(kind == 1);
            }
        }
        if (c == '-' || digit(c)) {
            auto start = at_;
            while (at_ < bytes_.size() && (digit(bytes_[at_]) || bytes_[at_] == '-' || bytes_[at_] == '+' || bytes_[at_] == '.' || bytes_[at_] == 'e' || bytes_[at_] == 'E')) take();
            auto parsed = JsonNumber::parse(bytes_.substr(start, at_ - start), limits_.max_number_bytes);
            if (!parsed) { auto error = std::move(parsed).error(); error.byte_offset = start; throw Failure{std::move(error)}; }
            return JsonValue(std::move(parsed).value());
        }
        bad("unexpected JSON token");
    }
public:
    Parser(std::string_view bytes, JsonLimits limits, std::size_t& work, Control control = {}) : bytes_(bytes), limits_(limits), work_(work), control_(control) {}
    Result<JsonValue, CodecError> run() {
        try {
            control_.check();
            if (bytes_.size() > limits_.max_bytes) bad("JSON input byte limit", CodecError::Kind::ResourceLimit);
            if (limits_.max_depth > 256) bad("JSON nesting ceiling exceeds 256", CodecError::Kind::ResourceLimit);
            auto out = value(0); space(); if (at_ != bytes_.size()) bad("trailing JSON input");
            control_.check();
            return Result<JsonValue, CodecError>::success(std::move(out));
        } catch (Failure& failure) {
            if (!failure.error.byte_offset) failure.error.byte_offset = at_;
            return Result<JsonValue, CodecError>::failure(std::move(failure.error));
        }
    }
};

class Writer {
    JsonLimits limits_;
    std::size_t& work_;
    Control control_;
    std::string output_;
    void put(std::string_view bytes, const std::string& path) {
        if (bytes.size() > work_ || bytes.size() > limits_.max_bytes - output_.size())
            fail(CodecError::Kind::ResourceLimit, {}, path, "JSON output byte/work limit");
        work_ -= bytes.size(); output_.append(bytes);
    }
    void quote(std::string_view text, const std::string& path) {
        put("\"", path);
        for (std::size_t at = 0; at < text.size();) {
            if ((at & 1023) == 0) control_.check({}, path);
            auto start = at; auto point = unicode_scalar(text, at);
            if (point == '"') put("\\\"", path);
            else if (point == '\\') put("\\\\", path);
            else if (point < 0x20) {
                static constexpr char hex[] = "0123456789abcdef";
                std::string escape = "\\u00"; escape.push_back(hex[point >> 4]); escape.push_back(hex[point & 15]); put(escape, path);
            } else put(text.substr(start, at - start), path);
        }
        put("\"", path);
    }
    void value(const JsonValue& value, const std::string& path, std::size_t depth) {
        control_.check({}, path);
        if (value.value.valueless_by_exception()) fail(CodecError::Kind::Model, {}, path, "valueless JSON variant");
        if (depth > limits_.max_depth || !work_) fail(CodecError::Kind::ResourceLimit, {}, path, "JSON nesting/work limit");
        --work_;
        if (value.is<Null>()) put("null", path);
        else if (value.is<bool>()) put(value.as<bool>() ? "true" : "false", path);
        else if (value.is<JsonNumber>()) {
            auto& token = value.as<JsonNumber>().token();
            if (token.size() > limits_.max_number_bytes) fail(CodecError::Kind::ResourceLimit, {}, path, "JSON number token limit");
            put(token, path);
        } else if (value.is<std::string>()) quote(value.as<std::string>(), path);
        else if (value.is<JsonValue::Array>()) {
            put("[", path); std::size_t index = 0;
            for (auto& child : value.as<JsonValue::Array>()) {
                if (index) put(",", path);
                this->value(child, child_path(path, std::to_string(index++)), depth + 1);
            }
            put("]", path);
        } else {
            put("{", path); bool first = true;
            for (auto& [key, child] : value.as<JsonValue::Object>()) {
                if (!first) put(",", path);
                first = false;
                quote(key, path); put(":", path); this->value(child, child_path(path, key), depth + 1);
            }
            put("}", path);
        }
    }
public:
    explicit Writer(JsonLimits limits, std::size_t& work, Control control = {}) : limits_(limits), work_(work), control_(control) {}
    Result<std::string, CodecError> run(const JsonValue& json) {
        try {
            if (limits_.max_depth > 256) fail(CodecError::Kind::ResourceLimit, {}, "", "JSON nesting ceiling exceeds 256");
            value(json, "", 0); return Result<std::string, CodecError>::success(std::move(output_));
        } catch (Failure& failure) { return Result<std::string, CodecError>::failure(std::move(failure.error)); }
    }
};

Context::Guard Context::enter(const Source& source, const std::string& path) {
    spend(1, source, path);
    if (depth >= limits.max_depth) fail(CodecError::Kind::ResourceLimit, source, path, "native conversion nesting limit");
    ++depth; return Guard{*this};
}
void Context::spend(std::size_t amount, const Source& source, const std::string& path) {
    control.check(source, path);
    if (amount > remaining) fail(CodecError::Kind::ResourceLimit, source, path, "native conversion work limit");
    remaining -= amount;
}
std::string Context::text(std::string_view value, const Source& source, const std::string& path) {
    spend(value.size(), source, path);
    try { (void)unicode_length(value); }
    catch (Failure&) { fail(CodecError::Kind::Model, source, path, "native string is not valid UTF-8"); }
    return std::string(value);
}
JsonValue Context::copy(const JsonValue& value, const Source& source, const std::string& path) {
    auto guard = enter(source, path);
    if (value.is<Null>()) return JsonValue(Null{});
    if (value.is<bool>()) return JsonValue(value.as<bool>());
    if (value.is<JsonNumber>()) { spend(value.as<JsonNumber>().token().size(), source, path); return JsonValue(value.as<JsonNumber>()); }
    if (value.is<std::string>()) return JsonValue(text(value.as<std::string>(), source, path));
    if (value.is<JsonValue::Array>()) {
        JsonValue::Array out; std::size_t i = 0;
        for (auto& item : value.as<JsonValue::Array>()) out.push_back(copy(item, source, child_path(path, std::to_string(i++))));
        return JsonValue(std::move(out));
    }
    if (!value.is<JsonValue::Object>()) fail(CodecError::Kind::Model, source, path, "valueless JSON variant");
    JsonValue::Object out;
    for (auto& [key, item] : value.as<JsonValue::Object>()) {
        auto name = text(key, source, path);
        out.emplace(std::move(name), copy(item, source, child_path(path, key)));
    }
    return JsonValue(std::move(out));
}
JsonValue parse_document(std::string_view bytes, Context& context, const Source& source) {
    auto parsed = Parser(bytes, context.limits, context.remaining, context.control).run();
    if (!parsed) { auto error = std::move(parsed).error(); error.source = source; throw Failure{std::move(error)}; }
    return std::move(parsed).value();
}
std::string write_document(const JsonValue& value, Context& context, const Source& source) {
    auto written = Writer(context.limits, context.remaining, context.control).run(value);
    if (!written) { auto error = std::move(written).error(); error.source = source; throw Failure{std::move(error)}; }
    return std::move(written).value();
}
const JsonValue& member(const JsonValue::Object& object, std::string_view key, const Source& source, const std::string& path) {
    auto it = object.find(key);
    if (it == object.end()) fail(CodecError::Kind::Model, source, child_path(path, key), "required native member is absent");
    return it->second;
}
JsonValue literal(std::string_view token) {
    JsonLimits limits; limits.max_bytes = token.size(); limits.max_work = token.size() * 8 + 1024; limits.max_number_bytes = token.size(); limits.max_depth = 256;
    auto parsed = parse_json(token, limits);
    if (!parsed) throw Failure{std::move(parsed).error()};
    return std::move(parsed).value();
}
} // namespace detail

Result<JsonNumber, CodecError> JsonNumber::parse(std::string_view token, std::size_t max_bytes) {
    if (token.size() > std::min<std::size_t>(max_bytes, 65536)) return Result<JsonNumber, CodecError>::failure({CodecError::Kind::ResourceLimit, {}, "", "exact number token byte limit", std::nullopt});
    if (!detail::number_grammar(token)) return Result<JsonNumber, CodecError>::failure({CodecError::Kind::InvalidJson, {}, "", "invalid exact JSON number", std::nullopt});
    return Result<JsonNumber, CodecError>::success(JsonNumber(std::string(token)));
}
bool JsonNumber::is_integer() const { return detail::Decimal::parse(token()).integral(); }
int JsonNumber::compare(const JsonNumber& other) const { return detail::Decimal::parse(token()).compare(detail::Decimal::parse(other.token())); }
Result<JsonInteger, CodecError> JsonInteger::from_number(const JsonNumber& number) {
    if (!number.is_integer()) return Result<JsonInteger, CodecError>::failure({CodecError::Kind::Model, {}, "", "exact number is not mathematically integral", std::nullopt});
    return Result<JsonInteger, CodecError>::success(JsonInteger(number));
}
Result<JsonInteger, CodecError> JsonInteger::parse(std::string_view token) {
    auto number = JsonNumber::parse(token);
    if (!number) return Result<JsonInteger, CodecError>::failure(std::move(number).error());
    return from_number(number.value());
}
Result<std::int64_t, CodecError> JsonInteger::to_int64() const {
    auto decimal = detail::Decimal::parse(token());
    if (!decimal.sign) return Result<std::int64_t, CodecError>::success(0);
    auto limit = detail::Decimal::parse(decimal.sign < 0 ? "-9223372036854775808" : "9223372036854775807");
    if (decimal.compare(limit) * decimal.sign > 0)
        return Result<std::int64_t, CodecError>::failure({CodecError::Kind::Model, {}, "", "integer outside int64 range", std::nullopt});
    std::size_t padding = 0;
    for (char c : decimal.exponent.digits) padding = padding * 10 + static_cast<unsigned>(c - '0');
    // The range comparison proves at most 19 total digits before allocation.
    auto text = (decimal.sign < 0 ? "-" : "") + decimal.digits + std::string(padding, '0');
    std::int64_t result = 0;
    auto converted = std::from_chars(text.data(), text.data() + text.size(), result);
    if (converted.ec != std::errc{} || converted.ptr != text.data() + text.size())
        return Result<std::int64_t, CodecError>::failure({CodecError::Kind::Model, {}, "", "integer outside int64 range", std::nullopt});
    return Result<std::int64_t, CodecError>::success(result);
}
Result<JsonValue, CodecError> parse_json(std::string_view bytes, JsonLimits limits) { auto work = limits.max_work; return detail::Parser(bytes, limits, work).run(); }
Result<std::string, CodecError> write_json(const JsonValue& value, JsonLimits limits) { auto work = limits.max_work; return detail::Writer(limits, work).run(value); }
} // namespace @NAMESPACE@
