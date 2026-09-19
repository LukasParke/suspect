<?php
declare(strict_types=1);
namespace __NAMESPACE__;

/** Evaluated locations at one instance level. Every evaluation owns fresh sets. @internal */
final class ValidationAnnotations
{
    /** @var array<array-key,true> */
    public array $properties = [];
    /** @var array<int,true> */
    public array $items = [];
}

/** A completed outcome and its successful locally owned annotations. @internal */
final readonly class ValidationEvaluation
{
    public function __construct(public ?ValidationError $issue = null, public ValidationAnnotations $annotations = new ValidationAnnotations()) {}
}

/** Original compiled pattern and target. @internal */
final readonly class PatternProperty
{
    public function __construct(public string $text, public PatternProgram $program, public int $target) {}
}

/** V2 shares the owning session's work, equality, numeric and active-identity state. @internal */
trait ScopedValidation
{
    private function mergeAnnotations(ValidationAnnotations $into, ValidationAnnotations $from, string $source, string $path): void
    {
        $properties=$from->properties;ksort($properties,SORT_STRING);
        foreach($properties as$name=>$_){$this->spend($source,$path);$into->properties[$name]=true;}
        $items=$from->items;ksort($items,SORT_NUMERIC);
        foreach($items as$index=>$_){$this->spend($source,$path);$into->items[$index]=true;}
    }

    private function evaluateScoped(int $index, JsonValue $value, string $path, int $depth): ValidationEvaluation
    {
        if(!isset($this->nodes[$index])){$this->failure('',$path,'unknown instruction target');}
        $node=$this->nodes[$index];$this->spend($node->source,$path);
        if($depth>=RuntimeConfig::MAX_SCHEMA_DEPTH){$this->failure($node->source,$path,'evaluation depth/nonproductive recursion limit');}
        $entered=$this->enterResource($index,$node->source,$path);
        $identity=$index.':'.spl_object_id($value).':'.$this->resourceContext;
        if(isset($this->active[$identity])){$this->leaveResource($entered);$this->failure($node->source,$path,'evaluation depth/nonproductive recursion limit');}
        $this->active[$identity]=true;
        try{
            $local=new ValidationAnnotations();$first=null;
            foreach($node->checks as$check){
                $this->spend($check->source,$path);
                $result=$this->scopedInstruction($node,$check,$value,$path,$depth,$local);
                $first??=$result->issue;
                if($result->issue===null){$this->mergeAnnotations($local,$result->annotations,$check->source,$path);}
            }
            return new ValidationEvaluation($first,$first===null?$local:new ValidationAnnotations());
        }finally{unset($this->active[$identity]);$this->leaveResource($entered);}
    }

    private function scopedInstruction(ValidationNode $node, ValidationInstruction $check, JsonValue $value, string $path, int $depth, ValidationAnnotations $local): ValidationEvaluation
    {
        $produced=new ValidationAnnotations();$first=null;
        switch($check->op){
            case 'ref': return $this->evaluateScoped($check->target,$value,$path,$depth+1);
            case 'dynamicRef': return $this->evaluateScoped($this->dynamicTarget($check,$path),$value,$path,$depth+1);
            case 'allOf':
            case 'anyOf':
            case 'oneOf':
                $passing=[];$matches=0;
                foreach($check->targets as$target){
                    $this->spend($check->source,$path);$child=$this->evaluateScoped($target,$value,$path,$depth+1);
                    if($child->issue===null){++$matches;$passing[]=$child->annotations;}
                    elseif($check->op==='allOf'){$first??=$child->issue;}
                }
                $valid=match($check->op){'allOf'=>$matches===count($check->targets),'anyOf'=>$matches>0,default=>$matches===1};
                if(!$valid){return new ValidationEvaluation($first??$this->mismatch($check,$path));}
                foreach($passing as$annotations){$this->mergeAnnotations($produced,$annotations,$check->source,$path);}
                break;
            case 'not':
                return new ValidationEvaluation($this->evaluateScoped($check->target,$value,$path,$depth+1)->issue===null?$this->mismatch($check,$path):null);
            case 'if':
                $condition=$this->evaluateScoped($check->condition,$value,$path,$depth+1);
                $target=$condition->issue===null?$check->thenTarget:$check->elseTarget;
                if($condition->issue===null){$this->mergeAnnotations($local,$condition->annotations,$check->source,$path);}
                return $target===null?new ValidationEvaluation():$this->evaluateScoped($target,$value,$path,$depth+1);
            case 'dependentRequired':
                if($value->kind!==JsonKind::Object){break;}$members=$value->asObject();
                foreach($check->requirements as[$trigger,$names]){
                    $this->spend($check->source,$path);
                    if(array_key_exists($trigger,$members)){foreach($names as$name){
                        $this->spend($check->source,$path);
                        if(!array_key_exists($name,$members)){$first??=new ValidationError('invalid',$this->child($check->source,$trigger),$path,'required dependent member is absent');}
                    }}
                }
                break;
            case 'dependentSchemas':
                if($value->kind!==JsonKind::Object){break;}$members=$value->asObject();$passing=[];
                foreach($check->dependencies as[$trigger,$target]){
                    $this->spend($check->source,$path);
                    if(array_key_exists($trigger,$members)){
                        $child=$this->evaluateScoped($target,$value,$path,$depth+1);$first??=$child->issue;
                        if($child->issue===null){$passing[]=$child->annotations;}
                    }
                }
                if($first===null){foreach($passing as$annotations){$this->mergeAnnotations($produced,$annotations,$check->source,$path);}}
                break;
            case 'contains':
                if($value->kind!==JsonKind::Array){break;}$matches=0;
                foreach($value->asArray() as$index=>$item){
                    $this->spend($check->source,$path);
                    if($this->evaluateScoped($check->target,$item,$this->child($path,(string)$index),$depth+1)->issue===null){++$matches;$produced->items[$index]=true;}
                }
                $minimum=$check->containsMinimum;$maximum=$check->containsMaximum;
                $count=JsonNumber::fromInt($matches);
                if($matches!==0||($minimum!==null&&$minimum->compare(JsonNumber::fromInt(0))===0)){$this->mergeAnnotations($local,$produced,$check->source,$path);}
                $produced=new ValidationAnnotations();
                if($minimum===null?$matches<1:$count->compare($this->number($minimum,$check,$path))<0){$first=new ValidationError('invalid',$minimum===null?$check->source:$this->child($node->source,'minContains'),$path,'too few contains matches');}
                if($maximum!==null&&$count->compare($this->number($maximum,$check,$path))>0){$first??=new ValidationError('invalid',$this->child($node->source,'maxContains'),$path,'too many contains matches');}
                break;
            case 'properties':
            case 'additionalProperties':
            case 'additionalPropertiesWithPatterns':
            case 'patternProperties':
            case 'propertyNames':
            case 'unevaluatedProperties':
                if($value->kind!==JsonKind::Object){break;}$members=$value->asObject();ksort($members,SORT_STRING);
                if($check->op==='properties'){
                    foreach($check->properties as[$name,$target]){
                        $this->spend($check->source,$path);
                        if(array_key_exists($name,$members)){
                            $child=$this->evaluateScoped($target,$members[$name],$this->child($path,$name),$depth+1);$first??=$child->issue;$produced->properties[$name]=true;
                        }
                    }
                    break;
                }
                $patterns=$check->patterns;
                if($check->op==='additionalPropertiesWithPatterns'){
                    foreach($node->checks as$sibling){if($sibling->op==='patternProperties'){$patterns=$sibling->patterns;break;}}
                }
                $declared=array_fill_keys($check->declared,true);
                foreach($members as$name=>$item){
                    $this->spend($check->source,$path);$name=(string)$name;$memberPath=$this->child($path,$name);
                    if($check->op==='propertyNames'){
                        $key=JsonValue::fromString($name);$child=$this->evaluateScoped($check->target,$key,$memberPath,$depth+1);$first??=$child->issue;continue;
                    }
                    if($check->op==='unevaluatedProperties'&&isset($local->properties[$name])){continue;}
                    if(in_array($check->op,['additionalProperties','additionalPropertiesWithPatterns'],true)&&isset($declared[$name])){continue;}
                    if($check->op==='patternProperties'){
                        foreach($patterns as$pattern){
                            $this->spend($check->source,$path);
                            if($this->pattern($pattern->program,$name,$check,$path)){$child=$this->evaluateScoped($pattern->target,$item,$memberPath,$depth+1);$first??=$child->issue;$produced->properties[$name]=true;}
                        }
                        continue;
                    }
                    if($check->op==='additionalPropertiesWithPatterns'){
                        foreach($patterns as$pattern){$this->spend($check->source,$path);if($this->pattern($pattern->program,$name,$check,$path)){continue 2;}}
                    }
                    $child=$this->evaluateScoped($check->target,$item,$memberPath,$depth+1);$first??=$child->issue;$produced->properties[$name]=true;
                }
                break;
            case 'items':
            case 'prefixItems':
            case 'unevaluatedItems':
                if($value->kind!==JsonKind::Array){break;}
                foreach($value->asArray() as$index=>$item){
                    if($check->op==='items'&&$index<$check->start){continue;}
                    if($check->op==='prefixItems'&&!isset($check->targets[$index])){break;}
                    $this->spend($check->source,$path);
                    if($check->op==='unevaluatedItems'&&isset($local->items[$index])){continue;}
                    $target=$check->op==='prefixItems'?$check->targets[$index]:$check->target;
                    $child=$this->evaluateScoped($target,$item,$this->child($path,(string)$index),$depth+1);$first??=$child->issue;$produced->items[$index]=true;
                }
                break;
            default: $first=$this->instruction($check,$value,$path,$depth);
        }
        return new ValidationEvaluation($first,$first===null?$produced:new ValidationAnnotations());
    }
}
