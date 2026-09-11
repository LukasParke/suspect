/// Incremental byte framing. Buffer limits apply before decoding a line/event.
final class _Framer {
  _Framer(this.kind,this.maximum);
  final _MediaKind kind; final int maximum;
  final BytesBuilder _line=BytesBuilder();
  final StringBuffer _data=StringBuffer();
  String? _id; String? _event; JsonInteger? _retry;
  int _eventBytes=0; bool _hasData=false; bool _afterCr=false; bool _first=true;

  Iterable<JsonValue> add(List<int> bytes) sync* {
    for(final byte in bytes){
      if(byte<0||byte>255){throw const JsonException('invalid stream byte');}
      if(_afterCr){_afterCr=false;if(byte==10){continue;}}
      if(byte==13||byte==10){
        if(byte==13){_afterCr=true;}
        final item=_acceptLine(_line.takeBytes());
        if(item!=null){yield item;}
      }else{
        if(_line.length>=maximum){throw const JsonException('stream line limit exceeded',resourceLimit:true);}
        _line.addByte(byte);
      }
    }
  }
  JsonValue? _acceptLine(Uint8List bytes){
    if(_first){_first=false;if(bytes.length>=3&&bytes[0]==239&&bytes[1]==187&&bytes[2]==191){bytes=Uint8List.sublistView(bytes,3);}}
    String line;
    try{line=utf8.decode(bytes,allowMalformed:false);}on FormatException{throw const JsonException('invalid UTF-8 stream');}
    if(kind==_MediaKind.jsonl){
      if(line.isEmpty){throw const JsonException('JSON-lines records cannot be empty');}
      return parseJson(line,limits:JsonLimits(maxBytes:maximum,maxDepth:_decodeLimits.maxDepth,maxSteps:_decodeLimits.maxSteps,maxNumberBytes:_decodeLimits.maxNumberBytes));
    }
    _eventBytes+=bytes.length+1;
    if(_eventBytes>maximum){throw const JsonException('SSE event limit exceeded',resourceLimit:true);}
    if(line.isEmpty){
      JsonValue? result;
      if(_hasData){
        final data=_data.toString();
        result=JsonObject({'data':JsonString(data.substring(0,data.length-1)),
          if(_event!=null)'event':JsonString(_event!),if(_id!=null)'id':JsonString(_id!),
          if(_retry!=null)'retry':_retry!});
      }
      _data.clear();_event=null;_id=null;_retry=null;_hasData=false;_eventBytes=0;
      return result;
    }
    if(line.startsWith(':')){return null;}
    final colon=line.indexOf(':');
    final field=colon<0?line:line.substring(0,colon);
    var value=colon<0?'':line.substring(colon+1);
    if(value.startsWith(' ')){value=value.substring(1);}
    switch(field){
      case 'data':_data.write(value);_data.write('\n');_hasData=true;
      case 'event':_event=value;
      case 'id':if(!value.contains('\u0000')){_id=value;}
      case 'retry':if(value.isNotEmpty&&value.codeUnits.every(_digit)){
        final normalized=value.replaceFirst(RegExp(r'^0+'),'');
        _retry=JsonInteger.parse(normalized.isEmpty?'0':normalized,maxBytes:_decodeLimits.maxNumberBytes);
      }
    }
    return null;
  }
  Iterable<JsonValue> finish() sync* {
    if(kind==_MediaKind.jsonl&&_line.length>0){final item=_acceptLine(_line.takeBytes());if(item!=null){yield item;}}
    // SSE dispatch requires a blank line; EOF never invents an event.
  }
}
