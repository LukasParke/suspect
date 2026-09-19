// A selected API can have no keyed credential attachments.
// ignore_for_file: unused_element_parameter
/// Explicit HTTP Basic credential values. Acquisition belongs to the caller.
final class BasicCredentials {
  const BasicCredentials(this.username,this.password);
  final String username; final String password;
  @override String toString()=>'BasicCredentials(redacted)';
}
/// A complete caller-selected Authorization value for an OAuth/OIDC hook.
final class AuthorizationCredential {
  const AuthorizationCredential(this.value);
  factory AuthorizationCredential.bearer(String token)=>AuthorizationCredential('Bearer ${_bearerToken(token)}');
  final String value;
  @override String toString()=>'AuthorizationCredential(redacted)';
}
typedef CredentialProvider=FutureOr<AuthorizationCredential?> Function(CredentialRequest request);

/// Required scopes/roles and original scheme metadata, with no login/refresh flow.
final class CredentialInfo {
  CredentialInfo(this.name,this.member,this.kind,this.source,Iterable<String> scopes,Iterable<String> roles,this.metadata,{this.urlBase})
      : scopes=List.unmodifiable(scopes),roles=List.unmodifiable(roles);
  final String name; final String member; final String kind; final SchemaSource source;
  final List<String> scopes; final List<String> roles; final JsonObject metadata;
  /// Relative OAuth/OIDC URLs use the selected effective server.
  final String? urlBase;
}
final class CredentialRequest {
  const CredentialRequest(this.requirement,this.operation,this.url,this.cancellation,{this.serverUrl});
  final CredentialInfo requirement; final SchemaSource operation; final Uri url;
  final CancellationToken cancellation;
  /// Selected effective server, before appending the operation path or query.
  final Uri? serverUrl;
}
enum _CredentialKind { bearer, basic, key, authorization }
final class _Requirement {
  const _Requirement(this.info,this.kind,{this.location=_Location.header,this.wireName='authorization'});
  final CredentialInfo info; final _CredentialKind kind; final _Location location; final String wireName;
}

/// One declared server variable, including its source default and enum values.
final class ServerVariable {
  const ServerVariable(this.name,this.defaultValue,{this.values=const []});
  final String name; final String defaultValue; final List<String> values;
}
/// Source server metadata. Variables are literal substitutions, not URI encoding.
final class ServerInfo {
  const ServerInfo(this.template,this.source,{this.variables=const [],this.name,this.description,this.documentBase});
  final String template; final SchemaSource source; final List<ServerVariable> variables;
  final String? name; final String? description;
  /// Physical retrieval-document root, separate from the declaration location.
  final SchemaSource? documentBase;
  String get urlBase=>'server-document';
}
/// Runtime choice of a declared server, or an explicit absolute URL override.
final class ServerSelection {
  const ServerSelection({this.index=0,this.variables=const {},this.documentUrl,this.override});
  final int index; final Map<String,String> variables; final Uri? documentUrl; final Uri? override;
}

Uri _serverUrl(_WireOperation op,ServerSelection selection,int limit){
  if(selection.override!=null){_checkUrl(selection.override!,limit);return selection.override!;}
  if(selection.index<0||selection.index>=op.servers.length){throw const ConfigurationException('server index is not declared');}
  final server=op.servers[selection.index];
  for(final name in selection.variables.keys){if(!server.variables.any((v)=>v.name==name)){throw const ConfigurationException('undeclared server variable override');}}
  var value=server.template;
  for(final variable in server.variables){
    final selected=selection.variables[variable.name]??variable.defaultValue;
    if(variable.values.isNotEmpty&&!variable.values.contains(selected)){throw const ConfigurationException('server variable is outside its declared enum');}
    if(selected.length>limit){throw const ResourceLimitException('server variable');}
    final marker='{${variable.name}}';final count=marker.allMatches(value).length;
    if(value.length+count*(selected.length-marker.length)>limit){throw const ResourceLimitException('expanded server URL');}
    value=value.replaceAll(marker,selected);
  }
  if(value.length>limit||value.contains(RegExp(r'[\s\\{}?#]'))){throw const ConfigurationException('invalid expanded server URL');}
  _checkPercent(value);
  if(_urlParts(value,limit).scheme==null){
    final base=selection.documentUrl?.toString()??server.documentBase?.document??server.source.document;
    final parsed=_urlParts(base,limit);
    if(parsed.scheme==null||!const ['http','https'].contains(parsed.scheme!.toLowerCase())||parsed.authority==null){throw const ConfigurationException('relative server needs an HTTP document URL');}
    value=_resolveUrlText(base,value,limit);
  } else {
    value=_resolveUrlText(value,value,limit);
  }
  final uri=_httpUri(value,limit);
  _checkUrl(uri,limit);return uri;
}
void _checkPercent(String value){
  for(var i=0;i<value.length;i++){if(value.codeUnitAt(i)==37){if(i+2>=value.length||!_hexDigit(value.codeUnitAt(i+1))||!_hexDigit(value.codeUnitAt(i+2))){throw const ConfigurationException('invalid URI percent escape');}i+=2;}}
}
void _checkUrl(Uri uri,int maximum){
  final text=uri.toString();
  if(text.length>maximum){throw const ResourceLimitException('server URL');}
  if(!const ['http','https'].contains(uri.scheme)||!uri.hasAuthority||uri.host.isEmpty||uri.userInfo.isNotEmpty||uri.hasQuery||uri.hasFragment||
      text.codeUnits.any((c)=>c<33||c==127||c==92)) {throw const ConfigurationException('server requires absolute HTTP(S), without userinfo, query or fragment');}
  _checkPercent(text);
}

String _bearerToken(String token){
  var padding=false;
  if(token.isEmpty||token.codeUnitAt(0)==61){throw const ConfigurationException('missing or invalid bearer credential');}
  for(final c in token.codeUnits){
    if(c==61){padding=true;continue;}
    if(padding||!(c>=65&&c<=90||c>=97&&c<=122||c>=48&&c<=57||'-._~+/'.codeUnits.contains(c))){throw const ConfigurationException('missing or invalid bearer credential');}
  }
  return token;
}

final class _RequestBuilder {
  _RequestBuilder(this.op,ServerSelection server,this.maximum):base=_serverUrl(op,server,maximum),path=op.path {
    final media=<String>{};for(final response in op.responses){for(final value in response.media){media.add(value.declared);}}
    if(media.isNotEmpty){header('accept',media.join(', '));}
    header('accept-encoding','identity');
  }
  final _WireOperation op; final Uri base; final int maximum;
  String path; final _WireBuffer query=_WireBuffer(2147483647); final _WireBuffer cookies=_WireBuffer(2147483647);
  final Map<String,String> headers={}; final Set<String> queryNames={}; final Set<String> cookieNames={};
  int headerBytes=2; bool wholeQuery=false;
  void header(String name,String value){
    final lower=name.toLowerCase();
    if(name.isEmpty||!name.codeUnits.every(_tchar)||value.codeUnits.any((c)=>c<32&&c!=9||c==127||c>255)){throw const ConfigurationException('invalid outbound header');}
    if(headers.containsKey(lower)){if(headers[lower]==value){return;}throw const ConfigurationException('conflicting outbound header values');}
    headerBytes+=name.length+value.length+4;if(headerBytes>maximum){throw const ResourceLimitException('request headers');}
    headers[lower]=value;
  }
  void _query(String value,{bool credential=false}){
    if(value.isEmpty){return;}
    for(final part in value.split('&')){
      final raw=part.split('=').first;String name;
      try{name=Uri.decodeComponent(raw);}on FormatException{throw const ConfigurationException('invalid query field');}
      if(credential&&queryNames.contains(name)){throw const ConfigurationException('credential collides with a query parameter');}
      queryNames.add(name);
    }
    if(query.length+value.length+1>maximum){throw const ResourceLimitException('request query');}
    if(query.length>0){query.add('&');}query.add(value);
  }
  void cookie(String value,{bool credential=false}){
    if(value.isEmpty){return;}
    for(final part in value.split(';')){final name=_trimOws(part).split('=').first;
      if(credential&&cookieNames.contains(name)){throw const ConfigurationException('credential collides with a cookie parameter');}cookieNames.add(name);}
    if(cookies.length+value.length+2>maximum){throw const ResourceLimitException('request cookies');}
    if(cookies.length>0){cookies.add('; ');}cookies.add(value);
  }
  void parameter(_Parameter parameter,JsonValue value){
    if(!parameter.required&&!parameter.serialization.content&&(value is JsonArray&&value.values.isEmpty||value is JsonObject&&value.values.isEmpty)){return;}
    final encoded=_serialize(parameter,value,maximum);
    switch(parameter.location){
      case _Location.path:
        final marker='{${parameter.name}}';final count=marker.allMatches(path).length;
        if(path.length+count*(encoded.length-marker.length)>maximum){throw const ResourceLimitException('request path');}
        path=path.replaceAll(marker,encoded);
      case _Location.query:_query(encoded);
      case _Location.querystring:wholeQuery=true;_query(encoded);
      case _Location.header:header(parameter.name,encoded);
      case _Location.cookie:cookie(encoded);
    }
  }
  void body(_BodyContent content){
    if(content.bytes.length>maximum){throw const ResourceLimitException('request body');}
    header('content-type',content.contentType);
  }
  void key(_Location location,String name,String value){
    switch(location){
      case _Location.header:header(name,value);
      case _Location.query:
        if(wholeQuery){throw const ConfigurationException('query credential has no complete-querystring mapping');}
        _query('${_percent(name,_Encoding.component,maximum)}=${_percent(value,_Encoding.component,maximum)}',credential:true);
      case _Location.cookie:cookie('${_percent(name,_Encoding.component,maximum)}=${_percent(value,_Encoding.component,maximum)}',credential:true);
      default:throw const ConfigurationException('unsupported credential attachment');
    }
  }
  Uri url({bool finalize=true}){
    if(path.contains('{')||path.contains('}')){throw const ConfigurationException('unsubstituted path parameter');}
    for(final segment in path.split('/')){
      final value=Uri.decodeComponent(segment);
      if(value=='.'||value=='..'){throw const ConfigurationException('dot segment has no stable path representation');}
    }
    var server=base.toString();if(server.endsWith('/')){server=server.substring(0,server.length-1);}
    if(server.length+path.length+query.length+1>maximum){throw const ResourceLimitException('request URL');}
    if(finalize&&cookies.length>0){header('cookie',cookies.toString());}
    return _httpUri('$server$path${query.length==0?'':'?${query.toString()}'}',maximum);
  }
}

Future<void> _attachCredentials(Credentials values,_WireOperation op,_RequestBuilder builder,CancellationToken cancellation,int? selection) async {
  if(op.security.isEmpty){if(selection!=null){throw const ConfigurationException('security alternative is not declared');}return;}
  if(selection!=null&&(selection<0||selection>=op.security.length)){throw const ConfigurationException('security alternative is not declared');}
  final choices=selection==null?List.generate(op.security.length,(i)=>i):[selection];
  for(final index in choices){
    cancellation.throwIfCancelled();
    final requirements=op.security[index];
    if(requirements.any((r)=>values._get(r.info.member)==null)){continue;}
    final attachments=<( _Location,String,String)>[];var complete=true;
    for(final requirement in requirements){
      final credential=values._get(requirement.info.member)!;
      String? value;
      switch(requirement.kind){
        case _CredentialKind.bearer:
          final token=credential as String;
          if(token.length>builder.maximum){throw const ResourceLimitException('credential bytes');}
          value='Bearer ${_bearerToken(token)}';
        case _CredentialKind.basic:
          final basic=credential as BasicCredentials;
          if(basic.username.contains(':')){throw const ConfigurationException('Basic username cannot contain colon');}
          final raw='${basic.username}:${basic.password}';
          if(raw.length>builder.maximum||_unicodeLength(raw)>builder.maximum){throw const ResourceLimitException('credential bytes');}
          value='Basic ${base64Encode(utf8.encode(raw))}';
        case _CredentialKind.key:value=credential as String;
        case _CredentialKind.authorization:
          final supplied=await (credential as CredentialProvider)(CredentialRequest(requirement.info,op.source,builder.url(finalize:false),cancellation,serverUrl:builder.base));
          value=supplied?.value;
      }
      cancellation.throwIfCancelled();
      if(value==null){complete=false;break;}
      if(value.isEmpty){throw const ConfigurationException('empty credential');}
      if(value.length>builder.maximum){throw const ResourceLimitException('credential bytes');}
      attachments.add((requirement.location,requirement.wireName,value));
    }
    if(!complete){continue;}
    for(final(location,name,value)in attachments){builder.key(location,name,value);}
    return;
  }
  throw const ConfigurationException('no complete declared credential alternative is available');
}
