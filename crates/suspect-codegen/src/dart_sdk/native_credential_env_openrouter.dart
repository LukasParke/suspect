import 'dart:async';
import 'dart:convert' show utf8;
import 'package:openrouter/openrouter.dart';

const keyReply='{"data":{"label":"controlled fixture","limit":null,"usage":0.125,"usage_daily":0,"usage_weekly":0,"usage_monthly":0,"byok_usage":0,"byok_usage_daily":0,"byok_usage_weekly":0,"byok_usage_monthly":0,"is_free_tier":false,"is_management_key":false,"is_provisioning_key":false,"limit_remaining":null,"limit_reset":null,"include_byok_in_limit":false,"creator_user_id":null,"rate_limit":{"requests":-1,"interval":"unused","note":"controlled fixture"}}}';
const creditsReply='{"data":{"total_credits":9007199254740993.0000000001,"total_usage":0.125}}';
void check(bool condition,String message){if(!condition)throw StateError(message);}
final class Capture implements HttpTransport {
  Capture(this.token);
  final String token;int requests=0;int releases=0;
  @override Future<TransportResponse> send(TransportRequest request) async {
    check(request.method=='GET','source method');
    check(request.url.toString()=='https://openrouter.ai/api/v1/key'||request.url.toString()=='https://openrouter.ai/api/v1/credits','source-default HTTPS URL');
    check(request.headers['authorization']=='Bearer $token','source bearer attachment from runtime value');requests++;
    return TransportResponse(status:200,headers:const {'content-type':['application/json']},body:Stream.value(utf8.encode(request.url.path.endsWith('/key')?keyReply:creditsReply)),onClose:() async {releases++;});
  }
  @override Future<void> close() async {check(requests==releases,'release');}
}
Future<void> calls(Client client) async {
  final key=await client.getCurrentKey();check(key.status==200&&key.data.data.usage.token=='0.125'&&!key.data.data.isManagementKey,'decoded actual key schema');
  final credits=await client.getCredits();check(credits.status==200&&credits.data.data.totalCredits.token=='9007199254740993.0000000001','decoded actual credits schema');
}
Future<void> realEnvironment() async {
  final transport=Capture('native-openrouter');final client=Client(transport:transport);
  try {await calls(client);}finally{await client.close();}
  print('DART_OPENROUTER_ENV_VM_OK');
}
Future<void> unavailable() async {
  final transport=Capture('never');final client=Client(transport:transport);
  try {try {await client.getCurrentKey();throw StateError('missing env accepted');}on ConfigurationException catch(error){check(error.message.length<256&&!error.toString().contains('node-env-must-not-be-read'),'bounded secret-free missing error');}check(transport.requests==0,'before HTTP');}
  finally{await client.close();}
  final explicitTransport=Capture('explicit-openrouter');final explicit=Client(transport:explicitTransport,credentials:const Credentials(apiKey:'explicit-openrouter'),environment:(_)=>throw StateError('must not read'));
  try {await calls(explicit);}finally{await explicit.close();}
  print('DART_OPENROUTER_ENV_UNAVAILABLE_OK');
}
Future<void> main() async {
  await unavailable();
  var reads=0;var value='portable-openrouter';final transport=Capture(value);
  final client=Client(transport:transport,environment:(name){check(name=='OPENROUTER_API_KEY','configured variable name');reads++;return value;});
  value='changed';try {await calls(client);check(reads==1,'portable creation-time snapshot');}finally{await client.close();}
  print('DART_OPENROUTER_ENV_PORTABLE_OK');
}
