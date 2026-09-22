// RFC 3986 path resolution over written components. Dart's default Uri decodes
// escaped dots before removing dot segments; HTTP server paths must retain them.
final class _UrlParts {
  const _UrlParts(this.scheme,this.authority,this.path,this.query,this.fragment);
  final String? scheme; final String? authority; final String path;
  final String? query; final String? fragment;
  String get text => '${scheme==null?'':'$scheme:'}${authority==null?'':'//$authority'}$path${query==null?'':'?$query'}${fragment==null?'':'#$fragment'}';
}
_UrlParts _urlParts(String text,int maximum) {
  if(text.length>maximum){throw const ResourceLimitException('URL');}
  if(text.codeUnits.any((v)=>v<=32||v==127||v==92)){throw const ConfigurationException('invalid URL characters');}
  _checkPercent(text);
  final match=RegExp(r'^(?:([A-Za-z][A-Za-z0-9+.-]*):)?(?://([^/?#]*))?([^?#]*)(?:\?([^#]*))?(?:#(.*))?$').firstMatch(text);
  if(match==null){throw const ConfigurationException('invalid URL reference');}
  final path=match.group(3)!;
  if(match.group(1)==null&&match.group(2)==null&&!path.startsWith('/')&&path.split('/').first.contains(':')){throw const ConfigurationException('relative URL first segment contains a colon');}
  return _UrlParts(match.group(1),match.group(2),path,match.group(4),match.group(5));
}
String _removeUrlDots(String path) {
  final parts=path.split('/');final output=<String>[];
  for(var i=0;i<parts.length;i++) {
    final part=parts[i];
    if(part=='.'||part=='..') {
      if(part=='..'&&output.isNotEmpty&&!(output.length==1&&output.first.isEmpty)){output.removeLast();}
      if(i==parts.length-1){output.add('');}
    } else {output.add(part);}
  }
  return output.join('/');
}
String _resolveUrlText(String base,String reference,int maximum) {
  final b=_urlParts(base,maximum);final r=_urlParts(reference,maximum);
  String? scheme=r.scheme;String? authority=r.authority;String path;String? query;
  if(scheme!=null) {path=_removeUrlDots(r.path);query=r.query;}
  else {
    scheme=b.scheme;
    if(authority!=null){path=_removeUrlDots(r.path);query=r.query;}
    else {
      authority=b.authority;
      if(r.path.isEmpty){path=b.path;query=r.query??b.query;}
      else {
        final prefix=b.authority!=null&&b.path.isEmpty?'/':b.path.substring(0,b.path.lastIndexOf('/')+1);
        if(prefix.length+r.path.length>maximum){throw const ResourceLimitException('resolved URL');}
        path=_removeUrlDots(r.path.startsWith('/')?r.path:'$prefix${r.path}');query=r.query;
      }
    }
  }
  final result=_UrlParts(scheme,authority,path,query,r.fragment).text;
  if(result.length>maximum){throw const ResourceLimitException('resolved URL');}
  return result;
}
String _urlUnicode(String value,int maximum) {
  _unicodeLength(value);
  final out=_WireBuffer(maximum);
  for(final scalar in value.runes) {
    final text=String.fromCharCode(scalar);
    out.add(scalar<128?text:_percent(text,_Encoding.component,maximum));
  }
  return out.toString();
}
Uri _httpUri(String text,int maximum) {
  text=_urlUnicode(text,maximum);
  final parts=_urlParts(text,maximum);final parsed=Uri.parse(text);
  if(parts.authority==null||!const ['http','https'].contains(parsed.scheme)||parsed.host.isEmpty||parsed.userInfo.isNotEmpty){throw const ConfigurationException('URL requires absolute HTTP(S) without userinfo');}
  if(!RegExp(r"^[A-Za-z0-9\-._~!$&'()*+,;=:@/%]*$").hasMatch(parts.path)){throw const ConfigurationException('invalid HTTP path characters');}
  // The host/scheme use the standard parser. Path/query/fragment retain their
  // written octets; Uri.removeFragment must not reparse the path in HttpClient.
  return _HttpUri(_UrlParts(parsed.scheme,parsed.authority,parts.path,parts.query,parts.fragment),parsed,maximum);
}

final class _HttpUri implements Uri {
  const _HttpUri(this._parts,this._parsed,this._maximum);
  final _UrlParts _parts;final Uri _parsed;final int _maximum;
  @override String get scheme=>_parsed.scheme;
  @override String get authority=>_parsed.authority;
  @override String get userInfo=>_parsed.userInfo;
  @override String get host=>_parsed.host;
  @override int get port=>_parsed.port;
  @override String get path=>_parts.path;
  @override String get query=>_parts.query??'';
  @override String get fragment=>_parts.fragment??'';
  @override List<String> get pathSegments=>List.unmodifiable((path.startsWith('/')?path.substring(1):path).split('/').where((v)=>path.isNotEmpty).map(Uri.decodeComponent));
  @override Map<String,String> get queryParameters=>_parsed.queryParameters;
  @override Map<String,List<String>> get queryParametersAll=>_parsed.queryParametersAll;
  @override bool get isAbsolute=>hasScheme&&!hasFragment;
  @override bool get hasScheme=>_parsed.hasScheme;
  @override bool get hasAuthority=>true;
  @override bool get hasPort=>_parsed.hasPort;
  @override bool get hasQuery=>_parts.query!=null;
  @override bool get hasFragment=>_parts.fragment!=null;
  @override bool get hasEmptyPath=>path.isEmpty;
  @override bool get hasAbsolutePath=>path.startsWith('/');
  @override String get origin=>_parsed.origin;
  @override bool isScheme(String scheme)=>_parsed.isScheme(scheme);
  @override String toFilePath({bool? windows})=>_parsed.toFilePath(windows:windows);
  @override Null get data=>null;
  @override int get hashCode=>toString().hashCode;
  @override bool operator ==(Object other)=>other is Uri&&toString()==other.toString();
  @override String toString()=>_parts.text;
  @override Uri removeFragment()=>!hasFragment?this:_httpUri(_UrlParts(scheme,authority,path,_parts.query,null).text,_maximum);
  @override Uri normalizePath()=>_httpUri(_UrlParts(scheme,authority,_removeUrlDots(path),_parts.query,_parts.fragment).text,_maximum);
  @override Uri resolve(String reference)=>_httpUri(_resolveUrlText(toString(),reference,_maximum),_maximum);
  @override Uri resolveUri(Uri reference)=>resolve(reference.toString());
  @override Uri replace({String? scheme,String? userInfo,String? host,int? port,String? path,Iterable<String>? pathSegments,String? query,Map<String,dynamic>? queryParameters,String? fragment}) {
    final changed=_parsed.replace(scheme:scheme,userInfo:userInfo,host:host,port:port,path:path,pathSegments:pathSegments,query:query,queryParameters:queryParameters,fragment:fragment);
    return _httpUri(_UrlParts(changed.scheme,changed.authority,path==null&&pathSegments==null?this.path:changed.path,
      query==null&&queryParameters==null?_parts.query:changed.hasQuery?changed.query:null,
      fragment==null?_parts.fragment:changed.fragment).text,_maximum);
  }
}
