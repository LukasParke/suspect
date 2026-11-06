import 'dart:convert' show utf8;
import 'package:generated_sdk/generated_sdk.dart';
import 'support.dart';

const _pet = '{"id":1,"name":"rex","tag":null,"owner":{"name":"ann"}}';

/// Local fixture transport: echoes JSON bodies and serves bytes/lists.
final class Oas30Transport implements HttpTransport {
  final List<TransportRequest> requests = [];
  @override
  Future<TransportResponse> send(TransportRequest request) async {
    requests.add(request);
    check(request.headers['authorization'] == 'Bearer oas30-token', 'bearer');
    final path = request.url.path;
    final (String media, List<int> bytes) = path.endsWith('/photo')
        ? ('application/octet-stream', const [0, 255])
        : request.method == 'GET'
        ? ('application/json', utf8.encode('[{"id":1,"name":"rex","tag":null,"owner":null}]'))
        : ('application/json', request.body!.toList());
    return TransportResponse(
      status: 200,
      headers: {
        'content-type': [media],
      },
      body: Stream<List<int>>.fromIterable([bytes]),
      onClose: () async {},
    );
  }

  @override
  Future<void> close() async {}
}

void invalid(void Function() action, String message) {
  check(fails<CodecException>(action).kind == CodecFailureKind.invalid, message);
}

Future<void> main() async {
  // Scalar nullable: the same-object type gains null; required stays required.
  final pet = petCodec.decode(_pet);
  check(pet.tag == null, 'scalar nullable lost');
  check(pet.owner?.name == 'ann', 'reference target lost');
  check(pet.nickname is Absent<String?> && pet.status is Absent<PetStatus>, 'omission lost');
  check(petCodec.encode(pet).contains('"tag":null'), 'explicit null not written');
  invalid(() => petCodec.decode('{"id":1,"name":"rex","owner":null}'), 'required nullable key omitted');
  invalid(() => petCodec.decode('{"id":1,"name":null,"tag":"t","owner":null}'), 'non-nullable null');

  // Object nullable through a Reference Object whose siblings (type: string,
  // readOnly) are ignored: null and objects are accepted, strings are not.
  check(petCodec.decode('{"id":2,"name":"rex","tag":"t","owner":null}').owner == null, 'object nullable lost');
  check(ownerCodec.decode('null') == null, 'nullable object codec');
  invalid(() => petCodec.decode('{"id":2,"name":"rex","tag":"t","owner":"ann"}'), 'ignored type sibling applied');

  // A $ref'd nullable scalar is nullable through its target and keeps minLength.
  final named = petCodec.decode('{"id":3,"name":"rex","tag":"t","owner":null,"nickname":null}');
  check(named.nickname is Present<String?> && (named.nickname as Present<String?>).value == null, 'referenced nullable scalar lost');
  check(nicknameCodec.decode('"r"') == 'r', 'referenced scalar value');
  invalid(() => petCodec.decode('{"id":3,"name":"rex","tag":"t","owner":null,"nickname":""}'), 'target minLength lost');

  // nullable cannot bypass an enum without null; an enum listing null admits it.
  final moody = petCodec.decode('{"id":4,"name":"rex","tag":"t","owner":null,"status":"sold","mood":null}');
  check(moody.status is Present<PetStatus> && (moody.status as Present<PetStatus>).value == PetStatus.sold, 'enum carrier');
  check(moody.mood is Present<PetMood?> && (moody.mood as Present<PetMood?>).value == null, 'nullable enum');
  invalid(() => petCodec.decode('{"id":4,"name":"rex","tag":"t","owner":null,"status":null}'), 'nullable bypassed enum');

  // nullable without a same-object type changes nothing: arbitrary JSON.
  final anything = petCodec.decode('{"id":5,"name":"rex","tag":"t","owner":null,"anything":[1,"x",null]}');
  check(anything.anything is Present<JsonValue>, 'arbitrary JSON lost');

  // Boolean exclusive bounds modify the adjacent bound: 0 < id < 100, 0 < weight <= 50.
  for (final id in ['0', '100', '-1']) {
    invalid(() => petCodec.decode('{"id":$id,"name":"rex","tag":"t","owner":null}'), 'exclusive id bound $id');
  }
  check(petCodec.decode('{"id":99,"name":"rex","tag":"t","owner":null,"weight":50}').weight is Present<JsonNumber>, 'inclusive maximum rejected');
  invalid(() => petCodec.decode('{"id":99,"name":"rex","tag":"t","owner":null,"weight":0}'), 'exclusive weight minimum');
  final mutated = Pet(id: JsonInteger.fromInt(1), name: 'rex', owner: null, tag: null);
  mutated.id = JsonInteger.fromInt(100);
  invalid(() => petCodec.encode(mutated), 'mutable exclusive maximum');

  // The same semantics hold over the generated client.
  final transport = Oas30Transport();
  final client = Client(
    transport: transport,
    credentials: const Credentials(bearer: 'oas30-token'),
    server: Uri.parse('https://example.test/api/v1'),
  );
  final echo = await client.echoPet(
    body: Pet(id: JsonInteger.fromInt(7), name: 'rex', owner: Owner(name: 'ann'), tag: null, nickname: const Present(null)),
  );
  check(echo.data.tag == null && echo.data.owner?.name == 'ann', 'nullable wire round trip');
  check(echo.data.nickname is Present<String?>, 'explicit null presence');
  final listed = await client.listPets(limit: Present(JsonInteger.fromInt(100)));
  check(listed.data.length == 1 && listed.data.single.owner == null, 'list response');
  check(transport.requests[1].url.query == 'limit=100', 'inclusive maximum query');
  await failsAsync<CodecException>(() => client.listPets(limit: Present(JsonInteger.fromInt(0))));
  await failsAsync<CodecException>(
    () => client.echoPet(body: Pet(id: JsonInteger.fromInt(0), name: 'rex', owner: null, tag: null)),
  );
  final photo = await client.getPhoto();
  check(photo.data.length == 2 && photo.data[0] == 0 && photo.data[1] == 255, 'OAS 3.0 binary bytes');
  check(transport.requests.length == 3, 'invalid requests reached the transport');
  await client.close();
  print('DART_OAS30_NULLABLE_BOUNDS_REFERENCE_SIBLINGS_OK');
}
