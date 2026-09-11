<?php
declare(strict_types=1);
namespace __NAMESPACE__;

/** Structural form/MIME rules over native bytes and separately checked part values. @internal */
final class Parts
{
    public static function queryForm(JsonValue $representation,JsonValue $value,?CallControl $control=null):string
    {
        [, $fields,$additional]=self::definition($representation);$parts=[];
        foreach($value->asObject() as$name=>$item){$part=self::field($fields,$additional,(string)$name);$values=Protocol::text($part,'multiplicity')==='repeated-array-items'?$item->asArray():[$item];$parts[$name]=array_map(static fn(JsonValue $v):PartValue=>new PartValue($v),$values);}
        return self::encode($representation,$parts,'application/x-www-form-urlencoded',RuntimeConfig::MAX_REQUEST_BYTES,RuntimeConfig::MAX_HEADER_BYTES,$control??new CallControl(static function():void{}))[0];
    }
    /** @return array{JsonValue,list<JsonValue>,JsonValue,bool} */
    private static function definition(JsonValue $representation): array
    {
        $multipart=Protocol::text($representation,'kind')==='multipart';
        $body=Protocol::get($representation,$multipart?'multipart':'form');
        if($multipart&&Protocol::text($body,'kind')!=='named'){throw new SdkError('protocol','positional multipart is not admitted');}
        return [Protocol::get($body,'rules'),Protocol::list($body,$multipart?'parts':'fields'),Protocol::get($body,'additional'),$multipart];
    }
    /** @param list<JsonValue> $fields */
    private static function field(array $fields,JsonValue $additional,string $name): JsonValue
    {
        foreach($fields as$field){$declared=Protocol::optional($field,'name');if($declared!==null&&$declared->asString()===$name){return $field;}}
        if(Protocol::text($additional,'kind')==='allowed'){return Protocol::get($additional,'part');}
        throw new JsonError('conversion','undeclared body member');
    }
    /** @param array<array-key,list<PartValue>> $parts */
    private static function rules(JsonValue $rules,array $parts):void
    {
        foreach(Protocol::list($rules,'required') as$required){if(!array_key_exists(Protocol::text($required,'value'),$parts)){throw new JsonError('conversion','required body member absent');}}
        $min=Protocol::optional($rules,'min_properties');$max=Protocol::optional($rules,'max_properties');$count=count($parts);
        if($min!==null&&$count<Protocol::integer($min,'value')||$max!==null&&$count>Protocol::integer($max,'value')){throw new JsonError('conversion','body property count outside source bounds');}
    }
    /** @param list<PartValue> $values */
    private static function multiplicity(JsonValue $part,array $values):void
    {
        if(Protocol::text($part,'multiplicity')==='one'){if(count($values)!==1){throw new JsonError('conversion','non-repeated part must occur exactly once');}return;}
        $min=Protocol::optional($part,'min_items');$max=Protocol::optional($part,'max_items');$count=count($values);
        if(Protocol::flag($part,'required')&&$count===0||$min!==null&&$count<Protocol::integer($min,'value')||$max!==null&&$count>Protocol::integer($max,'value')){throw new JsonError('conversion','part count outside source bounds');}
    }
    /** @param array<array-key,list<PartValue>> $parts
     * @return array{string,string}
     */
    public static function encode(JsonValue $representation,array $parts,string $contentType,int $maxBytes,int $maxHeaders,CallControl $control):array
    {
        [$rules,$fields,$additional,$multipart]=self::definition($representation);self::rules($rules,$parts);ksort($parts,SORT_STRING);
        $chunks=[];$bytes=0;
        foreach($parts as$name=>$values){
            $name=(string)$name;$part=self::field($fields,$additional,$name);self::multiplicity($part,$values);
            foreach($values as$value){
                $control->check();$policy=Protocol::get($part,'representation');$kind=Protocol::text($policy,'kind');
                if($kind==='binary'){
                    if(!$value->value instanceof Bytes){throw new JsonError('conversion','binary part requires Bytes');}$data=$value->value->value;
                    if(strlen($data)>Protocol::integer(Protocol::get($policy,'bytes'),'max_bytes')){throw new SdkError('resource_limit','binary part exceeds source byte policy');}
                }else{
                    if(!$value->value instanceof JsonValue){throw new JsonError('conversion','text/JSON part requires a checked value');}
                    $data=$kind==='json'?$value->value->toJson(new JsonLimits(maxBytes:$maxBytes,control:$control)):($kind==='style'?Protocol::parameter(JsonValue::fromObject(['serialization'=>Protocol::get($policy,'serialization')]),$value->value,$multipart?'header':'query',$name):Protocol::scalar($value->value,Protocol::text($policy,'scalar')));
                }
                if(!$multipart){
                    $data=$kind==='style'?$data:Protocol::encode($name,'form-url-encoded','query').'='.Protocol::encode($data,Protocol::text($policy,'outer_encoding'),'query');
                    $bytes+=strlen($data)+1;if($bytes>$maxBytes+1){throw new SdkError('resource_limit','form exceeds body ceiling');}$chunks[]=$data;continue;
                }
                $types=Protocol::list($part,'content_types');
                if($kind==='style'){
                    if($value->contentType!==null){throw new JsonError('conversion','style-encoded parts do not select Content-Type');}
                    $headers=['Content-Disposition'=>'form-data; name='.self::quote($name)];
                    if($value->filename!==null){$headers['Content-Disposition'].='; filename='.self::quote($value->filename);}
                    foreach($value->headers as$key=>$header){if(in_array(strtolower((string)$key),['content-type','content-disposition','content-length','transfer-encoding','content-encoding'],true)){throw new JsonError('conversion','reserved multipart header');}$headers[$key]=$header;}
                    Wire::headers($headers,$maxHeaders);$head='';foreach($headers as$key=>$header){$head.=$key.': '.$header."\r\n";}
                    $chunk=$head."\r\n".$data."\r\n";$bytes+=strlen($chunk);if($bytes>$maxBytes){throw new SdkError('resource_limit','multipart exceeds body ceiling');}$chunks[]=$chunk;continue;
                }
                $type=$value->contentType??($types===[]?'application/octet-stream':Protocol::text($types[0],'declared'));
                Protocol::media($type);$allowed=false;
                foreach($types as$declared){$candidate=JsonValue::fromObject(['media_type'=>$declared,'representation'=>JsonValue::fromObject(['kind'=>JsonValue::fromString('binary')])]);try{Protocol::matchMedia(JsonValue::fromArray([$candidate]),$type);$allowed=true;}catch(SdkError $e){}}
                if(!$allowed){throw new JsonError('conversion','part Content-Type is not declared');}
                $headers=['Content-Disposition'=>'form-data; name='.self::quote($name),'Content-Type'=>$type];
                if($value->filename!==null){$headers['Content-Disposition'].='; filename='.self::quote($value->filename);}
                foreach($value->headers as$key=>$header){if(in_array(strtolower((string)$key),['content-type','content-disposition','content-length','transfer-encoding','content-encoding'],true)){throw new JsonError('conversion','reserved multipart header');}$headers[$key]=$header;}
                Wire::headers($headers,$maxHeaders);$head='';foreach($headers as$key=>$header){$head.=$key.': '.$header."\r\n";}
                $chunk=$head."\r\n".$data."\r\n";$bytes+=strlen($chunk);if($bytes>$maxBytes){throw new SdkError('resource_limit','multipart exceeds body ceiling');}$chunks[]=$chunk;
            }
        }
        if(!$multipart){return [implode('&',$chunks),$contentType];}
        $boundary='sdk-'.bin2hex(random_bytes(16));
        foreach($chunks as$chunk){if(str_contains($chunk,"\r\n--".$boundary)){throw new SdkError('request_validation','multipart boundary collision');}}
        $body='';foreach($chunks as$chunk){$piece='--'.$boundary."\r\n".$chunk;if(strlen($piece)>$maxBytes-strlen($body)){throw new SdkError('resource_limit','multipart framing exceeds ceiling');}$body.=$piece;}
        $end='--'.$boundary."--\r\n";if(strlen($end)>$maxBytes-strlen($body)){throw new SdkError('resource_limit','multipart closing delimiter exceeds ceiling');}$body.=$end;
        [, $parameters]=Protocol::media($contentType);if(isset($parameters['boundary'])){throw new JsonError('conversion','multipart boundary is owned by the encoder');}
        return [$body,$contentType.'; boundary='.$boundary];
    }
    private static function quote(string $value):string
    {
        if(preg_match('/[\x00-\x1f\x7f]/',$value)===1){throw new JsonError('conversion','invalid multipart name/filename');}
        return '"'.str_replace(['\\','"'],['\\\\','\\"'],$value).'"';
    }

    /** @return array<array-key,list<PartValue>> */
    public static function decode(JsonValue $representation,string $body,string $contentType,int $maxHeaders,?CallControl $control=null):array
    {
        [$rules,$fields,$additional,$multipart]=self::definition($representation);$parts=[];
        if(!$multipart){
            $count=0;foreach($body===''?[]:explode('&',$body) as$entry){$control?->check();if(++$count>RuntimeConfig::MAX_NODES){throw new JsonError('resource_limit','too many form fields');}$pair=explode('=',$entry,2);if(count($pair)!==2||preg_match('/%(?![0-9a-fA-F]{2})/',$entry)===1){throw new JsonError('conversion','invalid form field');}$name=urldecode($pair[0]);$value=urldecode($pair[1]);$part=self::field($fields,$additional,$name);$parts[$name][]=self::value($part,$value,null,[],$name,false,$control);}
        }else{
            [, $parameters]=Protocol::media($contentType);$boundary=$parameters['boundary']??throw new JsonError('conversion','multipart response needs boundary');
            if($boundary===''||strlen($boundary)>70||str_ends_with($boundary,' ')||preg_match("/[^0-9A-Za-z'()+_,.\/:=? -]/",$boundary)===1){throw new JsonError('conversion','invalid MIME boundary');}
            // MIME preamble/epilogue are comments. Boundary prefixes in binary data are not delimiters.
            $current=self::delimiter($body,$boundary,0);$count=0;
            while(!$current[2]){
                $control?->check();if(++$count>RuntimeConfig::MAX_NODES){throw new JsonError('resource_limit','too many MIME parts');}
                $at=$current[1];$next=self::delimiter($body,$boundary,$at);
                $end=strpos($body,"\r\n\r\n",$at);if($end===false||$end+4>$next[0]){throw new JsonError('conversion','part headers are incomplete');}if($end-$at>$maxHeaders){throw new JsonError('resource_limit','part headers exceed ceiling');}
                $headers=[];foreach(explode("\r\n",substr($body,$at,$end-$at)) as$line){$colon=strpos($line,':');if($colon===false){throw new JsonError('conversion','malformed MIME header');}$key=strtolower(substr($line,0,$colon));$value=trim(substr($line,$colon+1));Wire::headerField($key,$value);if(isset($headers[$key])){throw new JsonError('conversion','duplicate MIME header');}$headers[$key]=$value;}
                $disposition=$headers['content-disposition']??throw new JsonError('conversion','named multipart part needs Content-Disposition');
                // Reuse the strict parameter grammar, preserving quoted pairs.
                [$kind,$attrs]=Protocol::media('application/'. $disposition);
                if($kind!=='application/form-data'||!isset($attrs['name'])){throw new JsonError('conversion','invalid named part disposition');}
                $start=$end+4;
                $name=$attrs['name'];$part=self::field($fields,$additional,$name);$parts[$name][]=self::value($part,substr($body,$start,$next[0]-$start),$attrs['filename']??null,$headers,$name,true,$control);
                $current=$next;
            }
        }
        self::rules($rules,$parts);foreach($parts as$name=>$values){self::multiplicity(self::field($fields,$additional,(string)$name),$values);}return $parts;
    }
    /** @return array{int,int,bool} */
    private static function delimiter(string $body,string $boundary,int $offset):array
    {
        if(preg_match('/(?:\A|\r\n)--'.preg_quote($boundary,'/').'(?:(--)[\t ]*(?:\r\n|\z)|[\t ]*\r\n)/',$body,$match,PREG_OFFSET_CAPTURE,$offset)!==1){throw new JsonError('conversion','MIME delimiter missing');}
        return [$match[0][1],$match[0][1]+strlen($match[0][0]),isset($match[1])];
    }
    /** @param array<array-key,string> $headers */
    private static function value(JsonValue $part,string $data,?string $filename,array $headers,string $name,bool $multipart,?CallControl $control):PartValue
    {
        $policy=Protocol::get($part,'representation');$kind=Protocol::text($policy,'kind');
        if(isset($headers['content-encoding'])&&strtolower(trim($headers['content-encoding']))!=='identity'){throw new JsonError('conversion','unsupported part content encoding');}
        foreach(['transfer-encoding','content-transfer-encoding'] as$key){if(isset($headers[$key])&&!in_array(strtolower(trim($headers[$key])),['identity','binary','8bit','7bit'],true)){throw new JsonError('conversion','unsupported part transfer encoding');}}
        if(isset($headers['content-type'])){
            $allowed=false;foreach(Protocol::list($part,'content_types') as$type){$candidate=JsonValue::fromObject(['media_type'=>$type,'representation'=>JsonValue::fromObject(['kind'=>JsonValue::fromString($kind==='binary'?'binary':($kind==='json'?'json':'text'))])]);try{Protocol::matchMedia(JsonValue::fromArray([$candidate]),$headers['content-type']);$allowed=true;}catch(SdkError $e){}}
            if(!$allowed){throw new JsonError('conversion','part Content-Type is not declared');}
        }
        if($kind==='binary'){if(strlen($data)>Protocol::integer(Protocol::get($policy,'bytes'),'max_bytes')){throw new JsonError('resource_limit','part exceeds byte bound');}$value=new Bytes($data);}
        elseif($kind==='json'){$value=JsonValue::parse($data,new JsonLimits(control:$control));}
        elseif($kind==='text'){$value=Protocol::parseScalar($data,Protocol::text($policy,'scalar'));}
        else {
            $serialization=Protocol::get($policy,'serialization');
            if(Protocol::text($serialization,'kind')!=='style'||Protocol::text(Protocol::get($serialization,'shape'),'kind')!=='scalar'){throw new JsonError('conversion','response form style needs a scalar carrier');}
            if($multipart){$prefix=$name.'=';if(Protocol::text($serialization,'style')!=='form'||!str_starts_with($data,$prefix)){throw new JsonError('conversion','invalid scalar styled part');}$data=substr($data,strlen($prefix));}
            $value=Protocol::parseScalar($data,Protocol::text(Protocol::get($serialization,'shape'),'scalar'));
        }
        return new PartValue($value,$filename,$headers['content-type']??null,$headers);
    }
}
