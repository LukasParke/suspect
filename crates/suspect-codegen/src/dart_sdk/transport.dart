// A source selection can omit Future, Stream or individual payload families.
// ignore_for_file: unused_element, unused_element_parameter
/// Cooperative cancellation. Completed calls detach their registrations.
final class CancellationToken {
  bool _cancelled = false;
  int _next = 0;
  final Map<int, void Function()> _listeners = {};
  bool get isCancelled => _cancelled;
  void cancel() {
    if (_cancelled) { return; } _cancelled = true;
    final callbacks = _listeners.values.toList(); _listeners.clear();
    for (final callback in callbacks) { try { callback(); } on Object { /* Other observers must still run. */ } }
  }
  void Function() onCancel(void Function() callback) {
    if (_cancelled) { callback(); return () {}; }
    final id = _next++; _listeners[id] = callback;
    return () { _listeners.remove(id); };
  }
  void throwIfCancelled() { if (_cancelled) { throw const CancelledException(); } }
}

/// Portable byte-stream HTTP interface. Clients own the supplied transport.
abstract interface class HttpTransport {
  Future<TransportResponse> send(TransportRequest request);
  Future<void> close();
}

/// Immutable, fully assembled request; numeric values are already exact bytes.
final class TransportRequest {
  TransportRequest._(this.method, this.url, Map<String,String> headers,
      Uint8List? body, this.cancellation, this.timeout, this.maxResponseHeaderBytes, this.maxResponseBytes)
      : headers=Map.unmodifiable(headers), body=body?.asUnmodifiableView();
  final String method;
  final Uri url;
  final Map<String,String> headers;
  final Uint8List? body;
  final CancellationToken cancellation;
  final Duration timeout;
  final int maxResponseHeaderBytes;
  final int maxResponseBytes;
  @override String toString() => 'TransportRequest($method, redacted)';
}

/// An adapter response. Its release callback must close unread/partial bodies.
final class TransportResponse {
  TransportResponse({required this.status,required this.headers,required this.body,
      required Future<void> Function() onClose}) : _onClose=onClose;
  final int status;
  final Map<String,List<String>> headers;
  final Stream<List<int>> body;
  final Future<void> Function() _onClose;
  Future<void>? _closed;
  Future<void> close() {
    final previous=_closed; if(previous!=null){return previous;}
    final done=Completer<void>(); _closed=done.future;
    unawaited(_finishClosing(done,_onClose)); return done.future;
  }
}
Future<void> _finishClosing(Completer<void> done,Future<void> Function() close) async {
  try {await close();done.complete();} on Object catch(error,stack){done.completeError(error,stack);}
}

/// HTTP-mandated absence of content, distinct from JSON null and empty bytes.
final class NoBody { const NoBody(); }

/// Immutable bounded raw response access, on success and failure alike.
final class RawResponse {
  RawResponse._(this.status,this.headers,Uint8List body,this.truncated) : body=body.asUnmodifiableView();
  final int status;
  final Map<String,List<String>> headers;
  final Uint8List body;
  final bool truncated;
  @override String toString()=>'RawResponse(HTTP $status, ${body.length} captured bytes, truncated: $truncated)';
}

/// Source-declared link information. It never invokes or selects an operation.
final class LinkMetadata {
  const LinkMetadata({required this.name,required this.target,required this.byReference,
    required this.source,required this.parameters,this.requestBody=const Absent(),this.server});
  final String name;
  final String target;
  final bool byReference;
  final SchemaSource source;
  final JsonObject parameters;
  final Presence<JsonValue> requestBody;
  final ServerInfo? server;
}

abstract class SdkResponse {
  const SdkResponse(this.response,{this.links=const []});
  final RawResponse response;
  final List<LinkMetadata> links;
  int get status=>response.status;
  bool get isSuccess=>status>=200&&status<300;
}
sealed class SdkException implements Exception { const SdkException(); }
abstract class ApiException extends SdkException {
  const ApiException(this.response,{this.links=const []});
  final RawResponse response;
  final List<LinkMetadata> links;
  int get status=>response.status;
  @override String toString()=>'ApiException(HTTP $status)';
}
final class CancelledException extends SdkException {
  const CancelledException({this.response}); final RawResponse? response;
  @override String toString()=>'CancelledException: call cancelled';
}
final class TimeoutException extends SdkException {
  const TimeoutException({this.response}); final RawResponse? response;
  @override String toString()=>'TimeoutException: exchange deadline exceeded';
}
final class ClientClosedException extends SdkException {
  const ClientClosedException({this.response}); final RawResponse? response;
  @override String toString()=>'ClientClosedException: client closed';
}
final class ConfigurationException extends SdkException {
  const ConfigurationException(this.message); final String message;
  @override String toString()=>'ConfigurationException: $message';
}
final class ResourceLimitException extends SdkException {
  const ResourceLimitException(this.resource,{this.response}); final String resource; final RawResponse? response;
  @override String toString()=>'ResourceLimitException: $resource ceiling exceeded';
}
final class TransportException extends SdkException {
  const TransportException({this.cause,this.response}); final Object? cause; final RawResponse? response;
  @override String toString()=>'TransportException: HTTP exchange failed';
}
final class UnexpectedResponseException extends SdkException {
  const UnexpectedResponseException(this.response); final RawResponse response;
  @override String toString()=>'UnexpectedResponseException(HTTP ${response.status})';
}
final class MediaTypeException extends SdkException {
  const MediaTypeException(this.response); final RawResponse response;
  @override String toString()=>'MediaTypeException(HTTP ${response.status})';
}
final class InvalidResponseException extends SdkException {
  const InvalidResponseException(this.response,{this.codecFailure}); final RawResponse response; final CodecException? codecFailure;
  @override String toString()=>'InvalidResponseException(HTTP ${response.status})';
}

final class _BodyContent {
  const _BodyContent(this.bytes,this.contentType);
  final Uint8List bytes; final String contentType;
}
final class _InputValue {
  const _InputValue(this.parameter,this.value); final _Parameter parameter; final JsonValue value;
}
final class _RequestInput {
  const _RequestInput(this.parameters,this.body); final List<_InputValue> parameters; final _BodyContent? body;
}
enum _Stop { cancelled, timeout, closed }
final class _Call {
  final token=CancellationToken(); final done=Completer<void>(); final stopped=Completer<void>();
  _Stop? reason; _Capture? capture; Timer? timer; void Function()? detach;
  void stop(_Stop next) {if(reason!=null){return;}reason=next;stopped.complete();token.cancel();}
  SdkException failure()=>switch(reason!){
    _Stop.cancelled=>CancelledException(response:capture?.raw()),
    _Stop.timeout=>TimeoutException(response:capture?.raw()),
    _Stop.closed=>ClientClosedException(response:capture?.raw()),
  };
  Future<T> race<T>(Future<T> work,{Future<void> Function(T)? late}) {
    final guarded=work.then<T>((value){
      if(reason!=null){if(late!=null){unawaited(Future<void>.sync(()=>late(value)).catchError((Object _){}));}throw failure();}
      return value;
    });
    return Future.any([guarded,stopped.future.then<T>((_)=>throw failure())]);
  }
}
final class _Capture {
  _Capture(this.status,this.maximum,this.captureLimit,{required bool store}) : body=store?BytesBuilder():null;
  final int status; final int maximum; final int captureLimit;
  final BytesBuilder? body; final BytesBuilder prefix=BytesBuilder();
  Map<String,List<String>> headers=const {}; int seen=0; int events=0; bool complete=false;
  RawResponse raw()=>RawResponse._(status,headers,prefix.toBytes(),!complete||seen>prefix.length);
  void add(List<int> chunk) {
    if(++events>maximum+1024){throw ResourceLimitException('response events',response:raw());}
    final available=maximum-seen; final count=chunk.length>available?available+1:chunk.length;
    final data=Uint8List(count);
    for(var i=0;i<count;i++){final byte=chunk[i];if(byte<0||byte>255){throw InvalidResponseException(raw());}data[i]=byte;}
    final keep=captureLimit-prefix.length;
    if(keep>0){prefix.add(data.sublist(0,count<keep?count:keep));}
    seen+=count;
    if(chunk.length>available){throw ResourceLimitException('response bytes',response:raw());}
    body?.add(data);
  }
}
final class _Received {
  const _Received(this.raw,this.bytes,this.responseIndex,this.mediaIndex,this.noBody);
  final RawResponse raw; final Uint8List bytes; final int responseIndex; final int mediaIndex; final bool noBody;
  int get status=>raw.status;
}
final class _StreamRecord {
  const _StreamRecord(this.received,this.item);
  final _Received received; final JsonValue? item;
}

abstract class _ClientBase {
  _ClientBase(this._transport,this._credentials,this._server,this._timeout,this._maxRequestBytes,
      this._maxResponseBytes,this._maxCaptureBytes,this._maxHeaderBytes,this._maxStreamBufferBytes,
      String? userAgent,String? applicationId,List<int> ceilings): _userAgent=userAgent,_applicationId=applicationId {
    final values=[_maxRequestBytes,_maxResponseBytes,_maxCaptureBytes,_maxHeaderBytes,_maxStreamBufferBytes];
    for(var i=0;i<values.length;i++){if(values[i]<1||values[i]>ceilings[i]){throw const ConfigurationException('resource ceilings must be positive and may only be lowered');}}
    if(_maxCaptureBytes>_maxResponseBytes){throw const ConfigurationException('capture must fit response');}
    _checkTimeout(_timeout);
  }
  final HttpTransport _transport; final Credentials _credentials; final ServerSelection _server;
  final Duration _timeout; final int _maxRequestBytes; final int _maxResponseBytes;
  final int _maxCaptureBytes; final int _maxHeaderBytes; final int _maxStreamBufferBytes;
  /// Full ua/v1 User-Agent override; a non-empty value wins entirely and an
  /// explicit empty string suppresses the attribution header.
  final String? _userAgent;
  /// Replaces the SDK identity token in the automatic attribution header:
  /// `<name>` or `<name>/<version>` of RFC 9110 tokens.
  final String? _applicationId;
  final Set<_Call> _calls={}; bool _closed=false; Future<void>? _closing;
  Future<void> close(){
    final previous=_closing;if(previous!=null){return previous;}
    final done=Completer<void>();_closing=done.future;unawaited(_finishClosing(done,_close));return done.future;
  }
  Future<void> _close() async {
    _closed=true;final calls=_calls.toList();for(final call in calls){call.stop(_Stop.closed);}
    try{await _transport.close();}on Object catch(error){throw TransportException(cause:error);}
    finally{await Future.wait(calls.map((call)=>call.done.future));}
  }
  _Call _begin(CancellationToken? cancellation,Duration timeout){
    if(_closed){throw const ClientClosedException();}cancellation?.throwIfCancelled();_checkTimeout(timeout);
    final call=_Call();_calls.add(call);call.detach=cancellation?.onCancel(()=>call.stop(_Stop.cancelled));
    call.timer=Timer(timeout,()=>call.stop(_Stop.timeout));return call;
  }
  Future<void> _release(_Call call,TransportResponse? response,StreamIterator<List<int>>? input) async {
    call.timer?.cancel();call.detach?.call();
    try{try{await input?.cancel();}finally{await response?.close();}}
    finally{_calls.remove(call);call.done.complete();}
  }
  /// ua/v1 application identity: `<name>` or `<name>/<version>` of RFC 9110 tokens.
  static final RegExp _userAgentIdentity=RegExp(r"^[A-Za-z0-9!#$%&'*+.^`|~-]+(?:/[A-Za-z0-9!#$%&'*+.^`|~-]+)?$");
  /// ua/v1 attribution: an explicit caller User-Agent wins entirely, an explicit
  /// empty string suppresses the header, and the default identifies suspect as
  /// the generator and the SDK package or a caller-supplied application as the
  /// client. Dart exposes no stable runtime version API, so v1 records the
  /// language version as `unknown` instead of omitting the comment segment.
  String? _resolveUserAgent(){
    final override=_userAgent;
    if(override!=null){return override.isEmpty?null:override;}
    if(userAgentSuspectVersion.isEmpty){return null;}
    var identity='$userAgentSdkName/$userAgentSdkVersion';
    final application=_applicationId;
    if(application!=null&&application.isNotEmpty){
      if(application.length>128||!_userAgentIdentity.hasMatch(application)){return null;}
      identity=application;
    }
    return 'suspect/$userAgentSuspectVersion $identity (dart/unknown; openapi/$userAgentSpecVersion)';
  }
  Future<TransportResponse> _send(_Call call,_WireOperation op,_RequestInput values,Duration timeout,ServerSelection? server,int? alternative) async {
    final builder=_RequestBuilder(op,server??_server,_maxRequestBytes);
    for(final value in values.parameters){builder.parameter(value.parameter,value.value);}
    if(values.body!=null){builder.body(values.body!);}
    await call.race(_attachCredentials(_credentials,op,builder,call.token,alternative));
    // ua/v1 attribution applies after declared parameters and credentials so a
    // source-declared user-agent header keeps precedence.
    final userAgent=_resolveUserAgent();
    if(userAgent!=null&&!builder.headers.containsKey('user-agent')){builder.header('user-agent',userAgent);}
    return call.race<TransportResponse>(Future.sync(()=>_transport.send(
        TransportRequest._(op.method,builder.url(),builder.headers,values.body?.bytes,call.token,timeout,_maxHeaderBytes,_maxResponseBytes))),late:(value)=>value.close());
  }
  (_Capture,int,int,bool) _inspect(_Call call,_WireOperation op,TransportResponse response,bool stream){
    final capture=_Capture(response.status,_maxResponseBytes,_maxCaptureBytes,store:!stream);call.capture=capture;
    if(response.status<100||response.status>599){throw InvalidResponseException(capture.raw());}
    capture.headers=_responseHeaders(response.headers,_maxHeaderBytes,capture);
    final index=op.response(response.status);
    final forbidden=op.method=='HEAD'||response.status<200||const[204,205,304].contains(response.status);
    var media=-1;
    if(index>=0&&!forbidden&&op.responses[index].media.isNotEmpty){
      final types=capture.headers['content-type'];final encoding=capture.headers['content-encoding'];
      if(types==null||types.length!=1||encoding!=null&&(encoding.length!=1||encoding.single.toLowerCase()!='identity')){throw MediaTypeException(capture.raw());}
      try{media=_selectMedia(op.responses[index].media,types.single);}on ConfigurationException{throw MediaTypeException(capture.raw());}
    }
    return(capture,index,media,forbidden);
  }
  Future<_Received> _collect(_Call call,TransportResponse response,StreamIterator<List<int>> input,
      _Capture capture,int index,int media,bool forbidden,_WireOperation op) async {
    final expected=forbidden?null:_contentLength(capture);
    final max=media<0?_maxResponseBytes:op.responses[index].media[media].maxBytes;
    if(!forbidden){
      while(await call.race(input.moveNext())){
        capture.add(input.current);
        if(capture.seen>max){throw ResourceLimitException('declared body bytes',response:capture.raw());}
      }
      if(expected!=null&&expected!=capture.seen){throw InvalidResponseException(capture.raw());}
    }
    capture.complete=true;
    return _Received(capture.raw(),capture.body?.takeBytes()??Uint8List(0),index,media,forbidden);
  }
  Future<_Received> _exchange(_WireOperation op,_RequestInput Function() prepare,CancellationToken? cancellation,
      Duration? timeout,ServerSelection? server,int? alternative) async {
    final limit=timeout??_timeout;final call=_begin(cancellation,limit);
    TransportResponse? response;StreamIterator<List<int>>? input;Object? primary;
    try{
      response=await _send(call,op,prepare(),limit,server,alternative);
      final(capture,index,media,forbidden)=_inspect(call,op,response,false);
      input=StreamIterator(response.body);
      return await _collect(call,response,input,capture,index,media,forbidden,op);
    }on Object catch(error){primary=error;if(call.reason!=null){throw call.failure();}if(error is SdkException||error is CodecException){rethrow;}if(error is JsonException){if(error.resourceLimit){throw const ResourceLimitException('request encoding');}throw const ConfigurationException('invalid native JSON input');}throw TransportException(cause:error,response:call.capture?.raw());}
    finally{
      try{await _release(call,response,input);}on Object catch(error){if(primary==null){throw TransportException(cause:error,response:call.capture?.raw());}}
      if(primary==null&&call.reason!=null){throw call.failure();}
    }
  }

  Stream<T> _stream<T>(_WireOperation op,_RequestInput Function() prepare,T Function(_StreamRecord) convert,
      CancellationToken? cancellation,Duration? timeout,ServerSelection? server,int? alternative){
    late StreamController<T> controller;
    _Call? active;Completer<void>? paused;Future<void>? running;var listenerCancelled=false;
    Future<void> drive() async {
      TransportResponse? response;StreamIterator<List<int>>? input;_Call? call;
      try{
        if(listenerCancelled){return;}
        final limit=timeout??_timeout;call=_begin(cancellation,limit);active=call;
        response=await _send(call,op,prepare(),limit,server,alternative);
        final(capture,index,media,forbidden)=_inspect(call,op,response,true);
        input=StreamIterator(response.body);
        if(index<0||response.status<200||response.status>=300||forbidden){
          // Errors are finite complete bodies, not an invented item stream.
          final errors=_Capture(response.status,_maxResponseBytes,_maxCaptureBytes,store:true)..headers=capture.headers;
          call.capture=errors;
          final received=await _collect(call,response,input,errors,index,media,forbidden,op);
          controller.add(convert(_StreamRecord(received,null)));
          return;
        }
        final spec=op.responses[index].media[media];
        final framer=_Framer(spec.kind,spec.maxItemBytes);
        final expected=_contentLength(capture);
        while(await call.race(input.moveNext())){
          final chunk=input.current;
          if(chunk.length>_maxStreamBufferBytes){throw ResourceLimitException('stream chunk buffer',response:capture.raw());}
          capture.add(chunk);
          for(final item in framer.add(chunk)){
            if(paused!=null){await call.race(paused!.future);}
            if(listenerCancelled){return;}
            controller.add(convert(_StreamRecord(_Received(capture.raw(),Uint8List(0),index,media,false),item)));
            if(paused!=null){await call.race(paused!.future);}
          }
        }
        if(expected!=null&&expected!=capture.seen){throw InvalidResponseException(capture.raw());}
        for(final item in framer.finish()){
          if(paused!=null){await call.race(paused!.future);}
          if(listenerCancelled){return;}
          controller.add(convert(_StreamRecord(_Received(capture.raw(),Uint8List(0),index,media,false),item)));
        }
        capture.complete=true;
      }on Object catch(error,stack){
        if(!listenerCancelled){
          final failure=call?.reason!=null?call!.failure():error is SdkException||error is CodecException?error:
              error is JsonException?(error.resourceLimit?ResourceLimitException('stream framing',response:call?.capture?.raw()):InvalidResponseException(call!.capture!.raw())):TransportException(cause:error,response:call?.capture?.raw());
          controller.addError(failure,stack);
        }
      }finally{
        if(call!=null){try{await _release(call,response,input);}on Object catch(error,stack){if(!listenerCancelled){controller.addError(TransportException(cause:error),stack);}}}
        unawaited(controller.close());
      }
    }
    controller=StreamController<T>(sync:true,onListen:(){running=Future<void>.microtask(drive);},
      onPause:(){paused??=Completer<void>();},onResume:(){final wait=paused;paused=null;wait?.complete();},
      onCancel:() async {listenerCancelled=true;active?.stop(_Stop.cancelled);await running;});
    return controller.stream;
  }
}

void _checkTimeout(Duration value){if(value<=Duration.zero||value>const Duration(days:1)){throw const ConfigurationException('timeout must be positive and at most one day');}}
Map<String,List<String>> _responseHeaders(Map<String,List<String>> input,int maximum,_Capture capture){
  final result=<String,List<String>>{};var size=2;
  for(final entry in input.entries){
    size+=entry.key.length+4;
    if(size>maximum){throw ResourceLimitException('response headers',response:capture.raw());}
    if(entry.key.isEmpty||!entry.key.codeUnits.every(_tchar)){throw InvalidResponseException(capture.raw());}
    final values=result.putIfAbsent(entry.key.toLowerCase(),()=>[]);
    for(final value in entry.value){
      size+=entry.key.length+value.length+4;
      if(size>maximum){throw ResourceLimitException('response headers',response:capture.raw());}
      if(value.codeUnits.any((c)=>c<32&&c!=9||c==127||c>255)){throw InvalidResponseException(capture.raw());}
      values.add(_trimOws(value));
    }
  }
  return Map.unmodifiable(result.map((key,value)=>MapEntry(key,List<String>.unmodifiable(value))));
}
int? _contentLength(_Capture capture){
  final lengths=capture.headers['content-length'];final transfer=capture.headers['transfer-encoding'];
  if(transfer!=null&&(lengths!=null||transfer.length!=1||transfer.single.toLowerCase()!='chunked')){throw InvalidResponseException(capture.raw());}
  if(lengths==null){return null;}
  if(lengths.length!=1||lengths.single.isEmpty||!lengths.single.codeUnits.every(_digit)){throw InvalidResponseException(capture.raw());}
  final token=lengths.single.replaceFirst(RegExp(r'^0+'),'');final max=capture.maximum.toString();
  if(token.length>max.length||token.length==max.length&&token.compareTo(max)>0){throw ResourceLimitException('response bytes',response:capture.raw());}
  return token.isEmpty?0:int.parse(token,radix:10);
}

Uint8List _bytes(List<int> value,int maximum){
  if(value.length>maximum){throw const ResourceLimitException('encoded body/part bytes');}
  final result=Uint8List(value.length);
  for(var i=0;i<value.length;i++){if(value[i]<0||value[i]>255){throw const ConfigurationException('invalid native byte value');}result[i]=value[i];}
  return result;
}
Uint8List _jsonBytes(JsonValue value,int maximum)=>Uint8List.fromList(utf8.encode(writeJson(value,
    limits:JsonLimits(maxBytes:maximum,maxDepth:_encodeLimits.maxDepth,maxSteps:_encodeLimits.maxSteps,maxNumberBytes:_encodeLimits.maxNumberBytes))));
Uint8List _textBytes(String value,int maximum){if(value.length>maximum||_unicodeLength(value)>maximum){throw const ResourceLimitException('encoded text bytes');}return Uint8List.fromList(utf8.encode(value));}
JsonValue _responseJson(_Received response){try{return parseJsonBytes(response.bytes,limits:_decodeLimits);}on JsonException{throw InvalidResponseException(response.raw);}}
String _responseText(_Received response){try{return utf8.decode(response.bytes,allowMalformed:false);}on FormatException{throw InvalidResponseException(response.raw);}}
T _decoded<T>(_Received response,T Function() decode){try{return decode();}on CodecException catch(error){throw InvalidResponseException(response.raw,codecFailure:error);}on JsonException{throw InvalidResponseException(response.raw);}}
