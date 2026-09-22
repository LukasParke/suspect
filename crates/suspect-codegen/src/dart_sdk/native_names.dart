import 'package:generated_sdk/generated_sdk.dart';
import 'support.dart';

Future<void> main() async {
  __MODEL_CASES__
  check(fails<CodecException>(() => impossibleCodec.encode(Impossible(value: 'x', number: JsonInteger.fromInt(1), nothing: null))).kind == CodecFailureKind.invalid,
      'contradictory string/integer/null constants stay checked rather than becoming invalid Dart getters');
  check(fails<CodecException>(() => impossibleCodec.decode('{"value":false,"number":true,"nothing":"x"}')).kind == CodecFailureKind.invalid,
      'contradictory constants cannot bypass the declared type checks');
  final recording = Recording([
    Fixture(method: 'POST', url: __URL__, text: '{"ok":true}'),
    Fixture(method: 'POST', url: __URL__, requestBody: 'null', text: '{"ok":true}'),
    Fixture(method: 'POST', url: __URL__, requestBody: '{"value":"native"}', text: '{"ok":true}'),
    Fixture(method: 'GET', url: 'https://example.test/api/v1/other', text: '{"ok":true}'),
    Fixture(method: 'GET', url: 'https://example.test/api/v1/scalars?enabled=false&amount=1e%2B000008&flags=true&flags=false&counts=1.0,2e0', text: '{"ok":true}'),
  ], token: 'name-token');
  final client = Client(transport: recording, credentials: Credentials(__CREDENTIAL__: 'name-token'));
  await client.__METHOD__(__ARGUMENTS__);
  await client.__METHOD__(__ARGUMENTS__, body: const Present(null));
  final response = await client.__METHOD__(__ARGUMENTS__, body: Present(__BODY__(value: 'native')));
  check(response.data.ok, 'native status class collided safely with its payload model');
  await client.__OTHER_METHOD__();
  await client.echoScalars(enabled: false, amount: JsonNumber.parse('1e+000008'),
      flags: const Present([true, false]), counts: Present([JsonInteger.parse('1.0'), JsonInteger.parse('2e0')]));
  check(recording.requests.length == 5 && recording.responsesClosed == 5, 'source names preserved over HTTP');
  await client.close();

  final bounded = Recording([]);
  final small = Client(transport: bounded, credentials: Credentials(__CREDENTIAL__: 'name-token'), maxRequestBytes: 8192);
  await failsAsync<ResourceLimitException>(() => small.__QUERY_METHOD__(__ARRAY_ARGUMENT__: List.filled(1000, 'x')));
  await failsAsync<ConfigurationException>(() => small.__QUERY_METHOD__(__ARRAY_ARGUMENT__: const []));
  check(bounded.requests.isEmpty, 'repeated long query keys are bounded before transport');
  await small.close();
  print('DART_ADVERSARIAL_NATIVE_NAMES_OPTIONAL_BODY_QUERY_OK');
}
