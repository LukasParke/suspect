import 'dart:async';
import 'package:openrouter/openrouter.dart';

void check(bool ok,String message){if(!ok)throw StateError(message);}
final class Script implements HttpTransport {
  final requests=<TransportRequest>[];int closed=0;int released=0;
  @override Future<TransportResponse> send(TransportRequest request) async {
    check(request.url.toString().startsWith('https://source.example/api/'),'source default HTTPS server');
    requests.add(request);
    return TransportResponse(status:204,headers:const {},body:const Stream.empty(),onClose:() async {released++;});
  }
  @override Future<void> close() async {closed++;check(released==requests.length,'release each response');}
}
Future<void> missing(Future<Object?> Function() call,Script script,String label) async {
  final before=script.requests.length;
  try {await call();throw StateError('$label accepted');}
  on ConfigurationException catch(error) {
    check(error.message.length<256,'bounded error');
    for(final secret in ['env-bearer','native-env','reader-secret','explicit','node-env-must-not-be-read']) {
      check(!error.toString().contains(secret),'secret-free error');
    }
  }
  check(script.requests.length==before,'failure before HTTP: $label');
}
dynamic _dynamicNull() => null;

Future<void> constructorControls() async {
  var nullReads=0;final nullTransport=Script();Client? invalidClient;
  var typeFailure=false;
  try {
    // Deliberately cross a dynamic boundary to exercise the constructor check.
    // ignore: argument_type_not_assignable
    invalidClient=Client(transport:nullTransport,credentials:_dynamicNull(),environment:(name){nullReads++;return 'env-bearer';});
  } on TypeError catch(error) {
    typeFailure=true;
    print('DART_ENV_DYNAMIC_NULL_REJECTED ${error.runtimeType}');
  }
  if(invalidClient!=null){await invalidClient.close();}else{await nullTransport.close();}
  check(typeFailure,'dynamic null must retain the native type error');
  check(nullReads==0&&nullTransport.requests.isEmpty,'dynamic null fails before environment or HTTP');

  var token='constructor-before';var snapshotReads=0;
  String? read(String name){snapshotReads++;return name=='OPENROUTER_API_KEY'?token:null;}
  final firstTransport=Script();final first=Client(transport:firstTransport,environment:read);
  check(snapshotReads==5,'omission sentinel takes one snapshot');token='constructor-after';
  try {await first.getCurrentKey();await first.getCurrentKey();check(firstTransport.requests.every((r)=>r.headers['authorization']=='Bearer constructor-before'),'omitted credentials retain their snapshot');}
  finally{await first.close();}
  check(snapshotReads==5,'no per-request reads');
  final nextTransport=Script();final next=Client(transport:nextTransport,environment:read);
  try {await next.getCurrentKey();check(nextTransport.requests.single.headers['authorization']=='Bearer constructor-after','new omitted client snapshots the new value');}
  finally{await next.close();}
  check(snapshotReads==10,'one snapshot per client');

  for(final credentials in <Credentials>[
    const Credentials(apiKey:'explicit'),const Credentials(apiKey:''),
    const Credentials(apiKey:null),const Credentials(),
    const Credentials(headerKey:'only-explicit-header'),
  ]) {
    var reads=0;final transport=Script();final c=Client(transport:transport,credentials:credentials,environment:(name){reads++;return 'env-bearer';});
    check(reads==0,'whole explicit Credentials object skips environment');
    try {
      await c.anonymous();
      if(credentials.apiKey=='explicit') {await c.getCurrentKey();check(transport.requests.last.headers['authorization']=='Bearer explicit','explicit wins');}
      else {await missing(()=>c.getCurrentKey(),transport,'explicit missing/empty/null member');}
      if(credentials.headerKey!=null) {await c.either();check(transport.requests.last.headers['x-key']=='only-explicit-header'&&!transport.requests.last.headers.containsKey('authorization'),'partial Credentials is not supplemented');}
      await missing(()=>c.together(),transport,'missing explicit members are not filled');
      check(reads==0,'no deferred supplement');
    }finally{await c.close();}
  }
  print('DART_CREDENTIAL_ENV_CONSTRUCTOR_OK');
}

Future<void> constructorDefaultEnvironment(String? expectedToken) async {
  final transport=Script();final client=Client(transport:transport);
  try {
    await client.anonymous();
    if(expectedToken==null){await missing(()=>client.getCurrentKey(),transport,'default unavailable');}
    else {await client.getCurrentKey();check(transport.requests.last.headers['authorization']=='Bearer $expectedToken','omission sentinel uses the VM environment');}
  }finally{await client.close();}
}
Future<void> exercise() async {
  final values=<String,String?>{'OPENROUTER_API_KEY':'env-bearer','DART_ENV_HEADER':'env-header','DART_ENV_QUERY':'q/key','DART_ENV_COOKIE':'cookie/value','DART_ENV_EXTRA':'extra'};
  final reads=<String,int>{};
  String? read(String name){reads.update(name,(n)=>n+1,ifAbsent:()=>1);return values[name];}
  check(reads.isEmpty,'no read before client creation');
  final script=Script();final client=Client(transport:script,environment:read);
  check(reads.length==5&&reads.values.every((n)=>n==1),'one creation-time read per configured variable');
  values['OPENROUTER_API_KEY']='changed-token';
  try {
    await client.getCurrentKey();await client.getCurrentKey();
    check(script.requests.take(2).every((r)=>r.headers['authorization']=='Bearer env-bearer'),'creation snapshot');
    await client.either(securityAlternative:1);check(script.requests.last.headers['x-key']=='env-header','explicit OR choice');
    await client.together();check(script.requests.last.headers['authorization']=='Bearer env-bearer'&&script.requests.last.headers['x-key']=='env-header','AND attachments');
    await client.anonymous();check(!script.requests.last.headers.containsKey('authorization'),'disabled security');
    await client.queryKey();check(script.requests.last.url.toString()=='https://source.example/api/query?token=q%2Fkey','query API key');
    await client.cookieKey();check(script.requests.last.headers['cookie']=='session=cookie%2Fvalue','cookie API key');
    await client.allocated();check(script.requests.last.headers['x-extra']=='extra','allocated scheme member');
    check(reads.values.every((n)=>n==1),'no per-request reads');
  }finally{await client.close();}
  final changed=Script();final newer=Client(transport:changed,environment:read);
  try {await newer.getCurrentKey();check(changed.requests.single.headers['authorization']=='Bearer changed-token','new client takes a new snapshot');}
  finally{await newer.close();}
  check(reads.values.every((n)=>n==2),'snapshot only at each creation');

  for(final mode in ['missing','empty','unavailable','invalid','oversized']) {
    final transport=Script();
    final c=Client(transport:transport,maxRequestBytes:256,environment:(name)=>switch(mode){
      'empty'=>'', 'unavailable'=>throw StateError('reader-secret'),
      'invalid'=>'\r\nreader-secret', 'oversized'=>'x'*257, _=>null,
    });
    try {
      await c.anonymous();await c.optional();
      check(transport.requests.every((r)=>!r.headers.containsKey('authorization')),'anonymous works for $mode');
      await missing(()=>c.getCurrentKey(),transport,mode);
      await missing(()=>c.together(),transport,'AND $mode');
    }finally{await c.close();}
  }
  final alternative=Script();final c=Client(transport:alternative,environment:(name)=>name=='DART_ENV_HEADER'?'only-header':null);
  try {
    await c.either();check(alternative.requests.single.headers['x-key']=='only-header','missing first alternative does not block second');
    await missing(()=>c.either(securityAlternative:0),alternative,'explicit missing alternative');
    await missing(()=>c.together(),alternative,'missing AND member');
  }finally{await c.close();}

  await constructorControls();
  // On portable/JS the default reader is unavailable even if Node has values.
  // The native controls run this section with the process variables absent.
  final noEnvironment=Script();final portable=Client(transport:noEnvironment);
  try {await portable.anonymous();await missing(()=>portable.getCurrentKey(),noEnvironment,'default unavailable');}
  finally{await portable.close();}
  print('DART_CREDENTIAL_ENV_CONTROLS_OK');
}
Future<void> main()=>exercise();
