// HttpClient uppercases methods. This finite HTTP/1.1 path preserves custom
// tokens, normal certificate verification and independently cancellable sockets.
final class _ExactIoExchange {
  Socket? socket; ConnectionTask<Socket>? connection; void Function()? detach;
  bool closed=false;
  void close(){if(closed){return;}closed=true;detach?.call();connection?.cancel();socket?.destroy();}
}
String _ioOws(String value){
  var start=0;var end=value.length;
  while(start<end&&(value.codeUnitAt(start)==32||value.codeUnitAt(start)==9)){start++;}
  while(end>start&&(value.codeUnitAt(end-1)==32||value.codeUnitAt(end-1)==9)){end--;}
  return value.substring(start,end);
}
Future<TransportResponse> _exactRequest(TransportRequest request,Set<_ExactIoExchange> active,bool Function() clientClosed) async {
  final exchange=_ExactIoExchange();active.add(exchange);
  void release(){exchange.close();active.remove(exchange);}
  exchange.detach=request.cancellation.onCancel(release);
  _SocketInput? input;
  try{
    final connection=await Socket.startConnect(request.url.host,request.url.port);
    exchange.connection=connection;if(exchange.closed){connection.cancel();}
    var socket=await connection.socket;exchange.socket=socket;
    if(exchange.closed){socket.destroy();request.cancellation.throwIfCancelled();throw const ClientClosedException();}
    request.cancellation.throwIfCancelled();if(clientClosed()){throw const ClientClosedException();}
    if(request.url.scheme=='https'){
      socket=await SecureSocket.secure(socket,host:request.url.host);exchange.socket=socket;
      if(exchange.closed){socket.destroy();request.cancellation.throwIfCancelled();throw const ClientClosedException();}
    }
    final path=request.url.path.isEmpty?'/':request.url.path;
    final target='$path${request.url.hasQuery?'?${request.url.query}':''}';
    final headers=StringBuffer('${request.method} $target HTTP/1.1\r\nHost: ${request.url.authority}\r\nConnection: close\r\nContent-Length: ${request.body?.length??0}\r\n');
    for(final entry in request.headers.entries){
      if(const['host','connection','content-length','transfer-encoding'].contains(entry.key.toLowerCase())){throw const ConfigurationException('framing header cannot be supplied to the exact-method adapter');}
      headers.write('${entry.key}: ${entry.value}\r\n');
    }
    headers.write('\r\n');socket.add(latin1.encode(headers.toString()));if(request.body!=null){socket.add(request.body!);}await socket.flush();
    input=_SocketInput(socket);var headerBytes=0;
    for(var interim=0;interim<9;interim++){
      final statusLine=await input.line(request.maxResponseHeaderBytes-headerBytes);headerBytes+=statusLine.length+2;
      final match=RegExp(r'^HTTP/1\.[01] ([1-5][0-9][0-9])(?: .*)?$').firstMatch(latin1.decode(statusLine));
      if(match==null){throw const FormatException('invalid HTTP status line');}
      final status=int.parse(match[1]!);final fields=<String,List<String>>{};
      while(true){
        final line=await input.line(request.maxResponseHeaderBytes-headerBytes);headerBytes+=line.length+2;
        if(headerBytes>request.maxResponseHeaderBytes){throw const ResourceLimitException('response headers');}
        if(line.isEmpty){break;}
        final text=latin1.decode(line);final colon=text.indexOf(':');
        if(colon<1||text.startsWith(' ')||text.startsWith('\t')){throw const FormatException('invalid HTTP header line');}
        fields.putIfAbsent(text.substring(0,colon).toLowerCase(),()=>[]).add(_ioOws(text.substring(colon+1)));
      }
      if(status<200&&status!=101){continue;}
      final cursor=input;
      return TransportResponse(status:status,headers:fields,body:_exactBody(cursor,fields,status,request),
          onClose:() async{release();await cursor.cancel();});
    }
    throw const ResourceLimitException('interim HTTP responses');
  }on Object{release();await input?.cancel();rethrow;}
}
final class _SocketInput {
  _SocketInput(Socket socket):iterator=StreamIterator<List<int>>(socket);
  final StreamIterator<List<int>> iterator;List<int> current=const [];int offset=0;
  Future<bool> more() async {
    while(offset==current.length){
      if(!await iterator.moveNext()){return false;}
      current=iterator.current;offset=0;
    }
    return true;
  }
  Future<List<int>> line(int limit) async {
    if(limit<0){throw const ResourceLimitException('response header/line bytes');}
    final result=<int>[];
    while(await more()){
      final b=current[offset++];result.add(b);
      if(result.length>limit){throw const ResourceLimitException('response header/line bytes');}
      if(b==10){if(result.length<2||result[result.length-2]!=13){throw const FormatException('HTTP lines require CRLF');}return result.sublist(0,result.length-2);}
    }
    throw const FormatException('incomplete HTTP line');
  }
  Future<Uint8List> take(int maximum) async {
    if(!await more()){throw const FormatException('incomplete HTTP body');}
    final available=current.length-offset;final count=maximum<available?maximum:available;
    final result=Uint8List.fromList(current.sublist(offset,offset+count));offset+=count;return result;
  }
  Future<void> cancel()=>iterator.cancel();
}
Stream<List<int>> _exactBody(_SocketInput input,Map<String,List<String>> headers,int status,TransportRequest request) async* {
  if(request.method=='HEAD'||status<200||const[204,205,304].contains(status)){return;}
  final transfer=headers['transfer-encoding'];final lengths=headers['content-length'];
  if(transfer!=null){
    if(lengths!=null||transfer.length!=1||transfer.single.toLowerCase()!='chunked'){throw const FormatException('ambiguous transfer framing');}
    while(true){
      final token=latin1.decode(await input.line(request.maxResponseHeaderBytes)).split(';').first;
      if(token.isEmpty||!RegExp(r'^[0-9a-fA-F]+$').hasMatch(token)){throw const FormatException('invalid chunk size');}
      final normalized=token.replaceFirst(RegExp(r'^0+'),'');
      if(normalized.length>8){throw const ResourceLimitException('HTTP chunk bytes');}
      var remaining=normalized.isEmpty?0:int.parse(normalized,radix:16);
      if(remaining>request.maxResponseBytes){throw const ResourceLimitException('HTTP chunk bytes');}
      if(remaining==0){
        var total=0;
        while(true){final trailer=await input.line(request.maxResponseHeaderBytes-total);total+=trailer.length+2;if(trailer.isEmpty){return;}if(!trailer.contains(58)){throw const FormatException('invalid HTTP trailer');}}
      }
      while(remaining>0){final chunk=await input.take(remaining<8192?remaining:8192);remaining-=chunk.length;yield chunk;}
      if((await input.line(2)).isNotEmpty){throw const FormatException('invalid chunk terminator');}
    }
  }
  if(lengths!=null){
    if(lengths.length!=1||!RegExp(r'^[0-9]+$').hasMatch(lengths.single)){throw const FormatException('invalid Content-Length');}
    final normalized=lengths.single.replaceFirst(RegExp(r'^0+'),'');
    if(normalized.length>10){throw const ResourceLimitException('HTTP body bytes');}
    var remaining=normalized.isEmpty?0:int.parse(normalized,radix:10);
    if(remaining>request.maxResponseBytes){throw const ResourceLimitException('HTTP body bytes');}
    while(remaining>0){final bytes=await input.take(remaining<8192?remaining:8192);remaining-=bytes.length;yield bytes;}
  }else{
    var total=0;
    while(await input.more()){
      final bytes=await input.take(8192);total+=bytes.length;
      if(total>request.maxResponseBytes){throw const ResourceLimitException('HTTP body bytes');}yield bytes;
    }
  }
}
