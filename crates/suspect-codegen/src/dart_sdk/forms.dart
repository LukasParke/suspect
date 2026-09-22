// Some selected APIs use no styled or constrained parts.
// ignore_for_file: unused_element, unused_element_parameter
enum _PartKind { json, text, bytes, style }
final class _PartSpec {
  const _PartSpec(this.name,this.kind,{this.scalar=_Scalar.string,this.serialization,
      this.encoding=_Encoding.none,this.contentTypes=const [],this.required=false,
      this.repeated=false,this.minimum,this.maximum,this.maxBytes=8388608});
  final String? name; final _PartKind kind; final _Scalar scalar;
  final _Serialization? serialization; final _Encoding encoding;
  final List<String> contentTypes; final bool required; final bool repeated;
  final String? minimum; final String? maximum; final int maxBytes;
}
final class _FormSpec {
  const _FormSpec(this.fields,{this.extra,this.required=const [],this.minimum,this.maximum});
  final Map<String,_PartSpec> fields; final _PartSpec? extra; final List<String> required;
  final String? minimum; final String? maximum;
}
final class _PartData {
  const _PartData(this.value,{this.headers=const {},this.contentType,this.filename});
  final Object value; final Map<String,String> headers; final String? contentType; final String? filename;
}
Uint8List _partRaw(Uint8List bytes,_Conversion conversion,int maximum){
  conversion.spend(bytes.length+1);
  return _bytes(bytes,maximum);
}
int _partCount(int previous,int added,int maximum){
  if(added>16384-previous||added>maximum-previous){throw const ResourceLimitException('physical part count');}
  return previous+added;
}
bool _countFits(int count,String? minimum,String? maximum){
  final value=BigInt.from(count);
  return (minimum==null||value>=BigInt.parse(minimum,radix:10))&&(maximum==null||value<=BigInt.parse(maximum,radix:10));
}
void _structural(_FormSpec spec,Map<String,List<_PartData>> fields){
  if(!_countFits(fields.length,spec.minimum,spec.maximum)||spec.required.any((name)=>!fields.containsKey(name))){
    throw const ConfigurationException('form/multipart required fields or property count violated');
  }
  for(final entry in fields.entries){
    final part=spec.fields[entry.key]??spec.extra;
    if(part==null){throw const ConfigurationException('undeclared form/multipart field');}
    if(part.repeated){
      if(!_countFits(entry.value.length,part.minimum,part.maximum)||part.required&&entry.value.isEmpty){throw const ConfigurationException('repeated part cardinality violated');}
    }else if(entry.value.length!=1){throw const ConfigurationException('a single part needs exactly one value');}
  }
}
String _partText(String name,_PartSpec spec,_PartData part,int maximum){
  switch(spec.kind){
    case _PartKind.json:return writeJson(part.value as JsonValue,limits:JsonLimits(maxBytes:maximum,maxNumberBytes:_encodeLimits.maxNumberBytes));
    case _PartKind.text:return _scalarText(part.value as JsonValue,spec.scalar);
    case _PartKind.style:return _serialize(_Parameter(name,_Location.query,spec.required,spec.serialization!),part.value as JsonValue,maximum);
    case _PartKind.bytes:throw const ConfigurationException('byte parts have no form text mapping');
  }
}
Uint8List _partBytes(String name,_PartSpec spec,_PartData part,int maximum){
  final limit=spec.maxBytes<maximum?spec.maxBytes:maximum;
  return spec.kind==_PartKind.bytes?_bytes(part.value as Uint8List,limit):_textBytes(_partText(name,spec,part,limit),limit);
}
String _formString(_FormSpec spec,Map<String,List<_PartData>> fields,int maximum){
  _structural(spec,fields);final out=_WireBuffer(maximum);
  final keys=fields.keys.toList()..sort(_scalarCompare);
  for(final name in keys){
    final field=spec.fields[name]??spec.extra!;
    for(final value in fields[name]!){
      if(out.length>0){out.add('&');}
      final text=_partText(name,field,value,out.remaining);
      if(field.kind==_PartKind.style){out.add(text);}
      else{out.add(_percent(name,_Encoding.form,out.remaining));out.add('=');out.add(_percent(text,field.encoding,out.remaining));}
    }
  }
  return out.toString();
}
String _formJson(_FormSpec spec,JsonValue value,int maximum){
  if(value is! JsonObject){throw const ConfigurationException('querystring form requires an object');}
  final fields=<String,List<_PartData>>{};
  var parts=0;
  for(final entry in value.values.entries){
    final part=spec.fields[entry.key]??spec.extra;
    if(part==null){throw const ConfigurationException('undeclared form field');}
    if(part.repeated){
      if(entry.value is! JsonArray){throw const ConfigurationException('repeated form value must be an array');}
      parts=_partCount(parts,(entry.value as JsonArray).values.length,maximum);
      fields[entry.key]=(entry.value as JsonArray).values.map(_PartData.new).toList();
    }else{parts=_partCount(parts,1,maximum);fields[entry.key]=[_PartData(entry.value)];}
  }
  return _formString(spec,fields,maximum);
}

String _partMedia(_PartSpec spec,String? supplied){
  if(spec.kind==_PartKind.style){
    if(supplied!=null){throw const ConfigurationException('styled parts do not select contentType');}
    return '';
  }
  final choices=spec.contentTypes.map((m)=>_MediaType.parse(m,ranges:true)).toList();
  final selected=supplied??(choices.length==1&&choices.single.type!='*'&&choices.single.subtype!='*'?spec.contentTypes.single:null);
  if(selected==null){throw const ConfigurationException('part requires an explicit concrete contentType');}
  final actual=_MediaType.parse(selected);
  if(!choices.any((choice)=>choice.matches(actual))){throw const ConfigurationException('undeclared part contentType');}
  if(spec.kind!=_PartKind.bytes&&actual.parameters['charset']!=null&&actual.parameters['charset']!.toLowerCase()!='utf-8'){
    throw const ConfigurationException('part charset must be UTF-8');
  }
  return selected;
}
String _dispositionQuote(String value){
  _unicodeLength(value);
  if(value.codeUnits.any((c)=>c<32||c==127)){throw const ConfigurationException('multipart disposition contains controls');}
  return value.replaceAll('\\','\\\\').replaceAll('"','\\"');
}
int _boundaryCounter=0;
bool _containsBytes(List<int> haystack,List<int> needle){
  outer:for(var i=0;i+needle.length<=haystack.length;i++){
    for(var j=0;j<needle.length;j++){if(haystack[i+j]!=needle[j]){continue outer;}}
    return true;
  }
  return false;
}
_BodyContent _aggregateBody(_FormSpec spec,Map<String,List<_PartData>> fields,String media,int maximum,{required bool multipart}){
  if(!multipart){return _BodyContent(_textBytes(_formString(spec,fields,maximum),maximum),media);}
  _structural(spec,fields);
  final prepared=<(String,_PartSpec,_PartData,Uint8List)>[];
  final keys=fields.keys.toList()..sort(_scalarCompare);
  var payloadBytes=0;
  for(final name in keys){final part=spec.fields[name]??spec.extra!;
    for(final value in fields[name]!){final bytes=_partBytes(name,part,value,maximum-payloadBytes);payloadBytes+=bytes.length;prepared.add((name,part,value,bytes));}}
  final selected=_MediaType.parse(media);
  var boundary=selected.parameters['boundary'];
  if(boundary!=null&&(boundary.isEmpty||boundary.length>70||boundary.endsWith(' ')||boundary.codeUnits.any((c)=>c<32||c>126||c==34||c==92))){throw const ConfigurationException('invalid multipart boundary');}
  bool collision(String boundary)=>prepared.any((part)=>_containsBytes(part.$4,utf8.encode('--$boundary')));
  if(boundary!=null){if(collision(boundary)){throw const ConfigurationException('multipart content collides with declared boundary');}}
  else{
    for(var attempt=0;attempt<16;attempt++){
      _boundaryCounter=(_boundaryCounter+1)%2147483647;
      final candidate='suspect-dart-$_boundaryCounter';
      if(!collision(candidate)){boundary=candidate;break;}
    }
    if(boundary==null){throw const ResourceLimitException('multipart boundary search');}
  }
  final out=BytesBuilder();var size=0;
  void add(List<int> bytes){size+=bytes.length;if(size>maximum){throw const ResourceLimitException('multipart body bytes');}out.add(bytes);}
  void text(String value){add(_textBytes(value,maximum-size));}
  for(final(name,spec,value,bytes)in prepared){
    text('--$boundary\r\n');
    final disposition='form-data; name="${_dispositionQuote(name)}"${value.filename==null?'':'; filename="${_dispositionQuote(value.filename!)}"'}';
    final headers=<String,String>{};
    for(final entry in value.headers.entries){
      final key=entry.key.toLowerCase();
      if(entry.key.isEmpty||!entry.key.codeUnits.every(_tchar)||entry.value.codeUnits.any((c)=>c<32&&c!=9||c==127||c>255)||headers.containsKey(key)){
        throw const ConfigurationException('invalid multipart header');
      }
      headers[key]=entry.value;
    }
    if(headers['content-disposition']!=null&&headers['content-disposition']!=disposition){throw const ConfigurationException('named part disposition must retain its declared field name');}
    headers['content-disposition']=disposition;
    final contentType=_partMedia(spec,value.contentType);
    if(contentType.isNotEmpty){headers['content-type']=contentType;}
    for(final entry in headers.entries){text('${entry.key}: ${entry.value}\r\n');}
    text('\r\n');add(bytes);text('\r\n');
  }
  text('--$boundary--\r\n');
  return _BodyContent(out.takeBytes(),selected.parameters.containsKey('boundary')?media:'$media; boundary=$boundary');
}
