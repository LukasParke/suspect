#include "@PACKAGE@/runtime.hpp"
#include "number.hpp"

namespace @NAMESPACE@::detail {
static CodecError mismatch(const Source& source,const std::string& path,const char* message="schema assertion rejected value") {
    return {CodecError::Kind::Validation,source,path,message,std::nullopt};
}
void Validation::merge(Annotations& into,Annotations from,const Source& source,const std::string& path) {
    for(auto& name:from.properties){spend(steps_,1,source,path);into.properties.insert(std::move(name));}
    for(auto index:from.items){spend(steps_,1,source,path);into.items.insert(index);}
}
Validation::Evaluated Validation::evaluate_v2(std::size_t index,const JsonValue& value,const std::string& path,std::size_t depth) {
    if(index>=program_.nodes.size())fail(CodecError::Kind::EvaluationFailure,{},path,"unknown schema target");
    const auto& node=program_.nodes[index];spend(steps_,1,node.source,path);
    if(depth>=program_.max_depth)fail(CodecError::Kind::EvaluationFailure,node.source,path,"schema depth limit");
    ResourceRestore restore{*this,resource_stack_.size(),resource_context_};
    if(resources_)enter_resource(index,node.source,path);
    auto key=std::pair{index,&value};
    auto resource_key=std::tuple{index,&value,resource_context_};
    if(resources_?!resource_active_.insert(resource_key).second:!active_.insert(key).second)fail(CodecError::Kind::EvaluationFailure,node.source,path,"nonproductive recursive schema and resource-context identity");
    struct Leave {Validation& v;decltype(key) identity;decltype(resource_key) resource_identity;~Leave(){if(v.resources_)v.resource_active_.erase(resource_identity);else v.active_.erase(identity);}} leave{*this,key,resource_key};
    Evaluated result;
    for(const auto& check:node.checks){
        spend(steps_,1,check.source,path);
        // A reference needs no opcode-local frame. Keep recursive dispatch
        // small in unoptimized builds, where sibling-case locals share a frame.
        auto next=check.op==Op::Ref?evaluate_v2(check.target,value,path,depth+1):check.op==Op::DynamicRef?evaluate_v2(dynamic_target(check,path),value,path,depth+1):apply_v2(node,check,value,path,depth,result.annotations);
        if(next.failure){if(!result.failure)result.failure=std::move(next.failure);}
        else merge(result.annotations,std::move(next.annotations),check.source,path);
    }
    if(result.failure)result.annotations={};
    return result;
}
void Validation::numeric_instance_v2(const JsonValue& value,const Source& source,const std::string& path) {
    if(numeric_checked_.insert(&value).second){
        const auto& token=value.as<JsonNumber>().token();
        if(token.size()>program_.max_number_bytes)fail(CodecError::Kind::EvaluationFailure,source,path,"exact numeric instance exceeds operand limit");
        spend(numeric_,token.size(),source,path);
    }
}
Validation::Evaluated Validation::same_instance_v2(const Check& check,const JsonValue& value,const std::string& path,std::size_t depth,Annotations& local) {
    if(check.op==Op::Ref)return evaluate_v2(check.target,value,path,depth+1);
    if(check.op==Op::If){
        auto condition=evaluate_v2(check.target,value,path,depth+1);
        const auto branch=condition.failure?check.else_target:check.then_target;
        if(!condition.failure)merge(local,std::move(condition.annotations),check.source,path);
        return branch?evaluate_v2(*branch,value,path,depth+1):Evaluated{};
    }
    if(check.op==Op::Not){auto child=evaluate_v2(check.target,value,path,depth+1);return child.failure?Evaluated{}:Evaluated{mismatch(check.source,path),{}};}
    Evaluated result;std::vector<Annotations> passing;
    for(auto target:check.targets){
        spend(steps_,1,check.source,path);auto child=evaluate_v2(target,value,path,depth+1);
        if(!child.failure)passing.push_back(std::move(child.annotations));
        else if(check.op==Op::AllOf&&!result.failure)result.failure=std::move(child.failure);
    }
    if(check.op==Op::AnyOf&&passing.empty())result.failure=mismatch(check.source,path);
    if(check.op==Op::OneOf&&passing.size()!=1)result.failure=mismatch(check.source,path);
    if(!result.failure)for(auto& annotations:passing)merge(result.annotations,std::move(annotations),check.source,path);
    return result;
}
Validation::Evaluated Validation::object_v2(const Node& node,const Check& check,const JsonValue::Object& object,const std::string& path,std::size_t depth,const Annotations& local) {
    Evaluated result;
    auto child=[&](std::size_t target,const JsonValue& value,const std::string& name,bool mark=true){
        auto next=evaluate_v2(target,value,child_path(path,name),depth+1);
        if(next.failure&&!result.failure)result.failure=std::move(next.failure);
        if(mark)result.annotations.properties.insert(name);
    };
    switch(check.op){
        case Op::Properties:
            for(const auto& property:check.properties){spend(steps_,1,check.source,path);auto found=object.find(property.name);if(found!=object.end())child(property.target,found->second,found->first);}
            break;
        case Op::Required:
            for(const auto& name:check.names){spend(steps_,1,check.source,path);if(!object.contains(name)&&!result.failure)result.failure=mismatch(check.source,path,"required property absent");}
            break;
        case Op::DependentRequired:
            for(std::size_t i=0;i<check.dependencies.size();++i){const auto& [trigger,names]=check.dependencies[i];spend(steps_,1,check.source,path);if(object.contains(trigger))for(const auto& name:names){
                spend(steps_,1,check.source,path);if(!object.contains(name)&&!result.failure){if(i>=check.dependency_sources.size())fail(CodecError::Kind::EvaluationFailure,check.source,path,"dependent source descriptor absent");result.failure=mismatch(check.dependency_sources[i],path,"dependent required property absent");}
            }}
            break;
        case Op::PropertyNames:
            for(const auto& [name,value]:object){(void)value;spend(steps_,1,check.source,path);JsonValue key(name);child(check.target,key,name,false);}
            break;
        case Op::PatternProperties:
            for(const auto& [name,value]:object){spend(steps_,1,check.source,path);for(const auto& item:check.patterns){spend(steps_,1,check.source,path);if(pattern_v2(item.pattern,name,check.source,path))child(item.target,value,name);}}
            break;
        case Op::AdditionalProperties: case Op::AdditionalPropertiesWithPatterns: {
            const Check* patterns=nullptr;
            if(check.op==Op::AdditionalPropertiesWithPatterns){
                for(const auto& candidate:node.checks)if(candidate.op==Op::PatternProperties){patterns=&candidate;break;}
                if(!patterns)fail(CodecError::Kind::EvaluationFailure,check.source,path,"missing adjacent pattern descriptor");
            }
            for(const auto& [name,value]:object){
                spend(steps_,1,check.source,path);if(std::find(check.names.begin(),check.names.end(),name)!=check.names.end())continue;
                bool excluded=false;
                if(patterns)for(const auto& pattern:patterns->patterns){spend(steps_,1,check.source,path);if(pattern_v2(pattern.pattern,name,check.source,path)){excluded=true;break;}}
                if(!excluded)child(check.target,value,name);
            }
            break;
        }
        case Op::UnevaluatedProperties:
            for(const auto& [name,value]:object){spend(steps_,1,check.source,path);if(!local.properties.contains(name))child(check.target,value,name);}
            break;
        default: fail(CodecError::Kind::EvaluationFailure,check.source,path,"invalid object opcode");
    }
    return result;
}
Validation::Evaluated Validation::array_v2(const Node&,const Check& check,const JsonValue::Array& array,const std::string& path,std::size_t depth,Annotations& local) {
    Evaluated result;
    if(check.op==Op::Contains){
        std::size_t matched=0;
        for(std::size_t index=0;index<array.size();++index){spend(steps_,1,check.source,path);auto child=evaluate_v2(check.target,array[index],child_path(path,std::to_string(index)),depth+1);if(!child.failure){++matched;result.annotations.items.insert(index);}}
        auto count=Decimal::parse(std::to_string(matched));
        auto bound=[&](const std::string& token){
            if(token.size()>program_.max_number_bytes)fail(CodecError::Kind::EvaluationFailure,check.source,path,"contains bound exceeds operand limit");
            auto value=JsonNumber::parse(token);if(!value||!value.value().is_integer()||Decimal::parse(token).sign<0)fail(CodecError::Kind::EvaluationFailure,check.source,path,"invalid contains bound");return Decimal::parse(token);
        };
        auto minimum=check.minimum?bound(*check.minimum):Decimal::parse("1");
        if(matched||minimum.sign==0)merge(local,std::move(result.annotations),check.source,path);
        result.annotations={};
        if(count.compare(minimum)<0)result.failure=mismatch(check.minimum_source.value_or(check.source),path,"too few contains matches");
        if(check.maximum&&count.compare(bound(*check.maximum))>0&&!result.failure)result.failure=mismatch(check.maximum_source.value_or(check.source),path,"too many contains matches");
        return result;
    }
    auto first=check.op==Op::Items?check.start:0;
    auto last=check.op==Op::PrefixItems?std::min(check.targets.size(),array.size()):array.size();
    for(auto index=first;index<last;++index){
        spend(steps_,1,check.source,path);
        if(check.op==Op::UnevaluatedItems&&local.items.contains(index))continue;
        auto child=evaluate_v2(check.op==Op::PrefixItems?check.targets[index]:check.target,array[index],child_path(path,std::to_string(index)),depth+1);
        if(child.failure&&!result.failure)result.failure=std::move(child.failure);
        result.annotations.items.insert(index);
    }
    return result;
}
Validation::Evaluated Validation::apply_v2(const Node& node,const Check& check,const JsonValue& value,const std::string& path,std::size_t depth,Annotations& local) {
    switch(check.op){
        case Op::Ref: case Op::AllOf: case Op::AnyOf: case Op::OneOf: case Op::Not: case Op::If:
            return same_instance_v2(check,value,path,depth,local);
        case Op::Properties: case Op::Required: case Op::DependentRequired: case Op::PropertyNames:
        case Op::PatternProperties: case Op::AdditionalProperties: case Op::AdditionalPropertiesWithPatterns: case Op::UnevaluatedProperties:
            return value.is<JsonValue::Object>()?object_v2(node,check,value.as<JsonValue::Object>(),path,depth,local):Evaluated{};
        case Op::Items: case Op::PrefixItems: case Op::Contains: case Op::UnevaluatedItems:
            return value.is<JsonValue::Array>()?array_v2(node,check,value.as<JsonValue::Array>(),path,depth,local):Evaluated{};
        case Op::DependentSchemas:return dependent_v2(check,value,path,depth);
        default:return scalar_v2(check,value,path,depth);
    }
}
Validation::Evaluated Validation::dependent_v2(const Check& check,const JsonValue& value,const std::string& path,std::size_t depth) {
    Evaluated result;std::vector<Annotations> passing;
    if(value.is<JsonValue::Object>())for(const auto& property:check.properties){
        spend(steps_,1,check.source,path);if(value.as<JsonValue::Object>().contains(property.name)){
            auto child=evaluate_v2(property.target,value,path,depth+1);
            if(!child.failure)passing.push_back(std::move(child.annotations));else if(!result.failure)result.failure=std::move(child.failure);
        }
    }
    if(!result.failure)for(auto& annotations:passing)merge(result.annotations,std::move(annotations),check.source,path);
    return result;
}
Validation::Evaluated Validation::scalar_v2(const Check& check,const JsonValue& value,const std::string& path,std::size_t depth) {
    switch(check.op){
        case Op::Type: {
            if(value.is<JsonNumber>()){
                if(std::find(check.names.begin(),check.names.end(),"number")!=check.names.end())return {};
                if(std::find(check.names.begin(),check.names.end(),"integer")!=check.names.end()){numeric_instance_v2(value,check.source,path);if(value.as<JsonNumber>().is_integer())return {};}
                return {mismatch(check.source,path),{}};
            }
            return {instruction(check,value,path,depth),{}};
        }
        case Op::Bound: case Op::MultipleOf: {
            if(!value.is<JsonNumber>())return {};
            numeric_instance_v2(value,check.source,path);
            if(check.operand.size()>program_.max_number_bytes)fail(CodecError::Kind::EvaluationFailure,check.source,path,"numeric operand exceeds limit");
            auto a=Decimal::parse(value.as<JsonNumber>().token()),b=Decimal::parse(check.operand);bool ok;
            if(check.op==Op::MultipleOf)ok=a.divisible(b,[&](std::size_t cost){spend(numeric_,cost,check.source,path);});
            else{auto order=a.compare(b);ok=!((check.flag&&order>0)||(!check.flag&&order<0)||(check.exclusive&&order==0));}
            return ok?Evaluated{}:Evaluated{mismatch(check.source,path),{}};
        }
        case Op::Pattern:
            if(value.is<std::string>()&&!pattern_v2(check.pattern,value.as<std::string>(),check.source,path))return {mismatch(check.source,path),{}};
            return {};
        default:return {instruction(check,value,path,depth),{}};
    }
}
bool Validation::pattern_v2(const PatternProgram& program,std::string_view text,const Source& source,const std::string& path) {
    std::vector<std::uint32_t> seen(program.states.size());std::vector<std::size_t> stack,active,seeds,next;
    std::uint32_t epoch=0;std::size_t offset=0;
    auto enqueue=[&](std::size_t index){spend(steps_,1,source,path);if(index>=seen.size())fail(CodecError::Kind::EvaluationFailure,source,path,"invalid pattern state");if(seen[index]!=epoch){seen[index]=epoch;stack.push_back(index);}};
    for(;;){
        spend(steps_,1,source,path);if(++epoch==0){std::fill(seen.begin(),seen.end(),0);epoch=1;}
        stack.clear();active.clear();enqueue(program.start);for(auto seed:seeds)enqueue(seed);
        while(!stack.empty()){
            auto index=stack.back();stack.pop_back();spend(steps_,1,source,path);const auto& state=program.states[index];
            switch(state.kind){
                case PatternState::Kind::Match:return true;
                case PatternState::Kind::Split:enqueue(state.second);enqueue(state.first);break;
                case PatternState::Kind::Jump:enqueue(state.first);break;
                case PatternState::Kind::Start:if(!offset)enqueue(state.first);break;
                case PatternState::Kind::End:if(offset==text.size())enqueue(state.first);break;
                case PatternState::Kind::Char:active.push_back(index);break;
            }
        }
        if(offset==text.size())return false;
        auto scalar=unicode_scalar(text,offset);next.clear();
        for(auto index:active)for(auto [low,high]:program.states[index].ranges){spend(steps_,1,source,path);if(scalar<low)break;if(scalar<=high){next.push_back(program.states[index].first);break;}}
        seeds.swap(next);
    }
}
} // namespace @NAMESPACE@::detail
