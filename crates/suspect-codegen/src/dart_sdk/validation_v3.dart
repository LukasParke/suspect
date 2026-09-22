// V3's only execution additions to the frozen scoped-applicator engine.
final class _DynamicResource {
  const _DynamicResource(this.bindings);
  final List<(String,int)> bindings;
}
final class _DynamicReference {
  const _DynamicReference(this.target,this.initialResource,this.anchor);
  final int target; final int initialResource; final String? anchor;
}
final class _ValidationSession extends _ValidationBase {
  final List<int> resourceStack=[];
  final Set<int> enteredResources={};
  // Intern exact ordered contexts. Record equality checks both components;
  // neither a truncated fingerprint nor node/instance identity alone suffices.
  final Map<(int,int),int> contexts={};
  final Map<(int,int),Set<JsonValue>> scopedActive={};
  int context=0;

  int? enter(int resource,SchemaSource source,String path) {
    if(enteredResources.contains(resource))return null;
    spend(source,path);
    final previous=context;
    context=contexts.putIfAbsent((previous,resource),()=>contexts.length+1);
    enteredResources.add(resource);resourceStack.add(resource);
    return previous;
  }
  void leave(int? previous) {
    if(previous==null)return;
    enteredResources.remove(resourceStack.removeLast());context=previous;
  }
  @override
  _Evaluated node(int index,JsonValue value,String path) {
    final owner=_validationNodes[index];spend(owner.source,path);
    if(depth>=_validationLimits.maxDepth)fail(owner.source,path,'schema evaluation depth budget exhausted');
    // Nested entry uses its indexed resource; the resource root is not evaluated.
    final previous=enter(_nodeResources[index],owner.source,path);
    final instances=scopedActive.putIfAbsent((index,context),HashSet<JsonValue>.identity);
    if(!instances.add(value)) {
      leave(previous);fail(owner.source,path,'nonproductive recursive schema evaluation');
    }
    depth++;
    try {
      var valid=true;final local=_Annotations();
      for(final check in owner.checks) {
        spend(check.source,path);
        final accepted=run(owner,check,value,path,local);valid=accepted&&valid;
      }
      return _Evaluated(valid,valid?local:_Annotations());
    } finally {depth--;instances.remove(value);leave(previous);}
  }
  @override
  bool run(_ValidationNode owner,_Check check,JsonValue value,String path,_Annotations local) {
    if(check.op!=_Op.ref||check.target>=0)return super.run(owner,check,value,path,local);
    final reference=_dynamicReferences[check];
    if(reference==null)fail(check.source,path,'missing checked dynamic reference');
    var target=reference.target;
    final anchor=reference.anchor;
    if(anchor!=null) {
      lookup:
      for(final resource in resourceStack) {
        spend(check.source,path);
        for(final (name,candidate) in _validationResources[resource].bindings) {
          spend(check.source,path);
          if(name==anchor){target=candidate;break lookup;}
        }
      }
    }
    // Do not enter the fallback resource until after lookup. Node starts a fresh
    // annotation scope, and all success/failure/trial paths restore resources.
    final child=node(target,value,path);
    if(child.valid)merge(local,child.annotations,check.source,path);
    return child.valid;
  }
}
