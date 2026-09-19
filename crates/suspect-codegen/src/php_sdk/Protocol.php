<?php
declare(strict_types=1);

namespace __NAMESPACE__;

/** Actual in-memory bytes. No implicit filesystem access, encoding, or JSON stand-in. */
final readonly class Bytes
{
    public function __construct(public string $value) {}
    public function length(): int { return strlen($this->value); }
}

/** An HTTP message with a protocol-forbidden body. */
enum NoBody { case Value; }

/** API key attachment uses the source's explicit header/query/cookie location. */
final readonly class ApiKeyCredential
{
    public function __construct(#[\SensitiveParameter] public string $value)
    {
        if ($value==='' || strlen($value)>8192 || preg_match('/[\x00-\x1f\x7f]/',$value)===1) { throw new SdkError('credentials','invalid API key bytes'); }
    }
    /** @return array<string,string> */
    public function __debugInfo(): array { return ['credential'=>'[redacted]']; }
}

/** A caller-produced Authorization value for OAuth/OIDC; no acquisition or refresh. */
final readonly class AuthorizationCredential
{
    public function __construct(#[\SensitiveParameter] public string $value)
    {
        if ($value==='' || strlen($value)>8192 || preg_match('/[\x00-\x1f\x7f]/',$value)===1) { throw new SdkError('credentials','invalid Authorization credential'); }
    }
    /** @return array<string,string> */
    public function __debugInfo(): array { return ['credential'=>'[redacted]']; }
}

/** Caller-supplied Basic credentials; token acquisition is outside this SDK. */
final readonly class BasicCredential
{
    public function __construct(#[\SensitiveParameter] public string $username, #[\SensitiveParameter] public string $password)
    {
        if (str_contains($username, ':')) { throw new SdkError('credentials', 'Basic username cannot contain a colon'); }
        if (strlen($username)+strlen($password)>8192 || preg_match('/[\x00-\x1f\x7f]/',$username.$password)===1) { throw new SdkError('credentials','Basic credential exceeds its bounded character profile'); }
    }
    /** @return array<string,string> */
    public function __debugInfo(): array { return ['credentials' => '[redacted]']; }
}

/** Immutable source metadata passed to a caller's OAuth/OIDC credential hook. */
final readonly class CredentialRequest
{
    /** @param list<string> $permissions */
    public function __construct(public string $operationId, public string $scheme, public string $kind, public array $permissions, public JsonValue $metadata, public ?string $serverUrl = null) {}
}

/** Checked parameter/part value after native model conversion. @internal */
final readonly class WireValue
{
    public function __construct(public string $name, public JsonValue $value) {}
}

/** Explicit selected request representation, already converted by typed codecs. @internal */
final readonly class PayloadValue
{
    /** @param JsonValue|Bytes|array<array-key, JsonValue|Bytes|PartValue|list<PartValue>> $value
     * @param array<array-key, JsonValue|Bytes|PartValue|list<PartValue>> $parts
     */
    public function __construct(public string $selection, public string $contentType, public JsonValue|Bytes|array $value, public array $parts = []) {}
}

/** Multipart payload metadata. Generated per-part subclasses retain the native value type. @internal */
final readonly class PartValue
{
    /** @param array<array-key,string> $headers */
    public function __construct(public JsonValue|Bytes $value, public ?string $filename = null, public ?string $contentType = null, public array $headers = []) {}
}

/** Source Link metadata, with no automatic invocation or expression evaluation. */
final readonly class Link
{
    public function __construct(public string $name, public JsonValue $metadata) {}
}

/** Interpreter for lowered HTTP descriptors, never OpenAPI or schema text. @internal */
final class Protocol
{
    public static function get(JsonValue $value, string $key): JsonValue
    {
        return $value->asObject()[$key] ?? throw new SdkError('protocol', 'missing generated protocol operand');
    }
    public static function text(JsonValue $value, string $key): string { return self::get($value,$key)->asString(); }
    public static function flag(JsonValue $value, string $key): bool { return self::get($value,$key)->asBool(); }
    public static function integer(JsonValue $value, string $key): int { return self::get($value,$key)->asNumber()->toInt(); }
    /** @return list<JsonValue> */
    public static function list(JsonValue $value, string $key): array { return self::get($value,$key)->asArray(); }
    public static function optional(JsonValue $value, string $key): ?JsonValue
    {
        $item=$value->asObject()[$key] ?? null;
        return $item?->kind === JsonKind::Null ? null : $item;
    }

    /** Attach one satisfiable source alternative, atomically preserving AND semantics.
     * @param array<array-key,string> $headers
     * @param list<string> $query
     * @param list<string> $cookies
     */
    public static function security(JsonValue $plan, Credentials $credentials, string $operation, ?int $selection, array &$headers, array &$query, array &$cookies, ?string $serverUrl = null): void
    {
        if (self::text($plan,'kind')!=='alternatives') {
            if ($selection!==null) { throw new SdkError('credentials','operation has no security alternatives to select'); }
            return;
        }
        $alternatives=self::list($plan,'alternatives');$chosen=null;
        foreach($alternatives as$i=>$alternative){
            if($selection!==null&&$i!==$selection){continue;}
            $available=true;foreach(self::list($alternative,'requirements') as$requirement){if(!$credentials->has(self::text($requirement,'name'))){$available=false;}}
            if($available){$chosen=$alternative;break;}
        }
        if($chosen===null){throw new SdkError('credentials','no complete source security alternative has credentials');}
        $attach=[];$queries=[];$cookieValues=[];
        foreach(self::list($chosen,'requirements') as$requirement){
            $name=self::text($requirement,'name');$hook=self::get($requirement,'credential');$kind=self::text($hook,'kind');
            $permissions=[];foreach(self::list(self::get($requirement,'permissions'),'names') as$p){$permissions[]=self::text($p,'value');}
            $request=new CredentialRequest($operation,$name,$kind,$permissions,$requirement,$serverUrl);
            $value=$credentials->resolve($request);
            if($kind==='bearer'){
                if(!is_string($value)){throw new SdkError('credentials','bearer scheme requires a token');}
                $header='authorization';$content='Bearer '.$value;
            }elseif($kind==='basic'){
                if(!$value instanceof BasicCredential){throw new SdkError('credentials','Basic scheme requires BasicCredential');}
                $header='authorization';$content='Basic '.base64_encode($value->username.':'.$value->password);
            }elseif($kind==='api-key'){
                if(!$value instanceof ApiKeyCredential){throw new SdkError('credentials','API key scheme requires ApiKeyCredential');}
                $key=self::text(self::get($hook,'name'),'value');$location=self::text($hook,'location');
                if($location==='query'){$queries[]=rawurlencode($key).'='.rawurlencode($value->value);continue;}
                if($location==='cookie'){$cookieValues[]=self::encode($key,'none','cookie').'='.self::encode($value->value,'none','cookie');continue;}
                $header=strtolower($key);$content=$value->value;
            }else{
                if(!$value instanceof AuthorizationCredential){throw new SdkError('credentials','OAuth/OIDC hook must supply AuthorizationCredential');}
                $header='authorization';$content=$value->value;
            }
            Wire::headerField($header,$content);
            if(isset($attach[$header])){throw new SdkError('credentials','conjunctive credentials conflict on a header');}
            $attach[$header]=$content;
        }
        $existing=[];foreach($headers as$key=>$_){$existing[strtolower((string)$key)]=true;}
        foreach($attach as$key=>$value){if(isset($existing[$key])){throw new SdkError('credentials','credential conflicts with request header');}$headers[$key]=$value;}
        foreach($queries as$part){$key=explode('=',$part,2)[0];foreach($query as$present){foreach(explode('&',$present) as$pair){if(explode('=',$pair,2)[0]===$key){throw new SdkError('credentials','credential conflicts with query parameter');}}}$query[]=$part;}
        foreach($cookieValues as$part){$key=explode('=',$part,2)[0];foreach($cookies as$present){if(explode('=',$present,2)[0]===$key){throw new SdkError('credentials','credential conflicts with cookie parameter');}}$cookies[]=$part;}
    }

    /** Resolve one explicit candidate/variable assignment against a caller base.
     * @param array<array-key,string> $variables
     */
    public static function server(JsonValue $servers, int $index, array $variables, ?string $base): string
    {
        $candidates=self::list($servers,'candidates');
        $candidate=$candidates[$index] ?? throw new SdkError('configuration','server selection is out of range');
        $url=self::text($candidate,'template'); $declared=[];
        foreach (self::list($candidate,'variables') as $variable) {
            $name=self::text($variable,'name'); $declared[$name]=true;
            $value=$variables[$name] ?? self::text(self::get($variable,'default'),'value');
            if (!is_string($value)) { throw new SdkError('configuration','server variables require strings'); }
            $allowed=self::optional($variable,'values');
            if ($allowed !== null) {
                $found=false;
                foreach ($allowed->asArray() as $choice) { $found=$found || self::text($choice,'value') === $value; }
                if (!$found) { throw new SdkError('configuration','server variable is outside its source enum'); }
            }
            if (preg_match('/[\x00-\x20\x7f{}\\\\]/',$value)===1) { throw new SdkError('configuration','invalid server variable bytes'); }
            $url=str_replace('{'.$name.'}',$value,$url);
        }
        foreach ($variables as $key=>$_) { if (!isset($declared[$key])) { throw new SdkError('configuration','unknown server variable'); } }
        if (strpbrk($url,'?#')!==false) { throw new SdkError('configuration','server URL cannot contain query or fragment'); }
        if (!preg_match('~\Ahttps?://~i',$url)) {
            if($base===null){
                $location=self::get($candidate,'document_base');
                $document=self::text(self::get($location,'source'),'document');if(preg_match('~\Ahttps?://~i',$document)===1){$base=$document;}
            }
            if ($base===null) { throw new SdkError('configuration','relative server requires an explicit absolute base URL'); }
            Wire::serverOrigin($base); $parts=parse_url($base);
            if ($parts===false || !isset($parts['scheme'],$parts['host'])) { throw new SdkError('configuration','invalid base URL'); }
            $authority=$parts['scheme'].'://'.$parts['host'].(isset($parts['port'])?':'.$parts['port']:'');
            if (str_starts_with($url,'//')) { $url=$parts['scheme'].':'.$url; }
            elseif (str_starts_with($url,'/')) { $url=$authority.$url; }
            else { $path=$parts['path']??'/';if($path===''){$path='/';} $slash=strrpos($path,'/'); $url=$authority.substr($path,0,$slash===false?0:$slash+1).$url; }
        }
        Wire::serverOrigin($url);$parsed=parse_url($url); if ($parsed===false || !isset($parsed['scheme'],$parsed['host'])) { throw new SdkError('configuration','invalid resolved server'); }
        $segments=[];$path=explode('/',$parsed['path']??'');$last=count($path)-1;
        foreach($path as$i=>$segment){
            if($segment==='.'||$segment==='..'){
                if($segment==='..'&&count($segments)>1){array_pop($segments);}
                if($i===$last){$segments[]='';}
            }else{$segments[]=$segment;}
        }
        $url=$parsed['scheme'].'://'.$parsed['host'].(isset($parsed['port'])?':'.$parsed['port']:'').implode('/',$segments);
        Wire::server($url); return $url;
    }

    public static function scalar(JsonValue $value, ?string $type = null): string
    {
        if ($value->kind===JsonKind::String && ($type===null||$type==='string')) { return $value->asString(); }
        if ($value->kind===JsonKind::Boolean && ($type===null||$type==='boolean')) { return $value->asBool()?'true':'false'; }
        if ($value->kind===JsonKind::Number && ($type===null||$type==='integer'||$type==='number')) {
            if ($type==='integer' && !$value->asNumber()->isInteger()) { throw new SdkError('request_validation','wire integer is fractional'); }
            return $value->asNumber()->token;
        }
        throw new SdkError('request_validation','value does not match the declared flat wire shape');
    }

    public static function encode(string $value, string $encoding, string $location, ?string $style = null, bool $composite = false): string
    {
        if ($style==='spaceDelimited' && str_contains($value,' ') || $style==='pipeDelimited' && str_contains($value,'|') || $style==='deepObject' && strpbrk($value,'[]')!==false) {
            throw new SdkError('request_validation','style delimiter inside data needs source-defined escaping');
        }
        if ($encoding==='none') {
            if (preg_match($location==='header'?'/[\x00-\x08\x0a-\x1f\x7f]/':'/[\x00-\x1f\x7f]/',$value)===1) { throw new SdkError('request_validation','control character in wire data'); }
            if ($location==='cookie' && preg_match('/[\x20\x09",;\\\\\x80-\xff]/',$value)===1) { throw new SdkError('request_validation','cookie value requires caller escaping'); }
            return $value;
        }
        if ($encoding==='form-url-encoded') { return str_replace('%2A','*',urlencode($value)); }
        if ($encoding!=='reserved-expansion') { return rawurlencode($value); }
        $hazards=match($location) {'path'=>'#[]/?','query','querystring'=>'#[]&=+','cookie'=>';,',default=>''};
        if ($composite) { $hazards.=match($style) {'simple','form','cookie'=>',','label'=>'.,','matrix'=>';,',default=>''}; }
        if ($hazards!=='' && strpbrk($value,$hazards)!==false) { throw new SdkError('request_validation','reserved expansion needs caller escaping'); }
        $out='';
        for ($i=0,$length=strlen($value);$i<$length;++$i) {
            $c=$value[$i];
            if ($c==='%' && $i+2<$length && ctype_xdigit(substr($value,$i+1,2))) { $out.=substr($value,$i,3); $i+=2; }
            elseif (str_contains(":/?#[]@!$&'()*+,;=",$c)) { $out.=$c; }
            else { $out.=rawurlencode($c); }
        }
        return $out;
    }

    public static function parameter(JsonValue $plan, JsonValue $value, string $location, string $name, ?CallControl $control=null): string
    {
        $mediaPlan=self::optional($plan,'content_media');
        if($mediaPlan!==null&&self::text(self::get($mediaPlan,'representation'),'kind')==='form'){return Parts::queryForm(self::get($mediaPlan,'representation'),$value,$control);}
        $serialization=self::get($plan,'serialization'); $encoding=self::text($serialization,'percent_encoding');
        if (self::text($serialization,'kind')==='content') {
            $media=self::get($serialization,'media_type');
            $type=self::text($media,'declared');
            $essence=strtolower(trim(explode(';',$type)[0]));
            $text=($essence==='application/json'||str_ends_with($essence,'+json')) ? $value->toJson() : self::scalar($value);
            $text=self::encode($text,$encoding,$location);
            return in_array($location,['query','cookie'],true)?rawurlencode($name).'='.$text:$text;
        }
        $style=self::text($serialization,'style'); $explode=self::flag($serialization,'explode');
        $shape=self::get($serialization,'shape'); $kind=self::text($shape,'kind');
        $encode=static fn(string $v):string=>self::encode($v,$encoding,$location,$style,$kind!=='scalar');
        $name=$location==='header'||$style==='cookie'?$name:rawurlencode($name);
        $scalar=null; $items=[]; $pairs=[];
        if ($kind==='scalar') { $scalar=$encode(self::scalar($value,self::text($shape,'scalar'))); }
        elseif ($kind==='array') {
            foreach ($value->asArray() as $item) { $items[]=$encode(self::scalar($item,self::text($shape,'items'))); }
            if ($items===[]) { throw new SdkError('request_validation','empty composite has no source-defined expansion'); }
        } else {
            $members=$value->asObject(); ksort($members,SORT_STRING);
            if ($members===[]) { throw new SdkError('request_validation','empty composite has no source-defined expansion'); }
            $properties=self::get($shape,'properties')->asObject(); $additional=self::get($shape,'additional');
            foreach ($members as $key=>$item) {
                $type=isset($properties[$key])?$properties[$key]->asString():null;
                if ($type===null && self::text($additional,'kind')==='forbidden') { throw new SdkError('request_validation','undeclared wire property'); }
                if ($type===null && self::text($additional,'kind')==='typed') { $type=self::text($additional,'scalar'); }
                $pairs[]=[$encode((string)$key),$encode(self::scalar($item,$type))];
            }
        }
        $flat=static function(string $separator) use($pairs):string { $parts=[];foreach($pairs as[$k,$v]){$parts[]=$k;$parts[]=$v;}return implode($separator,$parts); };
        $joined=static fn(string $separator):string=>implode($separator,array_map(static fn(array $p):string=>$p[0].'='.$p[1],$pairs));
        $named=static fn(string $n,string $v):string=>$v===''?$n:$n.'='.$v;
        return match($style) {
            'simple'=>$scalar??($kind==='array'?implode(',',$items):($explode?$joined(','):$flat(','))),
            'label'=>'.'.($scalar??($kind==='array'?implode($explode?'.':',',$items):($explode?$joined('.'):$flat(',')))),
            'matrix'=>$scalar!==null?';'.$named($name,$scalar):($kind==='array'?($explode?implode('',array_map(static fn(string $v):string=>';'.$named($name,$v),$items)):';'.$name.'='.implode(',',$items)):($explode?implode('',array_map(static fn(array $p):string=>';'.$named($p[0],$p[1]),$pairs)):';'.$name.'='.$flat(','))),
            'form','cookie'=>$scalar!==null?$name.'='.$scalar:($kind==='array'?($explode?implode($style==='cookie'?'; ':'&',array_map(static fn(string $v):string=>$name.'='.$v,$items)):$name.'='.implode(',',$items)):($explode?$joined($style==='cookie'?'; ':'&'):$name.'='.$flat(','))),
            'spaceDelimited','pipeDelimited'=>$name.'='.($kind==='array'?implode($style==='spaceDelimited'?'%20':'%7C',$items):$flat($style==='spaceDelimited'?'%20':'%7C')),
            'deepObject'=>implode('&',array_map(static fn(array $p):string=>$name.'%5B'.$p[0].'%5D='.$p[1],$pairs)),
            default=>throw new SdkError('protocol','unknown admitted parameter style'),
        };
    }

    /** Parse a concrete HTTP media type without dropping parameters or duplicates.
     * @return array{string,array<array-key,string>}
     */
    public static function media(string $value): array
    {
        Wire::headerField('content-type',$value);
        if (preg_match('/\A[ \t]*([a-zA-Z0-9!#$%&\'*+.^_`|~-]+\/[a-zA-Z0-9!#$%&\'*+.^_`|~-]+)[ \t]*/',$value,$m)!==1 || str_contains($m[1],'*')) { throw new SdkError('unexpected_media','invalid concrete Content-Type'); }
        $at=strlen($m[0]);$length=strlen($value);$params=[];
        while($at<$length) {
            if(preg_match('/\G;[ \t]*([!#$%&\'*+.^_`|~0-9A-Za-z-]+)[ \t]*=[ \t]*/A',$value,$m,0,$at)!==1){throw new SdkError('unexpected_media','invalid media parameter');}
            $at+=strlen($m[0]);$name=strtolower($m[1]);if(isset($params[$name])){throw new SdkError('unexpected_media','duplicate media parameter');}
            if(($value[$at]??'')==='"'){$at++;$part='';$closed=false;while($at<$length){$c=$value[$at++];if($c==='"'){$closed=true;break;}if($c==='\\'){if($at===$length){break;}$c=$value[$at++];}$part.=$c;}if(!$closed){throw new SdkError('unexpected_media','unterminated media parameter');}}
            else {if(preg_match('/\G([!#$%&\'*+.^_`|~0-9A-Za-z-]+)/A',$value,$m,0,$at)!==1){throw new SdkError('unexpected_media','invalid media parameter value');}$part=$m[1];$at+=strlen($m[0]);}
            $params[$name]=$part;while($at<$length&&str_contains(" \t",$value[$at])){$at++;}
        }
        return [strtolower(trim(explode(';',$value)[0])),$params];
    }

    public static function matchMedia(JsonValue $media, string $actual): int
    {
        [$essence,$params]=self::media($actual);$winner=-1;$score=-1;
        foreach($media->asArray() as $i=>$entry){$definition=self::get($entry,'media_type');$range=self::get($definition,'range');$kind=self::text($range,'kind');
            $matches=match($kind){'any'=>true,'type'=>str_starts_with($essence,self::text($range,'type_name').'/'),'concrete'=>$essence===self::text($range,'type_name').'/'.self::text($range,'subtype'),default=>false};
            $required=self::get($definition,'parameters')->asObject();foreach($required as$k=>$v){$matches=$matches&&isset($params[$k])&&($k==='charset'?strtolower($params[$k])===strtolower($v->asString()):$params[$k]===$v->asString());}
            $rank=($kind==='any'?0:($kind==='type'?1:2))*10000+count($required);if($matches&&$rank>$score){$winner=$i;$score=$rank;}
        }
        if($winner<0){throw new SdkError('unexpected_media','Content-Type is not declared');}
        $chosen=$media->asArray()[$winner];$kind=self::text(self::get($chosen,'representation'),'kind');
        if(in_array($kind,['text','stream','form'],true)&&isset($params['charset'])&&strtolower($params['charset'])!=='utf-8'){throw new SdkError('unexpected_media','unsupported response charset');}
        return $winner;
    }
    public static function matchStatus(JsonValue $responses, int $status): int
    {
        $winner=-1;$rank=0;
        foreach($responses->asArray() as$i=>$response){$rule=self::get($response,'status');$kind=self::text($rule,'kind');$score=match($kind){'exact'=>self::integer($rule,'value')===$status?3:0,'range'=>self::integer($rule,'value')===intdiv($status,100)?2:0,'default'=>1,default=>0};if($score>$rank){$rank=$score;$winner=$i;}}
        if($winner<0){throw new SdkError('unexpected_status','actual status is not declared');}return $winner;
    }

    public static function capture(HttpResponse|StreamResponse $response, CallContext $call):ResponseCapture
    {
        return $response instanceof StreamResponse?$response->capture($call->maxCaptureBytes,control:$call->control):$response->capture($call->maxCaptureBytes);
    }

    public static function parseScalar(string $value, string $type): JsonValue
    {
        return match($type){
            'string'=>JsonValue::fromString($value),
            'boolean'=>match($value){'true'=>JsonValue::fromBool(true),'false'=>JsonValue::fromBool(false),default=>throw new JsonError('conversion','invalid wire boolean')},
            'integer','number'=>JsonValue::fromNumber(JsonNumber::fromString($value)),
            default=>throw new SdkError('protocol','unknown scalar codec representation'),
        };
    }
    /** Bounded request-item encoding. The caller owns the input iterable.
     * @template T
     * @param iterable<T> $items
     * @param \Closure(T):JsonValue $encode
     */
    public static function encodeItems(iterable $items, \Closure $encode, string $framing, int $maxItem, CodecContext $context, int $maxBytes = RuntimeConfig::MAX_REQUEST_BYTES):Bytes
    {
        if($maxBytes<0||$maxBytes>RuntimeConfig::MAX_REQUEST_BYTES){throw new JsonError('resource_limit','invalid request stream ceiling');}
        $out='';$count=0;
        foreach($items as$item){
            $context->control?->check();if(++$count>RuntimeConfig::MAX_NODES){throw new JsonError('resource_limit','too many request stream items');}
            $frameLimit=min($maxItem,$maxBytes-strlen($out));if($frameLimit<1){throw new JsonError('resource_limit','request stream byte ceiling exceeded');}
            $json=$encode($item);
            if($framing==='json-lines'){$frame=$json->toJson(new JsonLimits(maxBytes:min($frameLimit,RuntimeConfig::MAX_JSON_BYTES),control:$context->control))."\n";}
            else{
                $members=$json->asObject();foreach($members as$key=>$_){if(!in_array((string)$key,['data','event','id','retry'],true)){throw new JsonError('conversion','undefined request SSE envelope field');}}
                $data=($members['data']??throw new JsonError('conversion','SSE data is required'))->asString();
                if(strlen($data)>$frameLimit){throw new JsonError('resource_limit','request SSE data exceeds frame ceiling');}
                if(str_contains($data,"\r")){throw new JsonError('conversion','SSE data cannot preserve a carriage return');}
                $frame='';foreach(['event','id'] as$key){if(isset($members[$key])){$value=$members[$key]->asString();if(strpbrk($value,"\r\n\0")!==false){throw new JsonError('conversion','invalid SSE field bytes');}$frame.=$key.': '.$value."\n";}}
                if(isset($members['retry'])){$retry=$members['retry']->asNumber();if(!$retry->isInteger()||$retry->compare(JsonNumber::fromInt(0))<0){throw new JsonError('conversion','invalid SSE retry');}$frame.='retry: '.$retry->toDecimalString(min($maxItem,65536))."\n";}
                foreach(explode("\n",$data) as$line){if(strlen($frame)+strlen($line)+8>$frameLimit){throw new JsonError('resource_limit','request SSE frame exceeds ceiling');}$frame.='data: '.$line."\n";}$frame.="\n";
            }
            if(strlen($frame)>$frameLimit){throw new JsonError('resource_limit','request stream byte ceiling exceeded');}
            $out.=$frame;
        }
        return new Bytes($out);
    }
    /** @param list<string> $values */
    public static function header(JsonValue $header, array $values, string $scalarHint): JsonValue
    {
        $wire=implode(', ',$values);$serialization=self::get($header,'serialization');
        if(self::text($serialization,'kind')==='content'){
            $media=strtolower(trim(explode(';',self::text(self::get($serialization,'media_type'),'declared'))[0]));
            return $media==='application/json'||str_ends_with($media,'+json')?JsonValue::parse($wire):self::parseScalar($wire,$scalarHint);
        }
        $shape=self::get($serialization,'shape');$kind=self::text($shape,'kind');
        if($kind==='scalar'){return self::parseScalar($wire,self::text($shape,'scalar'));}
        $values=array_map('trim',explode(',',$wire));
        if($kind==='array'){$items=[];foreach($values as$value){$items[]=self::parseScalar($value,self::text($shape,'items'));}return JsonValue::fromArray($items);}
        $members=[];$pairs=[];
        if(self::flag($serialization,'explode')){foreach($values as$value){$pair=explode('=',$value,2);if(count($pair)!==2){throw new JsonError('conversion','malformed exploded object header');}$pairs[]=[$pair[0],$pair[1]];}}
        else{if(count($values)%2!==0){throw new JsonError('conversion','malformed object header');}for($i=0;$i<count($values);$i+=2){$pairs[]=[$values[$i],$values[$i+1]];}}
        $properties=self::get($shape,'properties')->asObject();$additional=self::get($shape,'additional');
        foreach($pairs as[$name,$value]){
            if(array_key_exists($name,$members)){throw new JsonError('conversion','duplicate object header member');}
            $type=isset($properties[$name])?$properties[$name]->asString():null;
            if($type===null){if(self::text($additional,'kind')!=='typed'){throw new JsonError('conversion','object header extras require an explicit scalar type');}$type=self::text($additional,'scalar');}
            $members[$name]=self::parseScalar($value,$type);
        }
        return JsonValue::fromObject($members);
    }

    /** Validate one caller-supplied application identifier: `<name>` or `<name>/<version>`
     * of RFC 9110 tokens, at most 128 bytes. */
    private static function applicationIdentity(string $value): bool
    {
        return strlen($value) <= 128 && preg_match('/\A[A-Za-z0-9!#$%&\'*+.^_`|~-]+(?:\/[A-Za-z0-9!#$%&\'*+.^_`|~-]+)?\z/', $value) === 1;
    }

    /** ua/v1 attribution: an explicit caller User-Agent wins entirely, an explicit
     * empty value suppresses the header, and the automatic default identifies
     * suspect as the generator and the SDK package or a caller-supplied application
     * as the client. Invalid application identifiers degrade to no header rather
     * than a malformed User-Agent. */
    public static function userAgent(ClientOptions $options): ?string
    {
        if ($options->userAgent !== null) { return $options->userAgent === '' ? null : $options->userAgent; }
        $version=RuntimeConfig::ATTRIBUTION_SUSPECT_VERSION;
        if ($version==='') { return null; }
        $identity=RuntimeConfig::ATTRIBUTION_SDK_NAME.'/'.RuntimeConfig::ATTRIBUTION_SDK_VERSION;
        if ($options->applicationId!==null && $options->applicationId!=='') {
            if (!self::applicationIdentity($options->applicationId)) { return null; }
            $identity=$options->applicationId;
        }
        return 'suspect/'.$version.' '.$identity.' ('.RuntimeConfig::ATTRIBUTION_LANGUAGE.'/'.\PHP_VERSION.'; openapi/'.RuntimeConfig::ATTRIBUTION_SPEC_VERSION.')';
    }

    /** Execute prepared native values using one source-selected exchange.
     * @param array<array-key,JsonValue> $values
     * @return ($stream is true ? StreamResponse : HttpResponse)
     */
    public static function exchange(JsonValue $operation, Credentials $credentials, Transport $transport, ClientOptions $options, ?RequestOptions $requestOptions, CallContext $call, array $values, ?PayloadValue $payload, bool $stream = false): HttpResponse|StreamResponse
    {
        $call->check();
        $server=$options->serverUrl ?? self::server(self::get($operation,'servers'),$options->serverIndex,$options->serverVariables,$options->serverBaseUrl);
        $path=self::text($operation,'path');$query=[];$cookies=[];$headers=[];$querystring=null;
        foreach(self::list($operation,'parameters') as$parameter){
            $name=self::text($parameter,'name');$location=self::text($parameter,'location');
            $value=$values[$location.':'.$name]??null;
            if($value===null){if(self::flag($parameter,'required')){throw new SdkError('request_validation','required parameter is absent');}continue;}
            $encoded=self::parameter($parameter,$value,$location,$name,$call->control);
            if(strlen($encoded)>$options->maxRequestBytes){throw new SdkError('resource_limit','encoded parameter exceeds request ceiling');}
            switch($location){
                case 'path': $encoded=$encoded==='.'||$encoded==='..'?str_replace('.','%2E',$encoded):$encoded;$path=str_replace('{'.$name.'}',$encoded,$path);break;
                case 'query': $query[]=$encoded;break;
                case 'querystring': $querystring=$encoded;break;
                case 'header': Wire::headerField($name,$encoded);$key=strtolower($name);if(isset($headers[$key])){throw new SdkError('request_validation','duplicate header');}$headers[$key]=$encoded;break;
                case 'cookie': $cookies[]=$encoded;break;
                default: throw new SdkError('protocol','unknown parameter location');
            }
            $call->check();
        }
        self::security(self::get($operation,'security'),$credentials,self::text(self::get($operation,'operation_id'),'value'),$requestOptions?->securityAlternative,$headers,$query,$cookies,$server);
        if($querystring!==null){if($query!==[]){throw new SdkError('request_validation','whole querystring conflicts with named query data');}$query[]=$querystring;}
        if($cookies!==[]){if(isset($headers['cookie'])){throw new SdkError('request_validation','cookie header conflict');}$headers['cookie']=implode('; ',$cookies);}
        $body=null;$bodyPlan=self::optional($operation,'body');
        if($payload!==null){
            if($bodyPlan===null){throw new SdkError('request_validation','operation declares no request body');}
            $maxBodyBytes=min($options->maxRequestBytes,self::integer(self::get($bodyPlan,'limits'),'body'));
            $index=self::matchMedia(self::get($bodyPlan,'media'),$payload->contentType);
            if((string)$index!==$payload->selection){throw new SdkError('request_validation','representation selector cannot bypass a more specific media declaration');}
            $media=self::list($bodyPlan,'media')[$index];$representation=self::get($media,'representation');$kind=self::text($representation,'kind');
            if($kind==='json'){
                if(!$payload->value instanceof JsonValue){throw new SdkError('request_validation','JSON representation requires checked JSON');}
                if($maxBodyBytes<1){throw new SdkError('resource_limit','declared request byte ceiling is zero');}
                $body=$payload->value->toJson(new JsonLimits(maxBytes:$maxBodyBytes,control:$call->control));
            }elseif($kind==='text'){
                if(!$payload->value instanceof JsonValue){throw new SdkError('request_validation','text representation requires checked scalar');}
                $body=self::scalar($payload->value);
                [, $params]=self::media($payload->contentType);if(isset($params['charset'])&&strtolower($params['charset'])!=='utf-8'){throw new SdkError('request_validation','unsupported text charset');}
            }elseif($kind==='binary'||$kind==='stream'){
                if(!$payload->value instanceof Bytes){throw new SdkError('request_validation','binary representation requires Bytes');}
                $body=$payload->value->value;
                if($kind==='binary'&&strlen($body)>self::integer(self::get($representation,'bytes'),'max_bytes')){throw new SdkError('resource_limit','declared byte policy exceeded');}
            }elseif($kind==='form'||$kind==='multipart'){
                $partValues=[];
                foreach($payload->parts as$name=>$values){
                    if($values instanceof PartValue){$partValues[$name]=[$values];}
                    elseif(is_array($values)){foreach($values as$v){if(!$v instanceof PartValue){throw new JsonError('conversion','invalid native part list');}}$partValues[$name]=$values;}
                    else{throw new JsonError('conversion','invalid native body parts');}
                }
                [$body,$contentType]=Parts::encode($representation,$partValues,$payload->contentType,$maxBodyBytes,$options->maxHeaderBytes,$call->control);
            }else{throw new SdkError('protocol','request representation is not enabled by this runtime');}
            $headers['content-type']=$contentType??$payload->contentType;
            if(strlen($body)>$maxBodyBytes){throw new SdkError('resource_limit','declared request byte ceiling exceeded');}
        }elseif($bodyPlan!==null&&self::flag($bodyPlan,'required')){throw new SdkError('request_validation','required request body absent');}
        if(!isset($headers['accept'])){$media=[];foreach(self::list($operation,'responses') as$r){foreach(self::list($r,'media') as$m){$media[]=self::text(self::get($m,'media_type'),'declared');}}$headers['accept']=$media===[]?'*/*':implode(', ',array_unique($media));}
        $headers['accept-encoding']='identity';
        // ua/v1 attribution is applied last so an explicit caller-supplied
        // User-Agent header parameter keeps precedence over the automatic value.
        $userAgent=self::userAgent($options);
        if($userAgent!==null&&!isset($headers['user-agent'])){$headers['user-agent']=$userAgent;}
        $url=rtrim($server,'/').$path.($query===[]?'':'?'.implode('&',$query));
        if(strlen($url)>$options->maxRequestBytes||($body!==null&&strlen($body)>$options->maxRequestBytes)){throw new SdkError('resource_limit','request exceeds byte ceiling');}
        Wire::headers($headers,$options->maxHeaderBytes);Wire::serverOrigin($url);
        $request=new HttpRequest(self::text($operation,'method'),$url,$headers,$body,$call->remainingMilliseconds(),$call->maxResponseBytes,$options->maxHeaderBytes,$call->maxCaptureBytes,$call->control);
        if($stream){
            if(!$transport instanceof StreamTransport){throw new SdkError('transport','this operation requires an explicit StreamTransport');}
            try{$opened=$transport->open($request);}catch(SdkError $e){throw $e;}catch(\Throwable $e){throw new SdkError('transport','stream transport failed',previous:$e);}
            try{$call->check();Wire::headers($opened->headers,$options->maxHeaderBytes);Wire::contentEncoding(new HttpResponse($opened->status,$opened->headers,''),$call->maxCaptureBytes);return $opened;}
            catch(\Throwable $e){try{$opened->close();}catch(\Throwable $cleanup){}throw $e;}
        }
        try{$response=$transport->send($request);}
        catch(SdkError $error){throw $error;}
        catch(\Throwable $error){throw new SdkError('transport','custom transport failed',previous:$error);}
        try{$call->check();}catch(SdkError $error){throw $error->withCapture($response->capture($call->maxCaptureBytes,true));}
        if(strlen($response->body)>$call->maxResponseBytes||$response->headerBytes>$options->maxHeaderBytes){throw new SdkError('resource_limit','response exceeds byte ceiling',$response->capture($call->maxCaptureBytes,true));}
        Wire::contentEncoding($response,$call->maxCaptureBytes);
        return $response;
    }
}
