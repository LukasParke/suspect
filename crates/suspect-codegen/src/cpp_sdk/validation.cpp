#include "@PACKAGE@/runtime.hpp"
#include "number.hpp"

namespace @NAMESPACE@::detail {
static CodecError invalid(const Source& source, const std::string& path, std::string message) {
    return {CodecError::Kind::Validation, source, path, std::move(message), std::nullopt};
}
static std::string_view kind(const JsonValue& value) {
    switch (value.value.index()) {
        case 0: return "null"; case 1: return "boolean"; case 2: return "number";
        case 3: return "string"; case 4: return "array"; case 5: return "object";
        default: fail(CodecError::Kind::EvaluationFailure, {}, "", "valueless JSON variant");
    }
}
Validation::Validation(Control control) : Validation(validation_program(), control) {}
Validation::Validation(const Program& checked_program, Control control) : program_(checked_program), control_(control), steps_(program_.max_evaluation_steps), equality_(program_.max_equality_steps), numeric_(program_.max_evaluation_steps) {
    resources_=program_.version=="suspect.validation.experimental.v3"&&program_.profile=="oas31-jsonschema202012-resources-dynamic";
    scoped_=resources_||(program_.version=="suspect.validation.experimental.v2"&&program_.profile=="oas31-jsonschema202012-static-applicators");
    if(!scoped_&&(program_.version!="suspect.validation.experimental.v1"||program_.profile!="oas31-jsonschema202012-static-subset"))
        fail(CodecError::Kind::EvaluationFailure,{},"","unknown validation version/profile pair");
    if(!scoped_)for(const auto& node:program_.nodes)for(const auto& check:node.checks)if(check.op>=Op::If)
        fail(CodecError::Kind::EvaluationFailure,check.source,"","v2 opcode in a v1 program");
    check_resources();
}
void Validation::spend(std::size_t& budget, std::size_t cost, const Source& source, const std::string& path) {
    control_.check(source, path);
    if (cost > budget) fail(CodecError::Kind::EvaluationFailure, source, path, "validation evaluation/equality/numeric work exhausted");
    budget -= cost;
}
bool Validation::matches(std::size_t root, const JsonValue& value, const std::string& path) {
    if (!program_.roots.contains(root)) fail(CodecError::Kind::EvaluationFailure, {}, path, "schema root is not selected");
    numeric_checked_.clear();
    return scoped_?!evaluate_v2(root,value,path,0).failure:!evaluate(root, value, path, 0);
}
void Validation::require(std::size_t root, const JsonValue& value, const std::string& path) {
    if (!program_.roots.contains(root)) fail(CodecError::Kind::EvaluationFailure, {}, path, "schema root is not selected");
    numeric_checked_.clear();
    auto failure = scoped_?evaluate_v2(root,value,path,0).failure:evaluate(root, value, path, 0);
    if (failure) throw Failure{std::move(*failure)};
}
std::optional<CodecError> Validation::evaluate(std::size_t index, const JsonValue& value, const std::string& path, std::size_t depth) {
    if (index >= program_.nodes.size()) fail(CodecError::Kind::EvaluationFailure, {}, path, "unknown schema target");
    auto& node = program_.nodes[index];
    spend(steps_, 1, node.source, path);
    if (depth >= program_.max_depth) fail(CodecError::Kind::EvaluationFailure, node.source, path, "schema depth limit");
    auto key = std::pair{index, &value};
    if (!active_.insert(key).second) fail(CodecError::Kind::EvaluationFailure, node.source, path, "recursive schema revisited the same instance without progress");
    struct ActiveGuard { decltype(active_)& active; decltype(key) key_; ~ActiveGuard() { active.erase(key_); } } guard{active_, key};
    std::optional<CodecError> first;
    for (auto& check : node.checks) {
        spend(steps_, 1, check.source, path);
        auto outcome = instruction(check, value, path, depth);
        if (!first && outcome) first = std::move(outcome);
    }
    return first;
}
std::optional<CodecError> Validation::instruction(const Check& check, const JsonValue& value, const std::string& path, std::size_t depth) {
    auto bad = [&]() { return invalid(check.source, path, "schema assertion rejected value"); };
    auto number = [&](std::string_view token) {
        if (token.size() > program_.max_number_bytes) fail(CodecError::Kind::EvaluationFailure, check.source, path, "validation exact-number operand byte limit");
        spend(numeric_, token.size(), check.source, path);
        return Decimal::parse(token);
    };
    auto conjunction = [&](const auto& each) -> std::optional<CodecError> {
        std::optional<CodecError> first;
        each([&](std::size_t target, const JsonValue& child, const std::string& at) {
            spend(steps_, 1, check.source, path);
            auto outcome = evaluate(target, child, at, depth + 1);
            if (!first && outcome) first = std::move(outcome);
        });
        return first;
    };
    switch (check.op) {
        case Op::Always: if (!check.flag) return bad(); break;
        case Op::Type: {
            for (auto& allowed : check.names) {
                if (allowed == kind(value)) return std::nullopt;
                if (allowed == "integer" && value.is<JsonNumber>() && number(value.as<JsonNumber>().token()).integral()) return std::nullopt;
            }
            return bad();
        }
        case Op::Ref: return evaluate(check.target, value, path, depth + 1);
        case Op::Properties:
            if (value.is<JsonValue::Object>()) return conjunction([&](auto child) {
                for (auto& property : check.properties) {
                    spend(steps_, 1, check.source, path);
                    auto it = value.as<JsonValue::Object>().find(property.name);
                    if (it != value.as<JsonValue::Object>().end()) child(property.target, it->second, child_path(path, property.name));
                }
            });
            break;
        case Op::AdditionalProperties:
            if (value.is<JsonValue::Object>()) return conjunction([&](auto child) {
                for (auto& [key, v] : value.as<JsonValue::Object>()) {
                    spend(steps_, 1, check.source, path);
                    if (std::find(check.names.begin(), check.names.end(), key) == check.names.end()) child(check.target, v, child_path(path, key));
                }
            });
            break;
        case Op::Required:
            if (value.is<JsonValue::Object>()) {
                for (auto& key : check.names) {
                    spend(steps_, 1, check.source, path);
                    if (!value.as<JsonValue::Object>().contains(key)) return invalid(check.source, child_path(path, key), "required member absent");
                }
            }
            break;
        case Op::Items:
            if (value.is<JsonValue::Array>()) return conjunction([&](auto child) {
                auto& array = value.as<JsonValue::Array>();
                for (std::size_t i = check.start; i < array.size(); ++i) child(check.target, array[i], child_path(path, std::to_string(i)));
            });
            break;
        case Op::PrefixItems:
            if (value.is<JsonValue::Array>()) return conjunction([&](auto child) {
                auto& array = value.as<JsonValue::Array>();
                for (std::size_t i = 0; i < std::min(array.size(), check.targets.size()); ++i) child(check.targets[i], array[i], child_path(path, std::to_string(i)));
            });
            break;
        case Op::AllOf: return conjunction([&](auto child) { for (auto target : check.targets) child(target, value, path); });
        case Op::AnyOf: case Op::OneOf: {
            std::size_t matches = 0;
            for (auto target : check.targets) {
                spend(steps_, 1, check.source, path);
                if (!evaluate(target, value, path, depth + 1)) ++matches;
            }
            if (matches == 0 || (check.op == Op::OneOf && matches != 1)) return bad();
            break;
        }
        case Op::Not: if (!evaluate(check.target, value, path, depth + 1)) return bad(); break;
        case Op::Bound: case Op::MultipleOf:
            if (value.is<JsonNumber>()) {
                auto a = number(value.as<JsonNumber>().token()); auto b = number(check.operand);
                if (check.op == Op::Bound) {
                    int order = a.compare(b);
                    if ((check.flag && order > 0) || (!check.flag && order < 0) || (check.exclusive && order == 0)) return bad();
                } else if (!a.divisible(b, [&](std::size_t cost) { spend(numeric_, cost, check.source, path); })) return bad();
            }
            break;
        case Op::Count:
            if (kind(value) == check.operand) {
                auto size = value.is<std::string>() ? unicode_length(value.as<std::string>()) :
                    value.is<JsonValue::Array>() ? value.as<JsonValue::Array>().size() : value.as<JsonValue::Object>().size();
                // Cardinalities are compiled symbolic counts, independent of
                // the numeric-instance operand-byte policy.
                auto a = Decimal::parse(std::to_string(size)); auto b = Decimal::parse(check.names.at(0));
                int order = a.compare(b);
                if ((check.flag && order > 0) || (!check.flag && order < 0)) return bad();
            }
            break;
        case Op::Const:
            if (!equal(value, check.literals.at(0), check.source, path, 0)) return bad();
            break;
        case Op::Enum:
            for (auto& literal : check.literals) {
                spend(steps_, 1, check.source, path);
                if (equal(value, literal, check.source, path, 0)) return std::nullopt;
            }
            return bad();
        case Op::UniqueItems:
            if (value.is<JsonValue::Array>()) {
                auto& array = value.as<JsonValue::Array>();
                for (std::size_t i = 0; i < array.size(); ++i) {
                    spend(steps_, 1, check.source, path);
                    for (std::size_t j = 0; j < i; ++j) {
                        spend(steps_, 1, check.source, path);
                        if (equal(array[i], array[j], check.source, path, 0)) return bad();
                    }
                }
            }
            break;
        case Op::Pattern:
            if (value.is<std::string>() && !pattern(check.pattern, value.as<std::string>(), check.source, path)) return bad();
            break;
        default: fail(CodecError::Kind::EvaluationFailure, check.source, path, "unknown validation instruction");
    }
    return std::nullopt;
}
bool Validation::equal(const JsonValue& a, const JsonValue& b, const Source& source, const std::string& path, std::size_t depth) {
    spend(equality_, 1, source, path);
    if (depth > program_.max_depth) fail(CodecError::Kind::EvaluationFailure, source, path, "equality nesting limit");
    if (kind(a) != kind(b)) return false;
    if (a.is<Null>()) return true;
    if (a.is<bool>()) return a.as<bool>() == b.as<bool>();
    if (a.is<std::string>()) return a.as<std::string>() == b.as<std::string>();
    if (a.is<JsonNumber>()) {
        if (a.as<JsonNumber>().token().size() > program_.max_number_bytes || b.as<JsonNumber>().token().size() > program_.max_number_bytes)
            fail(CodecError::Kind::EvaluationFailure, source, path, "equality exact-number operand byte limit");
        spend(numeric_, a.as<JsonNumber>().token().size() + b.as<JsonNumber>().token().size(), source, path);
        return a.as<JsonNumber>() == b.as<JsonNumber>();
    }
    if (a.is<JsonValue::Array>()) {
        auto& left = a.as<JsonValue::Array>(); auto& right = b.as<JsonValue::Array>();
        if (left.size() != right.size()) return false;
        for (std::size_t i = 0; i < left.size(); ++i) if (!equal(left[i], right[i], source, path, depth + 1)) return false;
        return true;
    }
    auto& left = a.as<JsonValue::Object>(); auto& right = b.as<JsonValue::Object>();
    if (left.size() != right.size()) return false;
    for (auto& [key, v] : left) {
        auto it = right.find(key);
        if (it == right.end() || !equal(v, it->second, source, path, depth + 1)) return false;
    }
    return true;
}

bool Validation::pattern(const PatternProgram& program, std::string_view text, const Source& source, const std::string& path) {
    spend(steps_, 1, source, path);
    // Streaming Unicode scalar iteration avoids an input-sized scalar array.
    std::vector<std::size_t> seeds;
    std::size_t offset = 0;
    for (;;) {
        std::vector<std::size_t> pending = std::move(seeds);
        pending.push_back(program.start);
        std::set<std::size_t> seen;
        std::vector<std::size_t> consuming;
        while (!pending.empty()) {
            spend(steps_, 1, source, path);
            auto index = pending.back(); pending.pop_back();
            if (index >= program.states.size()) fail(CodecError::Kind::EvaluationFailure, source, path, "invalid pattern state");
            if (!seen.insert(index).second) continue;
            auto& state = program.states[index];
            switch (state.kind) {
                case PatternState::Kind::Match: return true;
                case PatternState::Kind::Split: pending.push_back(state.second); pending.push_back(state.first); break;
                case PatternState::Kind::Jump: pending.push_back(state.first); break;
                case PatternState::Kind::Start: if (offset == 0) pending.push_back(state.first); break;
                case PatternState::Kind::End: if (offset == text.size()) pending.push_back(state.first); break;
                case PatternState::Kind::Char: consuming.push_back(index); break;
            }
        }
        if (offset == text.size()) return false;
        spend(steps_, 1, source, path);
        auto point = unicode_scalar(text, offset);
        seeds.clear();
        for (auto index : consuming) {
            auto& state = program.states[index];
            for (auto [low, high] : state.ranges) {
                spend(steps_, 1, source, path);
                if (point < low) break;
                if (point <= high) { seeds.push_back(state.first); break; }
            }
        }
    }
}
} // namespace @NAMESPACE@::detail
