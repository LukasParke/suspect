#include "@PACKAGE@/runtime.hpp"
#include <limits>

namespace @NAMESPACE@::detail {
void Validation::check_resources() {
    if(!resources_){
        if(program_.resource_context)fail(CodecError::Kind::EvaluationFailure,{},"","resource metadata requires the v3 profile");
        for(const auto& node:program_.nodes)for(const auto& check:node.checks)if(check.op==Op::DynamicRef)fail(CodecError::Kind::EvaluationFailure,check.source,"","dynamic reference requires the v3 profile");
        return;
    }
    if(!program_.resource_context)fail(CodecError::Kind::EvaluationFailure,{},"","v3 resource context is absent");
    const auto& context=*program_.resource_context;
    if(context.node_scopes.size()!=program_.nodes.size())fail(CodecError::Kind::EvaluationFailure,{},"","resource node scopes are not aligned");
    for(std::size_t index=0;index<context.node_scopes.size();++index){
        const auto& scope=context.node_scopes[index];const auto& node=program_.nodes[index];
        if(scope.resource>=context.resources.size())fail(CodecError::Kind::EvaluationFailure,node.source,"","unknown node resource");
        for(const auto& check:node.checks)if(check.op==Op::DynamicRef){
            if(check.target>=context.node_scopes.size()||check.initial_resource>=context.resources.size()||context.node_scopes[check.target].resource!=check.initial_resource)
                fail(CodecError::Kind::EvaluationFailure,check.source,"","dynamic fallback resource/target mismatch");
            if(check.dynamic_anchor){
                const auto& bindings=context.resources[check.initial_resource].dynamic_anchors;
                if(std::none_of(bindings.begin(),bindings.end(),[&](const auto& b){return b.name==*check.dynamic_anchor&&b.target==check.target;}))
                    fail(CodecError::Kind::EvaluationFailure,check.source,"","dynamic fallback does not declare its anchor");
            }
        }
    }
    for(std::size_t index=0;index<context.resources.size();++index){
        std::set<std::string> names;
        for(const auto& binding:context.resources[index].dynamic_anchors){
            if(binding.target>=context.node_scopes.size()||context.node_scopes[binding.target].resource!=index||!names.insert(binding.name).second)
                fail(CodecError::Kind::EvaluationFailure,binding.source,"","invalid resource binding target or duplicate anchor");
        }
    }
}
Validation::ResourceRestore::~ResourceRestore() {
    while(validation.resource_stack_.size()>size){validation.entered_resources_.erase(validation.resource_stack_.back());validation.resource_stack_.pop_back();}
    validation.resource_context_=context;
}
void Validation::enter_resource(std::size_t node,const Source& source,const std::string& path) {
    const auto resource=program_.resource_context->node_scopes[node].resource;
    if(entered_resources_.contains(resource))return;
    spend(steps_,1,source,path);
    const auto prefix=std::pair{resource_context_,resource};
    auto found=resource_contexts_.find(prefix);
    if(found==resource_contexts_.end()){
        if(resource_contexts_.size()==std::numeric_limits<std::size_t>::max())fail(CodecError::Kind::EvaluationFailure,source,path,"resource context identity exhausted");
        found=resource_contexts_.emplace(prefix,resource_contexts_.size()+1).first;
    }
    // The pair is an exact ordered prefix identity, not a hash fingerprint.
    resource_stack_.push_back(resource);entered_resources_.insert(resource);resource_context_=found->second;
}
std::size_t Validation::dynamic_target(const Check& check,const std::string& path) {
    if(!resources_)fail(CodecError::Kind::EvaluationFailure,check.source,path,"dynamic reference outside v3 execution");
    if(!check.dynamic_anchor)return check.target;
    for(auto resource:resource_stack_){
        spend(steps_,1,check.source,path);
        for(const auto& binding:program_.resource_context->resources[resource].dynamic_anchors){
            spend(steps_,1,check.source,path);
            if(binding.name==*check.dynamic_anchor)return binding.target;
        }
    }
    return check.target;
}
} // namespace @NAMESPACE@::detail
