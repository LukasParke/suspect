import 'dart:async' hide TimeoutException;
import 'dart:convert' show utf8;
import 'dart:typed_data';
import 'package:generated_sdk/generated_sdk.dart';

void check(bool value,String message){if(!value){throw StateError(message);}}
final class EchoTransport implements HttpTransport {
  final requests=<TransportRequest>[];int released=0;
  @override Future<TransportResponse> send(TransportRequest request) async {
    requests.add(request);
    return TransportResponse(status:200,headers:{'content-type':[request.headers['content-type']!]},body:Stream.value(request.body!),onClose:() async{released++;});
  }
  @override Future<void> close() async{}
}
Future<void> main() async {
  final transport=EchoTransport();final client=Client(transport:transport);
  final data=Uint8List.fromList([0,255,32]);
  final binary=await client.legacyBytes(body:data);
  check(binary.data.length==3&&binary.data[1]==255,'explicit profile preserves raw bytes');
  check(transport.requests[0].body![0]==0,'no JSON null/string stand-in');
  final json=await client.jsonString(body:'A\u0000z');
  check(json.data=='A\u0000z','JSON media remains a string');
  check(utf8.decode(transport.requests[1].body!)=='"A\\u0000z"','JSON media keeps JSON encoding');
  check(transport.released==2,'profile responses are released');await client.close();
  print('DART_EXPLICIT_LEGACY_BYTES_AND_JSON_CONTROL_OK');
}
