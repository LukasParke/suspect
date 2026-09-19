<?php
declare(strict_types=1);
namespace __NAMESPACE__;

/** Indexed logical metadata retains its physical source boundary. @internal */
final readonly class ValidationResource
{
    /** @param list<string> $aliases
     * @param list<array{string,string,int}> $dynamicAnchors
     */
    public function __construct(public string $source, public string $kind, public string $canonicalUri, public string $baseUri, public array $aliases, public ?string $declarationSource, public array $dynamicAnchors) {}
}

/** Guarded v3 metadata, aligned to the immutable node array. @internal */
final readonly class ValidationResources
{
    /** @param list<ValidationResource> $resources
     * @param list<array{int,string,string}> $nodeScopes
     */
    public function __construct(public array $resources, public array $nodeScopes) {}
}

/** First-distinct entered resources and exact context interning, shared across trials. @internal */
trait ResourceValidation
{
    private readonly ?ValidationResources $resources;
    /** @var list<int> */
    private array $resourceStack=[];
    /** @var array<int,true> */
    private array $enteredResources=[];
    /** @var array<string,int> */
    private array $resourceContexts=[];
    private int $resourceContext=0;

    /** @param class-string $program */
    private function resourceMetadata(string $version,string $program):?ValidationResources
    {
        if($version!=='suspect.validation.experimental.v3'){return null;}
        $factory=[$program,'resources'];
        if(!is_callable($factory)){$this->failure('','','v3 resource metadata is missing');}
        $resources=$factory();
        if(!$resources instanceof ValidationResources){$this->failure('','','invalid native resource metadata');}
        return $resources;
    }

    private function enterResource(int $node,string $source,string $path):?int
    {
        if($this->resources===null){return null;}
        $scope=$this->resources->nodeScopes[$node]??null;
        if($scope===null||!isset($this->resources->resources[$scope[0]])){$this->failure($source,$path,'unaligned resource scope');}
        $resource=$scope[0];if(isset($this->enteredResources[$resource])){return null;}
        $this->spend($source,$path);$previous=$this->resourceContext;
        // Exact ordered-pair keys, not a hash-only context approximation.
        $key=$previous.':'.$resource;
        $this->resourceContext=$this->resourceContexts[$key]??=count($this->resourceContexts)+1;
        $this->enteredResources[$resource]=true;$this->resourceStack[]=$resource;
        return $previous;
    }
    private function leaveResource(?int $previous):void
    {
        if($previous===null){return;}
        $resource=array_pop($this->resourceStack);
        if($resource===null){$this->failure('','','resource scope underflow');}
        unset($this->enteredResources[$resource]);$this->resourceContext=$previous;
    }
    private function dynamicTarget(ValidationInstruction $check,string $path):int
    {
        if($this->resources===null){$this->failure($check->source,$path,'dynamic reference requires the v3 profile');}
        if(!isset($this->resources->resources[$check->initialResource])){$this->failure($check->source,$path,'invalid initial dynamic resource');}
        if($check->anchor===null){return $check->target;}
        foreach($this->resourceStack as$index){
            $this->spend($check->source,$path);
            foreach($this->resources->resources[$index]->dynamicAnchors as[$name,$source,$target]){
                $this->spend($check->source,$path);if($name===$check->anchor){return $target;}
            }
        }
        return $check->target;
    }
}
