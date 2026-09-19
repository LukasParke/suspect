import 'dart:async';
import 'dart:convert' show utf8;
import 'package:generated_sdk/generated_sdk.dart';

// An independent source/wire oracle, never obtained by encoding the tested SDK.
const wireEnvelope =
    '{"flexible":null,"patch":{"billing":null,"card":null,"kind":"card","x-n":9007199254740993,"x-null":null},"sequence":["s",9007199254740993,1e+000008],"stamp":null}';
const wireMixed =
    '{"s-required":"r","s-title":"t","s-null":null,"count":9007199254740993}';

void check(bool condition, String message) {
  if (!condition) throw StateError(message);
}

Envelope nativeEnvelope() => Envelope(
  flexible: Flexible.fromJson(const JsonNull()),
  patch: Patch(
    kind: 'card',
    xN: JsonInteger.parse('9007199254740993'),
    card: const Present(null),
    billing: const Present(null),
    extraFields: {'x-null': const JsonNull()},
  ),
  sequence: Sequence.fromJson(
    JsonArray([
      JsonString('s'),
      JsonInteger.parse('9007199254740993'),
      JsonInteger.parse('1e+000008'),
    ]),
  ),
  stamp: null,
);
MixedExtras nativeMixed() => MixedExtras(
  sRequired: JsonString('r'),
  extraFields: {
    's-title': JsonString('t'),
    's-null': const JsonNull(),
    'count': JsonInteger.parse('9007199254740993'),
  },
);

CodecException rejects(
  void Function() action,
  CodecFailureKind kind, [
  String? path,
]) {
  try {
    action();
  } on CodecException catch (error) {
    check(error.kind == kind, 'codec outcome: ${error.kind}, expected $kind');
    if (path != null)
      check(
        error.findings.any((f) => f.instancePath == path),
        'located instance: $path',
      );
    return error;
  }
  throw StateError('expected codec failure');
}

final class Fixture implements HttpTransport {
  final requests = <TransportRequest>[];
  int releases = 0;
  int closes = 0;
  bool invalid = false;
  @override
  Future<TransportResponse> send(TransportRequest request) async {
    requests.add(request);
    final stream = request.url.path.endsWith('/rows');
    final text = stream
        ? '$wireEnvelope\n{"stamp":null}\n'
        : invalid
        ? '{"stamp":null}'
        : request.body == null
        ? 'null'
        : utf8.decode(request.body!);
    return TransportResponse(
      status: 200,
      headers: {
        'content-type': [stream ? 'application/x-ndjson' : 'application/json'],
      },
      body: Stream.fromIterable(utf8.encode(text).map((v) => [v])),
      onClose: () async {
        releases++;
      },
    );
  }

  @override
  Future<void> close() async {
    closes++;
  }
}

Future<void> exercisePortable() async {
  final input = nativeEnvelope();
  check(
    envelopeCodec.encode(input) == wireEnvelope,
    'native field/extra/exact-token wire bytes',
  );
  final decoded = envelopeCodec.decode(wireEnvelope);
  final Patch? optionalPatch = decoded.patch;
  check(optionalPatch != null, 'source nullable object is present');
  final patch = optionalPatch!;
  check(
    patch.card is Present<String?> &&
        (patch.card as Present<String?>).value == null,
    'card null is present',
  );
  check(patch.billing is Present<String?>, 'dependent billing null retained');
  check(patch.enabled is Absent<bool>, 'optional Boolean absence');
  check(
    patch.extraFields['x-null'] is JsonNull,
    'pattern-matched nullable extra survives closed additionalProperties',
  );
  check(switch (patch.xN) {
    JsonNumber(token: '9007199254740993') => true,
    _ => false,
  }, 'required pattern-only native carrier');
  check(decoded.optional is Absent<String?>, 'schema default is not inserted');
  check(
    decoded.flexible.value is JsonNull,
    'checked carrier contains JSON null',
  );
  check(
    envelopeCodec.encode(decoded) == wireEnvelope,
    'lossless decode/re-encode',
  );

  input.optional = const Present(null);
  check(
    envelopeCodec.encode(input).contains('"optional":null'),
    'absent and present-null stay distinct',
  );
  input.optional = const Absent();
  final current = input.patch!;
  current.billing = const Absent();
  final dependency = rejects(
    () => envelopeCodec.encode(input),
    CodecFailureKind.invalid,
    '/patch',
  );
  check(
    dependency.findings.any(
      (f) => f.source.pointer.endsWith('/dependentRequired/card'),
    ),
    'dependentRequired source',
  );
  current.billing = const Present(null);
  current.enabled = const Present(true);
  rejects(
    () => envelopeCodec.encode(input),
    CodecFailureKind.invalid,
    '/patch',
  );
  current.note = const Present(null);
  envelopeCodec.encode(input);
  current.enabled = const Absent();
  current.note = const Absent();
  current.extraFields['x-n'] = JsonInteger.fromInt(1);
  rejects(() => envelopeCodec.encode(input), CodecFailureKind.conversion);
  current.extraFields.remove('x-n');
  current.extraFields['other'] = const JsonNull();
  rejects(
    () => envelopeCodec.encode(input),
    CodecFailureKind.invalid,
    '/patch/other',
  );
  current.extraFields.remove('other');
  current.xN = const JsonNull();
  rejects(
    () => envelopeCodec.encode(input),
    CodecFailureKind.invalid,
    '/patch/x-n',
  );
  current.xN = JsonInteger.parse('9007199254740993');
  rejects(
    () => sequenceCodec.decode('["s",1,false]'),
    CodecFailureKind.invalid,
    '/2',
  );
  rejects(
    () => sequenceCodec.decode('["s",1,2,3]'),
    CodecFailureKind.invalid,
    '',
  );
  rejects(
    () => Flexible.fromJson(JsonString('not null or a number array')),
    CodecFailureKind.invalid,
  );
  final source = <String, JsonValue>{'s-title': JsonString('owned')};
  final owned = MixedExtras(sRequired: const JsonNull(), extraFields: source);
  source['s-title'] = JsonString('changed');
  check(switch (owned.extraFields['s-title']) {
    JsonString(value: 'owned') => true,
    _ => false,
  }, 'caller map ownership');
  check(
    mixedExtrasCodec.encode(nativeMixed()) == wireMixed,
    'patterned extras are not narrowed to additionalProperties integers',
  );
  final mixed = mixedExtrasCodec.decode(wireMixed);
  check(mixed.extraFields['s-null'] is JsonNull, 'pattern-null retained');
  mixed.extraFields['count'] = JsonString('bad integer');
  rejects(
    () => mixedExtrasCodec.encode(mixed),
    CodecFailureKind.invalid,
    '/count',
  );

  final recursive = Recursive();
  recursive.next = Present(recursive);
  rejects(() => recursiveCodec.encode(recursive), CodecFailureKind.conversion);
  JsonValue deep = JsonObject({});
  for (var i = 0; i < 160; i++) {
    deep = JsonObject({'next': deep});
  }
  final depth = recursiveCodec.validate(deep);
  check(
    depth.status == ValidationStatus.evaluationFailure &&
        depth.findings.single.message.contains('depth'),
    'bounded productive recursion',
  );
  final immutable = Sequence.fromJson(
    JsonArray([JsonString('s'), JsonInteger.fromInt(1)]),
  );
  var immutableRejected = false;
  try {
    (immutable.value as JsonArray).values.add(const JsonNull());
  } on UnsupportedError {
    immutableRejected = true;
  }
  check(immutableRejected, 'checked carrier owns immutable JSON');

  final fixture = Fixture();
  final client = Client(transport: fixture);
  try {
    final Envelope result = (await client.scopedEcho(body: input)).data;
    check(
      envelopeCodec.encode(result) == wireEnvelope,
      'real Future operation model path',
    );
    check(fixture.requests.single.method == 'POST', 'operation method');
    final absent = await client.optionalPatch();
    final presentNull = await client.optionalPatch(
      body: const Present<Patch?>(null),
    );
    check(
      absent.data == null && presentNull.data == null,
      'nullable response type',
    );
    check(fixture.requests[1].body == null, 'absent request body');
    check(
      utf8.decode(fixture.requests[2].body!) == 'null',
      'present JSON-null request body',
    );
    final MixedExtras extras = (await client.mixedExtras(
      body: nativeMixed(),
    )).data;
    check(
      mixedExtrasCodec.encode(extras) == wireMixed,
      'actual patterned extras operation',
    );
    final calls = fixture.requests.length;
    current.billing = const Absent();
    try {
      await client.scopedEcho(body: input);
      throw StateError('invalid native request sent');
    } on CodecException catch (error) {
      check(error.kind == CodecFailureKind.invalid, 'pre-send schema failure');
    }
    check(fixture.requests.length == calls, 'invalid model prevents HTTP');
    current.billing = const Present(null);
    fixture.invalid = true;
    try {
      await client.scopedEcho(body: input);
      throw StateError('invalid response accepted');
    } on InvalidResponseException catch (error) {
      check(
        error.codecFailure?.kind == CodecFailureKind.invalid,
        'response schema rejection',
      );
      check(
        error.codecFailure!.findings.any(
          (f) => f.source.pointer.endsWith('/Envelope/required'),
        ),
        'original response schema source',
      );
    }
    fixture.invalid = false;
    final Stream<ScopedRowsStatus200> lazy = client.scopedRows();
    final beforeListen = fixture.requests.length;
    check(beforeListen == calls + 1, 'stream remains lazy');
    var seen = 0;
    try {
      await for (final row in lazy) {
        seen++;
        check(row.data.stamp == null, 'v2 item model');
      }
      throw StateError('invalid second item accepted');
    } on InvalidResponseException catch (error) {
      check(
        error.codecFailure?.kind == CodecFailureKind.invalid,
        'item schema failure',
      );
    }
    check(seen == 1, 'valid item before stream failure');
    final firstOnly = await client.scopedRows().take(1).toList();
    check(
      firstOnly.length == 1,
      'cancellation skips an unrequested invalid item',
    );
    check(
      fixture.releases == fixture.requests.length,
      'every response released after v2 validation/cancellation',
    );
  } finally {
    await client.close();
  }
  check(fixture.closes == 1, 'transport closed once');
  print('DART_V2_NATIVE_PORTABLE_OK');
}

Future<void> main() => exercisePortable();
