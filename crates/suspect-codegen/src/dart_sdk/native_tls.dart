import 'dart:async';
import 'dart:io' hide HttpException;
import 'package:openrouter/openrouter_io.dart' as sdk;

void check(bool value,String message){if(!value)throw StateError(message);}
Future<void> main(List<String> args) async {
  final serverContext=SecurityContext()..useCertificateChain(args[0])..usePrivateKey(args[1]);
  final server=await HttpServer.bindSecure(InternetAddress.loopbackIPv4,0,serverContext);
  var received=0;final waiting=Completer<void>();
  final subscription=server.listen((request) async {
    received++;
    if(request.uri.path=='/wait/key'){waiting.complete();return;}
    check(request.headers.value('host')=='localhost:${server.port}','Host authority including port');
    request.response.statusCode=204;await request.response.close();
  },onError:(Object error){ /* Failed TLS attempts must never become HTTP. */ });
  sdk.Client client(String host,{String path=''})=>sdk.Client(transport:sdk.IoTransport(),credentials:const sdk.Credentials(apiKey:'tls-fixture-only'),server:Uri.parse('https://$host:${server.port}$path'),timeout:const Duration(seconds:3));
  try {
    if(args[2]=='untrusted') {
      final c=client('localhost');
      try {await c.getCurrentKey();throw StateError('untrusted certificate accepted');}
      on sdk.TransportException catch(error){check(error.cause is HandshakeException,'TLS trust failure category');}
      finally{await c.close();}
      check(received==0,'untrusted connection reached HTTP');
      print('DART_TLS_UNTRUSTED_REJECTED');return;
    }
    SecurityContext.defaultContext.setTrustedCertificates(args[0]);
    final verified=client('localhost');
    try {final response=await verified.getCurrentKey();check(response.status==204,'verified TLS response');}
    finally{await verified.close();}
    check(received==1,'encrypted HTTP request reached secure server');
    final mismatch=client('127.0.0.1');
    try {await mismatch.getCurrentKey();throw StateError('wrong hostname accepted');}
    on sdk.TransportException catch(error){check(error.cause is HandshakeException,'hostname verification category');}
    finally{await mismatch.close();}
    check(received==1,'wrong-host connection reached HTTP');
    final cancellation=sdk.CancellationToken();final blocked=client('localhost',path:'/wait');
    final call=blocked.getCurrentKey(cancellation:cancellation).then<void>((_)=>throw StateError('cancelled request returned'),onError:(Object error){check(error is sdk.CancelledException,'cancellation category');});
    await waiting.future.timeout(const Duration(seconds:3));cancellation.cancel();await call;await blocked.close();
    print('DART_TLS_VERIFIED_HOSTNAME_AND_CANCEL_OK');
  } finally {await server.close(force:true);await subscription.cancel();}
}
