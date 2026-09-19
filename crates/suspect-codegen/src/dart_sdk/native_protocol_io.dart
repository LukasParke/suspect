import 'dart:async' hide TimeoutException;
import 'dart:io' show Platform,File;
import 'dart:typed_data';
import 'package:generated_sdk/generated_sdk_io.dart';
import 'portable.dart' as portable;

Future<void> main() async {
  await portable.main();
  final root=Platform.environment['SUSPECT_DART_HTTP_MARKERS']!;
  final server=Uri.parse(Platform.environment['SUSPECT_DART_HTTP_BASE']!);
  final client=Client(transport:IoTransport(),server:server,credentials:const Credentials(basic:BasicCredentials('u','p'),headerKey:'key',queryKey:'q/k',cookieKey:'cookie'));
  try{
    final json=await client.echo(id:'a/b 雪',body:EchoBodyJson(Request(message:'hello')),securityAlternative:0);
    portable.check(json is EchoStatus201,'real JSON response status');
    await client.echo(id:'text',body:EchoBodyText('hello 雪'),securityAlternative:2);
    final bytes=await client.echo(id:'bytes',body:EchoBodyBytes(Uint8List.fromList([0,255,4])),securityAlternative:2);
    portable.check(bytes is EchoStatus2XX&&bytes.status==202,'real binary range');
    await client.queryKeyCall();await client.cookieKeyCall();await client.headCall();await client.undocumented();
    await client.submitForm(body:SubmitFormBodyFields(name:'form value',tags:const Present(['x','y'])));
    await client.upload(body:UploadBodyFields(file:UploadBodyFieldsFilePart(value:Uint8List.fromList([0,255,13,10]),headers:UploadBodyFieldsFileHeaders(xPart:JsonInteger.parse('1.0'))),title:'real upload'));
    await client.search(query:SearchParameterN0ApplicationXWwwFormUrlencoded(q:'a b',tags:const Present(['x','y'])));
    await client.queryMethod();await client.purge();await client.lowerGet();
    portable.check((await client.lowerGet()).data.message=='chunked','exact custom method handles interim/chunked framing');
    await portable.fails<TimeoutException>(()=>client.lowerGet(timeout:const Duration(milliseconds:60)));
    final exactMarker=File('$root/exact-peer-closed');
    for(var i=0;i<200&&!exactMarker.existsSync();i++){await Future<void>.delayed(const Duration(milliseconds:5));}
    portable.check(exactMarker.existsSync(),'custom-method socket cancelled before response headers');
    final events=await client.events().toList();portable.check(events.length==2&&events.last.data.data=='[DONE]','real SSE bytes');
    final rows=await client.rows().toList();portable.check(rows.last.data.count.token=='1e+000008','real exact JSON lines');
    final first=await client.events().take(1).toList();portable.check(first.single.data.data=='cancel me','listener cancellation after item');
    final marker=File('$root/event-peer-closed');
    for(var i=0;i<200&&!marker.existsSync();i++){await Future<void>.delayed(const Duration(milliseconds:5));}
    portable.check(marker.existsSync(),'fixture observed stream peer close');
  }finally{await client.close();}
  print('DART_RICH_NATIVE_IO_OK');
}
