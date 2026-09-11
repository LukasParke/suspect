<?php
declare(strict_types=1);
namespace __NAMESPACE__;

require_once __DIR__ . '/Transport.php';

/** A single owned byte stream. read returns null only at EOF. */

/** Headers plus an owned response body, before buffering or typed item conversion. */
final readonly class StreamResponse
{
    /** @var array<array-key,list<string>> */
    public array $headers;
    /** @param array<array-key,string|list<string>> $headers */
    public function __construct(public int $status, array $headers, public BodyReader $reader)
    {
        $metadata=new HttpResponse($status,$headers,'');$this->headers=$metadata->headers;
    }
    public function close(): void { $this->reader->close(); }
    /** @return list<string> */
    public function headerValues(string $name):array{return $this->headers[strtolower($name)]??[];}
    public function header(string $name):?string{$values=$this->headerValues($name);return $values===[]?null:implode(', ',$values);}
    /** Capture and close an unexpected response without unbounded draining. */
    public function capture(int $maxBytes, bool $truncated = true, ?CallControl $control = null):ResponseCapture
    {
        $body='';$eof=false;
        try{while(strlen($body)<$maxBytes){$control?->check();$chunk=$this->reader->read();$control?->check();if($chunk===null){$eof=true;break;}if($chunk===''){throw new SdkError('transport','body reader made no progress');}$body.=substr($chunk,0,$maxBytes-strlen($body));}}
        catch(\Throwable $e){}
        finally{try{$this->reader->close();}catch(\Throwable $e){}}
        return new ResponseCapture($this->status,$this->headers,$body,!$eof||$truncated);
    }
    public function buffer(int $maxBytes, int $capture, ?CallControl $control = null): HttpResponse
    {
        $body='';$failed=false;
        try {
            while(true){
                $control?->check();$chunk=$this->reader->read();$control?->check();if($chunk===null){break;}if($chunk===''){throw new SdkError('transport','body reader made no progress');}
                if(strlen($chunk)>$maxBytes-strlen($body)){$body.=substr($chunk,0,$maxBytes-strlen($body));throw new SdkError('resource_limit','response exceeds byte ceiling');}
                $body.=$chunk;
            }
            return new HttpResponse($this->status,$this->headers,$body);
        } catch(SdkError $error){$failed=true;throw $error->withCapture(new ResponseCapture($this->status,$this->headers,substr($body,0,$capture),true));}
        catch(\Throwable $error){$failed=true;throw new SdkError('transport','body reader failed',new ResponseCapture($this->status,$this->headers,substr($body,0,$capture),true),$error);}
        finally{try{$this->reader->close();}catch(\Throwable $error){if(!$failed){throw new SdkError('transport','body cleanup failed',previous:$error);}}}
    }
}

/** Mutable callback state is separate from the CurlBody owner, avoiding a reference cycle. @internal */
final class CurlStreamState
{
    public int $status=0;
    /** @var array<array-key,list<string>> */
    public array $headers=[];
    public int $headerBytes=0;
    public bool $headersDone=false;
    public bool $paused=false;
    public bool $done=false;
    public ?string $pending=null;
    /** @var array{string,string}|null */
    public ?array $failure=null;
}

/** Pull-based cURL multi adapter. At most one native cURL chunk is queued. @internal */
final class CurlBody implements BodyReader
{
    private ?\CurlHandle $curl;
    private ?\CurlMultiHandle $multi;
    private CurlStreamState $state;
    private function __construct(private readonly HttpRequest $request)
    {
        $request->check();$urlParts=Wire::serverOrigin($request->url);
        if($request->url===''||$request->method===''){throw new SdkError('configuration','HTTP method and URL must be nonempty');}
        $curl=curl_init();if($curl===false){throw new SdkError('transport','cURL initialization failed');}
        $this->curl=$curl;$this->multi=curl_multi_init();$this->state=$state=new CurlStreamState();
        $lines=[];foreach($request->headers as$name=>$value){$lines[]=$name.': '.$value;}$lines[]='Expect:';
        $options=[CURLOPT_URL=>$request->url,CURLOPT_CUSTOMREQUEST=>$request->method,CURLOPT_HTTPHEADER=>$lines,
            CURLOPT_REQUEST_TARGET=>(isset($urlParts['path'])&&$urlParts['path']!==''?$urlParts['path']:'/').(isset($urlParts['query'])?'?'.$urlParts['query']:''),
            CURLOPT_NOBODY=>$request->method==='HEAD',CURLOPT_FOLLOWLOCATION=>false,CURLOPT_MAXREDIRS=>0,CURLOPT_PROXY=>'',CURLOPT_NETRC=>false,
            CURLOPT_PROTOCOLS_STR=>'http,https',CURLOPT_REDIR_PROTOCOLS_STR=>'http,https',CURLOPT_SSL_VERIFYPEER=>true,CURLOPT_SSL_VERIFYHOST=>2,
            CURLOPT_FORBID_REUSE=>true,CURLOPT_FRESH_CONNECT=>true,CURLOPT_PATH_AS_IS=>true,CURLOPT_HTTP_CONTENT_DECODING=>false,
            CURLOPT_TIMEOUT_MS=>$request->timeoutMilliseconds,CURLOPT_CONNECTTIMEOUT_MS=>min(10000,$request->timeoutMilliseconds),CURLOPT_NOSIGNAL=>true,
            CURLOPT_HEADERFUNCTION=>static function(\CurlHandle $handle,string $line)use($state,$request):int{
                try{
                    $request->check();$state->headerBytes+=strlen($line);
                    if($state->headerBytes>$request->maxHeaderBytes){throw new SdkError('resource_limit','stream headers exceed ceiling');}
                    if(str_starts_with($line,'HTTP/')){
                        if(preg_match('/\AHTTP\/[0-9.]+ ([0-9]{3})(?:[ \r\n]|$)/D',$line,$m)!==1){throw new SdkError('transport','invalid HTTP status line');}
                        $state->status=(int)$m[1];$state->headers=[];$state->headersDone=false;
                    }elseif(trim($line)===''){$state->headersDone=$state->status>=200;}
                    else{$colon=strpos($line,':');if($colon===false||$line[0]===' '||$line[0]==="\t"){throw new SdkError('transport','invalid header framing');}
                        $name=strtolower(substr($line,0,$colon));$value=trim(substr($line,$colon+1)," \t\r\n");Wire::headerField($name,$value);$state->headers[$name][]=$value;}
                    return strlen($line);
                }catch(SdkError $e){$state->failure=[$e->kind,$e->getMessage()];return 0;}
            },
            CURLOPT_WRITEFUNCTION=>static function(\CurlHandle $handle,string $chunk)use($state):int{
                if($state->pending!==null){$state->paused=true;return CURL_WRITEFUNC_PAUSE;}
                $state->pending=$chunk;return strlen($chunk);
            },
        ];
        if($request->body!==null){$options[CURLOPT_POSTFIELDS]=$request->body;}
        try{if(!curl_setopt_array($curl,$options)||curl_multi_add_handle($this->multi,$curl)!==CURLM_OK){throw new SdkError('transport','cURL stream configuration failed');}}
        catch(\Throwable $e){$this->close();throw $e;}
    }
    public static function open(HttpRequest $request): StreamResponse
    {
        if(!extension_loaded('curl')){throw new SdkError('transport','streaming requires ext-curl or an injected StreamTransport');}
        $body=new self($request);
        try{
            while(!$body->state->headersDone){$body->tick();if($body->state->done&&!$body->state->headersDone){throw new SdkError('transport','response headers incomplete');}}
            return new StreamResponse($body->state->status,$body->state->headers,$body);
        }catch(\Throwable $e){$body->close();throw $e;}
    }
    private function tick(): void
    {
        $this->request->check();
        if($this->curl===null||$this->multi===null){throw new SdkError('cancelled','stream is closed');}
        if($this->state->paused&&$this->state->pending===null){$this->state->paused=false;curl_pause($this->curl,CURLPAUSE_CONT);}
        do{$code=curl_multi_exec($this->multi,$running);}while($code===CURLM_CALL_MULTI_PERFORM);
        if($code!==CURLM_OK){throw new SdkError('transport','cURL multi transfer failed');}
        while(($message=curl_multi_info_read($this->multi))!==false){
            if($message['handle']===$this->curl){$this->state->done=true;if($message['result']!==CURLE_OK){$this->state->failure??=[$message['result']===CURLE_OPERATION_TIMEDOUT?'timeout':'transport','HTTP stream failed'];}}
        }
        if($this->state->failure!==null){throw new SdkError($this->state->failure[0],$this->state->failure[1]);}
        $this->request->check();
        if($this->state->pending===null&&!$this->state->done){if(curl_multi_select($this->multi,0.025)===-1){usleep(1000);}}
    }
    public function read(): ?string
    {
        try{
            while(true){
                $this->request->check();
                if($this->state->pending!==null){$chunk=$this->state->pending;$this->state->pending=null;return $chunk;}
                if($this->state->done){$this->close();return null;}
                $this->tick();
            }
        }catch(\Throwable $e){$this->close();throw $e;}
    }
    public function close(): void
    {
        if($this->multi!==null&&$this->curl!==null){curl_multi_remove_handle($this->multi,$this->curl);}
        $this->curl=null;$this->multi=null;$this->state->pending=null;$this->state->done=true;
    }
    public function __destruct(){ $this->close(); }
}

/** A source-typed, single-use closable iterable. Always close in finally when retaining its iterator.
 * @template T
 * @implements \IteratorAggregate<int,T>
 */
final class ItemStream implements \IteratorAggregate
{
    private bool $started=false;
    private bool $closed=false;
    /** @param \Closure(JsonValue):T $decode */
    public function __construct(private readonly StreamResponse $response, private readonly string $framing, private readonly \Closure $decode, private readonly CallContext $call, private readonly int $maxItemBytes, private readonly string $operationId, private readonly string $source, private readonly int $maxBodyBytes = RuntimeConfig::MAX_RESPONSE_BYTES) {}
    public function close():void {if(!$this->closed){$this->closed=true;$this->response->close();}}
    /** @return \Generator<int,T> */
    public function getIterator(): \Traversable
    {
        if($this->started||$this->closed){throw new SdkError('cancelled','item stream is single-use');}$this->started=true;
        $buffer='';$capture='';$total=0;$count=0;$bom=$this->framing==='server-sent-events';$event=[];$data=[];$eventBytes=0;$failed=false;$scan=0;
        try{
            while(true){
                $this->call->check();$chunk=$this->response->reader->read();$this->call->check();$final=$chunk===null;
                if($chunk===null){if($this->framing==='json-lines'||!str_ends_with($buffer,"\r")){break;}$chunk="\n";}
                elseif($chunk===''){throw new SdkError('transport','body reader made no progress');}
                $this->call->check();$total+=$final?0:strlen($chunk);
                if(!$final&&strlen($capture)<$this->call->maxCaptureBytes){$capture.=substr($chunk,0,$this->call->maxCaptureBytes-strlen($capture));}
                if($total>min($this->call->maxResponseBytes,$this->maxBodyBytes)){throw new SdkError('resource_limit','stream total byte ceiling exceeded');}
                $buffer.=$chunk;
                if($bom){if(strlen($buffer)<3){continue;}$bom=false;if(str_starts_with($buffer,"\xEF\xBB\xBF")){$buffer=substr($buffer,3);}}
                $start=0;
                while(($newline=$scan+strcspn($buffer,$this->framing==='json-lines'?"\n":"\r\n",$scan))<strlen($buffer)){
                    if($buffer[$newline]==="\r"&&$newline+1===strlen($buffer)){break;}
                    $line=substr($buffer,$start,$newline-$start);$skip=$buffer[$newline]==="\r"&&($buffer[$newline+1]??'')==="\n"?2:1;$start=$newline+$skip;$scan=$start;
                    if(strlen($line)>$this->maxItemBytes){throw new SdkError('resource_limit','stream line byte ceiling exceeded');}
                    if($this->framing==='json-lines'){
                        if($line===''){continue;}$item=JsonValue::parse($line,new JsonLimits(maxBytes:min($this->maxItemBytes,RuntimeConfig::MAX_JSON_BYTES),control:$this->call->control));
                    }else{
                        $eventBytes+=strlen($line)+$skip;if($eventBytes>$this->maxItemBytes){throw new SdkError('resource_limit','SSE frame byte ceiling exceeded');}
                        $line=self::utf8($line);
                        if($line===''){
                            if($data===[]){$event=[];$eventBytes=0;continue;}
                            $event['data']=JsonValue::fromString(implode("\n",$data));$item=JsonValue::fromObject($event);$event=[];$data=[];$eventBytes=0;
                        }else{
                            if($line[0]===':'){continue;}$parts=explode(':',$line,2);$value=$parts[1]??'';if(str_starts_with($value,' ')){$value=substr($value,1);}
                            if($parts[0]==='data'){$data[]=$value;}
                            elseif($parts[0]==='event'){$event['event']=JsonValue::fromString($value);}
                            elseif($parts[0]==='id'&&!str_contains($value,"\0")){$event['id']=JsonValue::fromString($value);}
                            elseif($parts[0]==='retry'&&preg_match('/\A[0-9]+\z/D',$value)===1){$event['retry']=JsonValue::fromNumber(JsonNumber::fromString(ltrim($value,'0')?:'0'));}
                            continue;
                        }
                    }
                    if(++$count>RuntimeConfig::MAX_NODES){throw new SdkError('resource_limit','stream item count exceeded');}
                    $value=($this->decode)($item);$this->call->check();yield $value;
                    if($this->closed){return;}
                }
                $scan=$newline-$start;$buffer=substr($buffer,$start);
                if(strlen($buffer)>$this->maxItemBytes){throw new SdkError('resource_limit','stream line byte ceiling exceeded');}
                if($final){break;}
            }
            if($this->framing==='json-lines'&&$buffer!==''){
                if(str_ends_with($buffer,"\r")){$buffer=substr($buffer,0,-1);}
                if($buffer!==''){if(++$count>RuntimeConfig::MAX_NODES){throw new SdkError('resource_limit','stream item count exceeded');}$value=($this->decode)(JsonValue::parse($buffer,new JsonLimits(maxBytes:min($this->maxItemBytes,RuntimeConfig::MAX_JSON_BYTES),control:$this->call->control)));$this->call->check();yield $value;}
            }
        }catch(JsonError|ValidationError $e){$failed=true;throw new SdkError('response_validation','stream item violates its source framing/schema',new ResponseCapture($this->response->status,$this->response->headers,$capture,$total>strlen($capture)),$e,$this->operationId,$this->source);}
        catch(SdkError $e){$failed=true;throw $e->withCapture(new ResponseCapture($this->response->status,$this->response->headers,$capture,true))->at($this->operationId,$this->source);}
        catch(\Throwable $e){$failed=true;throw new SdkError('transport','stream reader failed',new ResponseCapture($this->response->status,$this->response->headers,$capture,true),$e,$this->operationId,$this->source);}
        finally{try{$this->close();}catch(\Throwable $e){if(!$failed){throw new SdkError('transport','stream cleanup failed',previous:$e,operationId:$this->operationId,source:$this->source);}}}
    }
    /** HTML's UTF-8 decode algorithm replaces malformed sequences for SSE. */
    private static function utf8(string $bytes):string
    {
        if(preg_match('//u',$bytes)===1){return $bytes;}
        $value=json_decode(json_encode($bytes,JSON_INVALID_UTF8_SUBSTITUTE|JSON_THROW_ON_ERROR),false,2,JSON_THROW_ON_ERROR);
        if(!is_string($value)){throw new JsonError('syntax','invalid SSE string');}
        return $value;
    }
    public function __destruct(){ try{$this->close();}catch(\Throwable $error){} }
}
