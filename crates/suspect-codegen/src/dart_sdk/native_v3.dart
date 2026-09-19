import 'dart:async';
import 'dart:convert' show utf8;
import 'package:generated_sdk/generated_sdk.dart';

const strictWire='{"children":[{"data":"child"}],"data":"root"}';
const looseWire='{"children":[{"data":"child","extra":null}],"data":"root"}';
const numberWire='{"note":null,"payload":9007199254740993}';
void check(bool ok,String message){if(!ok)throw StateError(message);}
Tree tree({bool extra=false})=>Tree(data:'root',children:Present([
  JsonObject({'data':JsonString('child'),if(extra)'extra':const JsonNull()}),
]));
Template numbers()=>Template(payload:JsonInteger.parse('9007199254740993'),note:const Present(null));
void invalid(void Function() action,String pointer) {
  try{action();}on CodecException catch(error){check(error.kind==CodecFailureKind.invalid,'invalid vs incomplete');check(error.findings.any((f)=>f.instancePath==pointer),'instance pointer $pointer');return;}
  throw StateError('invalid value accepted');
}
final class Script implements HttpTransport {
  int count=0;int released=0;
  @override Future<TransportResponse> send(TransportRequest request) async {
    final stream=request.url.path.endsWith('/rows');
    final text=stream?'$strictWire\n$looseWire\n':count==4?looseWire:utf8.decode(request.body!);
    if(count==0)check(text==strictWire,'strict wire');
    if(count==1)check(text==looseWire,'unentered strict binding is inert');
    if(count==2)check(text==numberWire,'dynamic scalar type and exact number');
    if(count==3)check(text=='9007199254740993','static external alias bytes');
    count++;
    return TransportResponse(status:200,headers:{'content-type':[stream?'application/x-ndjson':'application/json']},body:Stream.fromIterable(utf8.encode(text).map((b)=>[b])),onClose:() async{released++;});
  }
  @override Future<void> close() async{check(count==6&&released==count,'responses released');}
}
Future<void> exercise(HttpTransport transport,{Uri? server}) async {
  check(strictCodec.encode(tree())==strictWire,'strict native object');
  check(treeCodec.encode(tree(extra:true))==looseWire,'loose native object');
  invalid(()=>strictCodec.encode(tree(extra:true)),'/children/0/extra');
  check(numberEnvelopeCodec.encode(numbers())==numberWire,'contextual carrier native construction');
  final fallback=Template(payload:JsonString('fallback'));
  check(templateCodec.encode(fallback)=='{"payload":"fallback"}','standalone fallback codec');
  invalid(()=>numberEnvelopeCodec.encode(fallback),'/payload');
  final n=numbers();n.note=const Absent();check(!numberEnvelopeCodec.encode(n).contains('note'),'absent remains absent');
  n.payload=const JsonNull();invalid(()=>numberEnvelopeCodec.encode(n),'/payload');
  final malformed=tree();malformed.extraFields['children']=const JsonNull();
  try{strictCodec.encode(malformed);throw StateError('extra collision accepted');}on CodecException catch(error){check(error.kind==CodecFailureKind.conversion,'extra collision');}
  final client=Client(transport:transport,server:server);
  try {
    final Tree strict=(await client.strictTree(body:tree())).data;
    check(strictCodec.encode(strict)==strictWire,'typed strict response');
    final Tree loose=(await client.looseTree(body:tree(extra:true))).data;
    check(treeCodec.encode(loose)==looseWire,'child extras retained');
    final Template scalar=(await client.numberEnvelope(body:numbers())).data;
    check(switch(scalar.payload){JsonNumber(token:'9007199254740993')=>true,_=>false},'native exact scalar override');
    final JsonInteger external=(await client.counter(body:JsonInteger.parse('9007199254740993'))).data;
    check(external.token=='9007199254740993','external counter');
    try{await client.counter(body:JsonInteger.parse('9007199254740992'));throw StateError('invalid counter sent');}
    on CodecException catch(error){check(error.findings.any((f)=>f.source.document=='https://cdn.dart.test/artifacts/counter.json'&&f.source.pointer=='/minimum'),'physical external finding, not logical URI or requested alias');}
    try{await client.strictTree(body:tree(extra:true));throw StateError('invalid child sent');}
    on CodecException catch(error){check(error.findings.any((f)=>f.instancePath=='/children/0/extra'),'pre-send outer binding');}
    try{await client.strictTree(body:tree());throw StateError('invalid remote strict tree accepted');}
    on InvalidResponseException catch(error){check(error.codecFailure!.findings.any((f)=>f.source.pointer=='/components/schemas/Strict/unevaluatedProperties'&&f.instancePath=='/children/0/extra'),'dynamic target finding');}
    var items=0;
    try{await for(final row in client.resourceRows()){items++;check(row.data.data=='root','typed dynamic stream item');}throw StateError('invalid dynamic item accepted');}
    on InvalidResponseException catch(error){check(error.codecFailure?.kind==CodecFailureKind.invalid,'stream scoped rejection');}
    check(items==1,'one valid dynamic item');
  } finally{await client.close();}
  print('DART_V3_SDK_OK');
}
Future<void> main()=>exercise(Script());
