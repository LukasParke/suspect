import 'dart:async' hide TimeoutException;
import 'dart:convert' show utf8;
import 'dart:typed_data';
import 'package:generated_sdk/generated_sdk.dart';

void check(bool value,String label){if(!value){throw StateError(label);}}
Future<E> fails<E extends Object>(Future<Object?> Function() run) async {
  try{await run();}on Object catch(error){if(error is E){return error;}rethrow;}
  throw StateError('expected $E');
}
final class Step {
  Step(this.method,this.url,{this.request,this.auth,this.cookie,this.status=200,
    this.media='application/json',this.reply='{"message":"ok"}',this.bytes,this.headers=const {},this.stream});
  final String method;final String url;final List<int>? request;final String? auth;final String? cookie;
  final int status;final String media;final String reply;final List<int>? bytes;
  final Map<String,List<String>> headers;final Stream<List<int>>? stream;
}
final class Script implements HttpTransport {
  Script(this.steps);
  final List<Step> steps;final List<TransportRequest> requests=[];int releases=0;int closes=0;
  @override Future<TransportResponse> send(TransportRequest request) async {
    requests.add(request);check(steps.isNotEmpty,'unplanned transport entry');final step=steps.removeAt(0);
    check(request.method==step.method,'method ${request.method}');check(request.url.toString()==step.url,'URL ${request.url}');
    check(request.headers['authorization']==step.auth,'authorization');check(request.headers['cookie']==step.cookie,'cookie');
    if(step.request!=null){check(_equal(request.body??[],step.request!),'request bytes');}
    return TransportResponse(status:step.status,headers:{'content-type':[step.media],...step.headers},
      body:step.stream??Stream.value(step.bytes??utf8.encode(step.reply)),onClose:() async{releases++;});
  }
  @override Future<void> close() async{closes++;}
}
bool _equal(List<int>a,List<int>b)=>a.length==b.length&&List.generate(a.length,(i)=>a[i]==b[i]).every((v)=>v);
const echoHeaders={'x-count':['1.0'],'x-meta':['level=2,who=ann'],'x-json':['{"ok":true}']};

Future<void> httpCases() async {
  final script=Script([
    Step('POST','https://first.test/base/echo/;id=a%2Fb?labels=x&labels=y&filter%5Ba%5D=2&filter%5Bb%5D=space%20value&allow=a%2Fb',
      request:utf8.encode('{"message":"hello"}'),auth:'Basic dTpw',cookie:'session=abc',status:201,headers:echoHeaders),
    Step('POST','https://first.test/base/echo/;id=x',request:utf8.encode('plain 雪'),auth:'Bearer t',status:201,media:'text/plain',reply:'text reply',headers:echoHeaders),
    Step('POST','https://first.test/base/echo/;id=x',request:[0,255,42],status:202,media:'application/vnd.file',bytes:[0,255,1]),
    Step('POST','https://first.test/base/echo/;id=x',request:utf8.encode('{"message":"x"}'),status:204,media:'ignored',bytes:[]),
    Step('POST','https://first.test/base/echo/;id=x',status:418,reply:'{"message":"denied"}'),
    Step('GET','https://second.test/v3/other'),
    Step('GET','https://first.test/base/query-key?access=a%2Fb%20%2B'),
    Step('GET','https://first.test/base/cookie-key',cookie:'token=a%2Fb'),
    Step('HEAD','https://first.test/base/head',status:204,headers:const{'x-count':['2'],'content-length':['99999']},bytes:[]),
    Step('GET','https://first.test/base/undocumented',media:'image/jpeg',bytes:[0,255,32]),
    Step('GET','https://first.test/base/free',reply:'{"exact":9007199254740993.000000000000000001}'),
    Step('QUERY','https://first.test/base/custom'),Step('PURGE','https://first.test/base/custom'),
    Step('GET','https://first.test/base/search?q=a+b%2B&tags=x&tags=y'),
    Step('POST','https://first.test/base/form',request:utf8.encode('name=a+b&options=%7B%22enabled%22%3Afalse%7D&tags=x&tags=y')),
  ]);
  final client=Client(transport:script,credentials:const Credentials(basic:BasicCredentials('u','p'),headerKey:'key',bearer:'t',queryKey:'a/b +',cookieKey:'a/b'));
  final first=await client.echo(id:'a/b',labels:const Present(['x','y']),filter:Present(EchoParameterN2(a:Present(JsonInteger.fromInt(2)),b:const Present('space value'))),
    allow:const Present('a%2Fb'),xFlags:const Present([true,false]),session:const Present('abc'),body:EchoBodyJson(Request(message:'hello')),securityAlternative:0);
  check(first is EchoStatus201,'exact status class');
  if(first is EchoStatus201){
    check(first.headers.xCount.token=='1.0','typed exact header');
    check(first.headers.xMeta is Present,'typed object header');
    check(first.headers.xJson is Present,'typed content header');
    check(first.links.single.name=='other'&&first.links.single.target=='other','links remain metadata');
    check(first.data is EchoStatus201Json,'JSON media class');
  }
  check(script.requests.first.headers['x-key']=='key'&&script.requests.first.headers['x-flags']=='true,false','AND auth and header styles');
  final text=await client.echo(id:'x',body:EchoBodyText('plain 雪'),securityAlternative:1);
  check(text is EchoStatus201&&text.data is EchoStatus201Text,'text response choice');
  final bytes=await client.echo(id:'x',body:EchoBodyBytes(Uint8List.fromList([0,255,42])),securityAlternative:2);
  check(bytes is EchoStatus2XX&&bytes.status==202&&bytes.data is EchoStatus2XXBytes,'range uses actual status and binary');
  final empty=await client.echo(id:'x',body:EchoBodyJson(Request(message:'x')),securityAlternative:2);
  check(empty is EchoStatus2XX&&empty.status==204&&empty.data is EchoStatus2XXNoBody,'HTTP-mandated absence differs from bytes/null');
  final error=await fails<EchoStatusDefaultApiException>(()=>client.echo(id:'x',body:EchoBodyText('x'),securityAlternative:2));
  check(error.status==418&&error.data is EchoStatusDefaultJson,'default uses actual status for failure');
  await client.other(server:ServerSelection(index:1,variables:const{'version':'v3'},documentUrl:Uri.parse('https://second.test/docs/spec.json')));
  await client.queryKeyCall();await client.cookieKeyCall();
  final head=await client.headCall();check(head.data.runtimeType==NoBody&&head.headers.xCount.toBigInt()==BigInt.two,'HEAD typed headers without body');
  check(_equal((await client.undocumented()).data,[0,255,32]),'undeclared content is bounded bytes');
  final free=await client.freeJson();check(free.data is JsonObject&&writeJson(free.data).contains('9007199254740993.000000000000000001'),'schema-free JSON exact');
  await client.queryMethod();await client.purge();
  await client.search(query:SearchParameterN0ApplicationXWwwFormUrlencoded(q:'a b+',tags:const Present(['x','y'])));
  await client.submitForm(body:SubmitFormBodyFields(name:'a b',tags:const Present(['x','y']),options:Present(SubmitFormBodyApplicationXWwwFormUrlencodedOptions(enabled:const Present(false)))));
  check(script.steps.isEmpty&&script.releases==15,'all response lifetimes released');await client.close();

  final never=Script([]);final guarded=Client(transport:never);
  await fails<ConfigurationException>(()=>guarded.echo(id:'x',allow:const Present('a&injected=1'),body:EchoBodyText('x'),securityAlternative:2));
  await fails<ConfigurationException>(()=>guarded.other(server:const ServerSelection(index:1,variables:{'version':'bad'})));
  await fails<ConfigurationException>(()=>guarded.other(server:const ServerSelection(index:1)));
  await fails<ConfigurationException>(()=>guarded.echo(id:'x',session:const Present('bad;cookie'),body:EchoBodyText('x'),securityAlternative:2));
  await fails<ConfigurationException>(()=>guarded.submitForm(body:SubmitFormBodyFields(name:'x',tags:const Present(['1','2','3','4']))));
  check(never.requests.isEmpty,'invalid representations never enter transport');await guarded.close();
}

Future<void> mediaAndBoundaryCases() async {
  for(final step in [
    Step('POST','https://first.test/base/echo/;id=x',status:201,media:'application/vnd.file',bytes:[1]),
    Step('POST','https://first.test/base/echo/;id=x',status:201,media:'text/plain;charset=utf-16',headers:echoHeaders),
    Step('POST','https://first.test/base/echo/;id=x',status:201,media:'application/json;',headers:echoHeaders),
  ]){
    final script=Script([step]);final client=Client(transport:script);
    await fails<MediaTypeException>(()=>client.echo(id:'x',body:EchoBodyText('x'),securityAlternative:2));
    check(script.releases==1,'media failure closes unread body');await client.close();
  }
  final missing=Script([Step('POST','https://first.test/base/echo/;id=x',status:201)]);final client=Client(transport:missing);
  await fails<InvalidResponseException>(()=>client.echo(id:'x',body:EchoBodyText('x'),securityAlternative:2));await client.close();
  final bounded=Script([Step('GET','https://first.test/base/undocumented',bytes:List.filled(1024,1))]);
  final small=Client(transport:bounded,maxResponseBytes:32,maxCaptureBytes:8);
  final failure=await fails<ResourceLimitException>(()=>small.undocumented());check(failure.response!.body.length==8&&failure.response!.truncated,'bounded raw bytes');await small.close();
}

Future<void> remainingStylesAndMedia() async {
  final script=Script([
    Step('GET','https://first.test/base/styles/.a%20b.c/;first=ann;role=admin?space=1.0%202&pipe=x%7Cy&object=a,z,b,false'),
    Step('GET','https://first.test/base/content?options=%7B%22message%22%3A%22json%20value%22%7D',headers:const{'x-number':['1e+000008']}),
    Step('GET','https://first.test/base/negotiate',media:'application/problem+json; profile=full',reply:'{"message":"full"}'),
    Step('GET','https://first.test/base/negotiate',media:'application/problem+json; profile=full',reply:'{"message":"wrong"}'),
    Step('GET','https://first.test/base/negotiate',media:'application/problem+json',reply:'{"message":"plain"}'),
    Step('GET','https://first.test/base/negotiate',media:'text/html; charset=iso-8859-1',bytes:[0,255,65]),
    Step('get','https://first.test/base/custom'),
  ]);
  final client=Client(transport:script);
  await client.styles(label:['a b','c'],matrix:StylesParameterN1(first:const Present('ann'),role:const Present('admin')),
    space:Present([JsonInteger.parse('1.0'),JsonInteger.fromInt(2)]),pipe:const Present(['x','y']),object:Present(StylesParameterN4(a:const Present('z'),b:const Present(false))));
  final header=await client.contentQuery(options:Request(message:'json value'),xOn:const Present(false));
  check(script.requests[1].headers['x-on']=='false'&&header.headers.xNumber.token=='1e+000008','content parameter/header representations');
  check((await client.negotiate()).data is NegotiateStatus200Json2,'most-specific media parameters');
  await fails<InvalidResponseException>(()=>client.negotiate());
  check((await client.negotiate()).data is NegotiateStatus200Json,'base concrete media');
  final bytes=await client.negotiate();check(bytes.data is NegotiateStatus200Bytes2,'type wildcard stays bytes even for text');
  await client.lowerGet();
  await fails<ConfigurationException>(()=>client.styles(label:['x'],matrix:StylesParameterN1(first:const Present('a')),pipe:const Present(['a|b'])));
  await fails<ConfigurationException>(()=>client.echo(id:'x',body:EchoBodyBytes(Uint8List(0),contentType:'application/json'),securityAlternative:2));
  check(script.requests.length==7,'invalid style/media controls do not send');await client.close();
}

Future<void> hooks() async {
  var oauthCalls=0;var oidcCalls=0;
  final script=Script([Step('GET','https://first.test/base/oauth',auth:'Bearer supplied'),Step('GET','https://first.test/base/oidc',auth:'Custom provided')]);
  final client=Client(transport:script,credentials:Credentials(oauth:(request) async {
    oauthCalls++;check(request.requirement.scopes.single=='read'&&request.requirement.kind=='oauth2','required OAuth metadata');
    check(writeJson(request.requirement.metadata).contains('https://auth.test/token'),'declared flow metadata');
    return AuthorizationCredential.bearer('supplied');
  },oidc:(request){oidcCalls++;check(writeJson(request.requirement.metadata).contains('openid-configuration'),'OIDC discovery metadata');return const AuthorizationCredential('Custom provided');}));
  await client.oauthCall();await client.oidcCall();check(oauthCalls==1&&oidcCalls==1&&script.requests.length==2,'hooks do not acquire or retry');await client.close();
  final pending=Completer<AuthorizationCredential?>();final started=Completer<void>();final unused=Script([]);
  final cancellation=CancellationToken();final late=Client(transport:unused,credentials:Credentials(oauth:(request){started.complete();return pending.future;}));
  final outcome=fails<CancelledException>(()=>late.oauthCall(cancellation:cancellation));await started.future;cancellation.cancel();await outcome;
  pending.complete(AuthorizationCredential.bearer('late'));await Future<void>.delayed(Duration.zero);
  check(unused.requests.isEmpty,'cancelled credential hook cannot send a late request');await late.close();
}

Future<void> multipartCase() async {
  final script=Script([Step('POST','https://first.test/base/multipart')]);final client=Client(transport:script);
  final file=UploadBodyFieldsFilePart(value:Uint8List.fromList([0,255,13,10]),filename:'a"b.bin',headers:UploadBodyFieldsFileHeaders(xPart:JsonInteger.parse('1.0')));
  await client.upload(body:UploadBodyFields(file:file,title:'title',meta:Present(Request(message:'part'))));
  final sent=script.requests.single;final media=sent.headers['content-type']!;final boundary=media.split('boundary=').last;
  final expected=<int>[...utf8.encode('--$boundary\r\nx-part: 1.0\r\ncontent-disposition: form-data; name="file"; filename="a\\"b.bin"\r\ncontent-type: application/octet-stream\r\n\r\n'),0,255,13,10,
    ...utf8.encode('\r\n--$boundary\r\ncontent-disposition: form-data; name="meta"\r\ncontent-type: application/json\r\n\r\n{"message":"part"}\r\n--$boundary\r\ncontent-disposition: form-data; name="title"\r\ncontent-type: text/plain\r\n\r\ntitle\r\n--$boundary--\r\n')];
  check(_equal(sent.body!,expected),'hand-authored multipart bytes');
  file.value=Uint8List(17);await fails<ResourceLimitException>(()=>client.upload(body:UploadBodyFields(file:file,title:'title')));
  check(script.requests.length==1,'mutated byte part revalidated');await client.close();
}

Future<void> framingCases() async {
  final eventBytes=utf8.encode('\ufeff: ignored\r\ndata: {"n":1}\r\ndata: 雪\r\nevent: update\r\nid: event-id\r\nretry: 00005\r\nunknown: nope\r\n\r\ndata: [DONE]\n\ndata: after\nid: bad\u0000id\nretry: -1\n\ndata: no eof delimiter');
  final events=Script([Step('GET','https://first.test/base/events',media:'text/event-stream',stream:Stream.fromIterable(eventBytes.map((b)=>[b])))]);
  final client=Client(transport:events);final values=await client.events().toList();
  check(values.length==3,'SSE dispatch, ignored fields, EOF and no sentinel');
  check(values[0].data.data=='{"n":1}\n雪','SSE data stays a string');
  check(switch(values[0].data.retry){Present(value:final v)=>v.token=='5',_=>false},'retry is exact integral metadata');
  check(values[1].data.retry is Absent&&values[1].data.event is Absent&&values[1].data.id is Absent,'absent envelope fields stay absent');
  check(values[1].data.data=='[DONE]'&&values[2].data.data=='after','no invented sentinel');
  check(values[2].data.id is Absent&&events.releases==1,'ignored invalid id and release');await client.close();

  final rows=Script([Step('GET','https://first.test/base/rows',media:'application/x-ndjson',reply:'{"count":9007199254740993}\r\n{"count":1e+000008}\n{"count":1.0}')]);
  final rowClient=Client(transport:rows);final data=await rowClient.rows().toList();
  check(data.map((v)=>v.data.count.token).join('|')=='9007199254740993|1e+000008|1.0','JSON lines retain integer lexemes');await rowClient.close();
  for(final bytes in [utf8.encode('data: ${'x'*1025}\n\n'),[100,97,116,97,58,32,0xed,0xa0,0x80,10,10]]){
    final script=Script([Step('GET','https://first.test/base/events',media:'text/event-stream',bytes:bytes)]);final c=Client(transport:script);
    if(bytes.length>1024){await fails<ResourceLimitException>(()=>c.events().toList());}else{await fails<InvalidResponseException>(()=>c.events().toList());}
    check(script.releases==1,'framing failure cleanup');await c.close();
  }
  final error=Script([Step('GET','https://first.test/base/events',status:400,reply:'{"message":"stream denied"}')]);final e=Client(transport:error);
  final denied=await fails<EventsStatus400>(()=>e.events().toList());check(denied.data.message=='stream denied','stream errors use declared payload codec');await e.close();
}

Future<void> listenerLifecycle() async {
  for(final closing in [false,true]){
    final input=StreamController<List<int>>();final started=Completer<void>();var cancelled=0;
    input.onListen=(){started.complete();};input.onCancel=(){cancelled++;};
    final script=Script([Step('GET','https://first.test/base/events',media:'text/event-stream',stream:input.stream)]);
    final client=Client(transport:script);
    final pending=fails<SdkException>(()=>client.events(timeout:const Duration(milliseconds:30)).toList());
    await started.future;if(closing){await client.close();}
    final error=await pending;
    check(closing?error is ClientClosedException:error is TimeoutException,'stream close/deadline classification');
    check(cancelled==1&&script.releases==1,'stalled stream releases resources');await input.close();await client.close();
  }
  final input=StreamController<List<int>>();var inputCancelled=0;input.onCancel=(){inputCancelled++;};
  final started=Completer<void>();input.onListen=(){started.complete();};
  final script=Script([Step('GET','https://first.test/base/events',media:'text/event-stream',stream:input.stream)]);
  final client=Client(transport:script);final stream=client.events();check(script.requests.isEmpty,'stream is lazy until subscribed');
  final first=Completer<void>();var count=0;late StreamSubscription<EventsStatus200> subscription;
  subscription=stream.listen((value){count++;subscription.pause();first.complete();});
  await started.future;input.add(utf8.encode('data: first\n\ndata: second\n\n'));await first.future;
  await Future<void>.delayed(const Duration(milliseconds:5));check(count==1,'listener pause prevents queued parsed items');
  await subscription.cancel();check(inputCancelled==1&&script.releases==1,'listener stop closes subscription and response');await input.close();await client.close();

  final pending=Completer<TransportResponse>();var lateClosed=0;final transport=Late(pending);
  final c=Client(transport:transport);final stop=CancellationToken();
  final result=fails<CancelledException>(()=>c.events(cancellation:stop).toList());await transport.started.future;stop.cancel();await result;
  pending.complete(TransportResponse(status:200,headers:const{'content-type':['text/event-stream']},body:const Stream.empty(),onClose:() async{lateClosed++;}));
  await Future<void>.delayed(Duration.zero);check(lateClosed==1,'late stream response cleanup');await c.close();
  final unstarted=Script([]);final lazy=Client(transport:unstarted);final sub=lazy.events().listen((_){});await sub.cancel();check(unstarted.requests.isEmpty,'immediate listener cancellation prevents request');await lazy.close();
}
final class Late implements HttpTransport {
  Late(this.pending);final Completer<TransportResponse> pending;final started=Completer<void>();
  @override Future<TransportResponse> send(TransportRequest request){started.complete();return pending.future;}
  @override Future<void> close() async{}
}

Future<void> main() async {
  await httpCases();await mediaAndBoundaryCases();await remainingStylesAndMedia();await hooks();await multipartCase();await framingCases();await listenerLifecycle();
  print('DART_RICH_PORTABLE_PROTOCOL_OK');
}
