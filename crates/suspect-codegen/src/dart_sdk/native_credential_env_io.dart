import 'package:openrouter/openrouter_io.dart';
import 'portable.dart' as control;

Future<void> main(List<String> args) async {
  if(args.single=='controls'){await control.exercise();return;}
  // Compile the exact live convenience form without making an account request.
  final native=Client(transport:IoTransport());await native.close();
  final script=control.Script();final client=Client(transport:script);
  try {
    await client.anonymous();
    if(args.single=='positive') {
      await client.getCurrentKey();control.check(script.requests.last.headers['authorization']=='Bearer native-env','real VM process environment');
      await client.together();control.check(script.requests.last.headers['x-key']=='native-header','real header environment');
      await client.queryKey();control.check(script.requests.last.url.query=='token=native%2Fquery','real query environment');
      await client.cookieKey();control.check(script.requests.last.headers['cookie']=='session=native%2Fcookie','real cookie environment');
      await client.allocated();control.check(script.requests.last.headers['x-extra']=='native-extra','allocated real environment');
    } else {
      await control.missing(()=>client.getCurrentKey(),script,args.single);
      await client.optional();
    }
  } finally{await client.close();}
  print('DART_CREDENTIAL_ENV_VM_${args.single.toUpperCase()}_OK');
}
