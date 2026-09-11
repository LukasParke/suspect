import 'dart:async';
import 'package:generated_sdk/generated_sdk.dart';

const entryOrigin = __ENTRY_ORIGIN__;
const partsOrigin = __PARTS_ORIGIN__;
const partsDocument = '$partsOrigin/retrieved/fragments/parts.json';
final expectedUrls = [
  '$entryOrigin/entry', '$entryOrigin/inherited', '$partsOrigin/empty',
  '$partsOrigin/retrieved/service/%2e%2e/Api%2Fv1/relative',
  '$partsOrigin/retrieved/fragments/service/dots',
  '$entryOrigin/absolute%2fBase/variable',
  '$partsOrigin/retrieved/runtime/%2E/Keep%2fCase/variable',
  '$entryOrigin/service/%2e%2e/Api%2Fv1/relative',
  '$entryOrigin/runtime/api/local',
  '$partsOrigin/retrieved/fragments/authbase/auth',
  '$partsOrigin/retrieved/fragments/authbase/auth',
  '$entryOrigin/chosen/relative',
];
void check(bool condition,String message) { if(!condition)throw StateError(message); }
final class Script implements HttpTransport {
  int seen=0;int released=0;
  @override Future<TransportResponse> send(TransportRequest request) async {
    check(request.url.toString()==expectedUrls[seen], 'URL $seen: ${request.url}');
    check(request.url.removeFragment().toString()==expectedUrls[seen], 'removeFragment preserves encoded path');
    check(request.url.replace(query:'key=a%2Fb').path==request.url.path, 'replace preserves untouched path');
    check(request.url.replace(query:'key=a%2Fb').queryParameters['key']=='a/b','URI query view');
    check(request.url.hashCode==request.url.toString().hashCode,'URI hash');
    if(seen==9||seen==10)check(request.headers['authorization']=='Bearer supplied','caller hook attachment');
    seen++;
    return TransportResponse(status:204,headers:const {},body:const Stream.empty(),onClose:() async {released++;});
  }
  @override Future<void> close() async {check(seen==expectedUrls.length&&released==seen,'complete fixture and releases');}
}
Future<void> exercise(HttpTransport transport) async {
  var hooks=0;
  AuthorizationCredential authorize(CredentialRequest request) {
    hooks++;
    check(request.requirement.urlBase=='effective-server','OAuth/OIDC base classification');
    final server=request.serverUrl!;
    check(server.toString()=='$partsOrigin/retrieved/fragments/authbase/','effective server excludes operation path');
    check(server.resolve('./token').toString()=='$partsOrigin/retrieved/fragments/authbase/token','metadata endpoint base');
    check(request.requirement.source.document==partsDocument,'physical security source');
    check(writeJson(request.requirement.metadata).contains(request.requirement.kind=='oauth2'?'./token':'./discovery'),'original relative endpoint retained');
    return const AuthorizationCredential('Bearer supplied');
  }
  final client=Client(transport:transport,credentials:Credentials(oauth:authorize,oidc:authorize));
  try {
    await client.entry();await client.inherited();await client.empty();
    final relative=await client.relative();
    final link=relative.links.single;
    check(link.server!.documentBase!.document==partsDocument,'link server physical retrieval base');
    check(link.server!.urlBase=='server-document','server URL base classification');
    check(link.server!.source.document==partsDocument,'link source remains physical');
    await client.dots();await client.variable();
    await client.variable(server:const ServerSelection(variables:{'base':'../runtime/%2E/Keep%2fCase'}));
    await client.relative(server:ServerSelection(documentUrl:Uri.parse('$entryOrigin/override/spec.json')));
    try {await client.local();throw StateError('local document guessed an origin');}
    on ConfigurationException { /* Explicit HTTP document URL is required. */ }
    await client.local(server:ServerSelection(documentUrl:Uri.parse('$entryOrigin/runtime/local.json')));
    await client.auth();await client.auth(securityAlternative:1);
    await client.relative(server:ServerSelection(override:Uri.parse('$entryOrigin/chosen')));
    for(final invalid in ['../bad%zz','http://host/path?query','../bad\\path']) {
      try {await client.variable(server:ServerSelection(variables:{'base':invalid}));throw StateError('invalid server override sent');}
      on ConfigurationException { /* Rejected before send. */ }
    }
    check(hooks==2,'caller hooks only, no token/discovery requests');
  } finally {await client.close();}
  print('DART_DOCUMENT_SERVER_OK');
}
Future<void> main()=>exercise(Script());
