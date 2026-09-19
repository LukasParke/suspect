import 'dart:async' hide TimeoutException;
import 'dart:convert' show utf8;
import 'package:generated_sdk/generated_sdk.dart';
import 'support.dart';

const widgetWire =
    r'{"id":"w1","amount":9007199254740993.000000000000000001,"meta":null,"child":{"label":"root"},"payload":{"kind":"standard","text":"plain"}}';
const listWire =
    r'{"items":[{"id":"w2","amount":0.0000000000000000001,"payload":{"kind":"secure","vault":"vlt-1"}}]}';
const updatedWire =
    r'{"id":"w1","amount":0.0000000000000000001,"meta":"present","payload":{"kind":"standard","text":"plain"}}';
const credentials = Credentials(apiKey: 'm2-key');

void exactValues() {
  for (final token in [
    '9007199254740993.000000000000000001',
    '18446744073709551615',
    '-0.00e+9999999999999999999999999999',
    '1e-9999999999999999999999999999',
    '1e+000009',
    '1e-000009',
    '1e000000000000000000000000000000000000000001',
    '1e+000000000000000000000000000000000000000008',
    '1e-000000000000000000000000000000000000000008',
  ]) {
    check(writeJson(parseJson(token)) == token, 'numeric token $token');
  }
  check(
    JsonInteger.parse('1000.000e-2').toBigInt() == BigInt.from(10),
    'mathematical integrality',
  );
  check(
    JsonNumber.parse('1e000008').compareTo(JsonNumber.parse('100000000')) == 0,
    'decimal exponent with leading zeroes',
  );
  check(
    JsonNumber.parse('0.000001000').compareTo(JsonNumber.parse('1e-6')) == 0,
    'symbolic comparison',
  );
  check(
    JsonNumber.parse('1e99999999999999999999999999999').isInteger,
    'symbolic large integral exponent',
  );
  check(
    !JsonNumber.parse('1e-99999999999999999999999999999').isInteger,
    'symbolic large fractional exponent',
  );
  check(
    JsonNumber.fromBigInt(BigInt.parse('184467440737095516150000')).token ==
        '184467440737095516150000',
    'BigInt input',
  );
  fails<JsonException>(() => JsonInteger.parse('0.1'));
  check(
    fails<JsonException>(
      () => JsonInteger.parse('1e999999999999999').toBigInt(),
    ).resourceLimit,
    'bounded integer expansion',
  );
  for (final text in [
    '',
    '+1',
    '01',
    '-01',
    '.1',
    '1.',
    '1e',
    '1e+',
    'NaN',
    'Infinity',
    '1,2',
    ' 1',
    '0x10',
    '1e0x10',
  ]) {
    check(
      !fails<JsonException>(() => JsonNumber.parse(text)).resourceLimit,
      'invalid numeric syntax $text',
    );
  }
  for (final text in [
    '[1,]',
    '{"a":1,}',
    r'{"a":1,"\u0061":2}',
    r'"\uD800"',
    r'"\uDC00"',
    r'"\uD800\u0041"',
    r'{"\uD800":1}',
    '"\n"',
    'null false',
  ]) {
    check(
      !fails<JsonException>(() => parseJson(text)).resourceLimit,
      'JSON syntax/Unicode $text',
    );
  }
  check(
    writeJson(parseJson(r'"\uD83D\uDE00"')) == '"😀"',
    'paired escaped surrogates',
  );
  check(
    writeJson(parseJson(r'{"😀":1,"e\u0301":2,"é":3}')) ==
        '{"😀":1,"é":2,"é":3}',
    'Unicode keys stay distinct',
  );
  for (final value in [
    String.fromCharCode(0xd800),
    String.fromCharCode(0xdc00),
    String.fromCharCodes([0xd800, 65]),
  ]) {
    fails<JsonException>(() => JsonString(value));
    fails<JsonException>(() => JsonObject({value: const JsonNull()}));
    fails<JsonException>(() => parseJson('"$value"'));
  }
  for (final bytes in [
    [34, 0xc0, 0xaf, 34],
    [34, 0xed, 0xa0, 0x80, 34],
    [34, 0xf4, 0x90, 0x80, 0x80, 34],
    [34, 256, 34],
    [-1],
  ]) {
    fails<JsonException>(() => parseJsonBytes(bytes));
  }
  check(
    fails<JsonException>(
      () => parseJson('"雪"', limits: const JsonLimits(maxBytes: 4)),
    ).resourceLimit,
    'UTF-8 input bytes',
  );
  check(
    fails<JsonException>(
      () => parseJson('[[[]]]', limits: const JsonLimits(maxDepth: 2)),
    ).resourceLimit,
    'parse depth',
  );
  check(
    fails<JsonException>(
      () => parseJson('[1,2]', limits: const JsonLimits(maxSteps: 2)),
    ).resourceLimit,
    'parse work',
  );
  check(
    fails<JsonException>(
      () => writeJson(JsonString('雪'), limits: const JsonLimits(maxBytes: 4)),
    ).resourceLimit,
    'UTF-8 output bytes',
  );
  check(
    fails<JsonException>(
      () =>
          writeJson(JsonString('abcd'), limits: const JsonLimits(maxSteps: 2)),
    ).resourceLimit,
    'writer work',
  );
  check(
    fails<JsonException>(
      () => JsonNumber.parse('12345', maxBytes: 4),
    ).resourceLimit,
    'numeric byte limit',
  );
  final source = <String, JsonValue>{'safe': JsonNumber.parse('1.00')};
  final snapshot = JsonObject(source);
  source['safe'] = const JsonBoolean(true);
  check(writeJson(snapshot) == '{"safe":1.00}', 'JSON object snapshot');
  fails<UnsupportedError>(() {
    snapshot.values.clear();
  });
}

void modelsAndCodecs() {
  final input = WidgetInput(
    name: 'alpha',
    amount: Present(JsonNumber.parse('1.2300e+2')),
  );
  check(
    widgetInputCodec.encode(input) == '{"amount":1.2300e+2,"name":"alpha"}',
    'native exact encode',
  );
  final native = widgetInputCodec.decode(widgetInputCodec.encode(input));
  check(switch (native.amount) {
    Present(value: final n) => n.token == '1.2300e+2',
    _ => false,
  }, 'native decimal decode');
  input.name = '';
  final invalid = fails<CodecException>(() => widgetInputCodec.encode(input));
  check(
    invalid.kind == CodecFailureKind.invalid && invalid.instancePath == '/name',
    'mutable string revalidation',
  );
  check(
    invalid.source.pointer ==
        '/components/schemas/WidgetInput/properties/name/minLength',
    'original keyword identity',
  );
  input.name = String.fromCharCode(0xd800);
  check(
    fails<CodecException>(() => widgetInputCodec.toJson(input)).kind ==
        CodecFailureKind.conversion,
    'native surrogate classification',
  );
  for (final wire in [
    '{}',
    '{"name":null}',
    '{"name":"x","amount":null}',
    '{"name":true}',
  ]) {
    check(
      fails<CodecException>(() => widgetInputCodec.decode(wire)).kind ==
          CodecFailureKind.invalid,
      'presence/type check',
    );
  }
  final extras = WidgetInput(
    name: 'x',
    extraFields: {
      'constructor': JsonNumber.parse('4.00'),
      '__proto__': const JsonBoolean(true),
    },
  );
  check(
    widgetInputCodec.encode(extras).contains('"__proto__":true'),
    'arbitrary extra wire keys',
  );
  extras.extraFields['name'] = JsonString('shadow');
  check(
    fails<CodecException>(() => widgetInputCodec.encode(extras)).kind ==
        CodecFailureKind.conversion,
    'extra collision',
  );
  final widget = widgetCodec.decode(widgetWire);
  check(
    widget.amount.token == '9007199254740993.000000000000000001',
    'exact model value',
  );
  check(switch (widget.meta) {
    Present<String?>(value: null) => true,
    _ => false,
  }, 'explicit null');
  check(switch (widget.child) {
    Present(value: final node) => node.label == 'root' && node.child is Absent,
    _ => false,
  }, 'recursive optional');
  final description = switch (widget.payload) {
    StandardPayload(text: final text) => text,
    SecurePayload(vault: final vault) => vault,
  };
  check(description == 'plain', 'sealed exhaustive source union');
  check(
    widgetCodec.decode(widgetCodec.encode(widget)).payload is StandardPayload,
    'direct union round trip',
  );
  final absent = widgetCodec.decode(
    r'{"id":"w","amount":1.0,"payload":{"kind":"secure","vault":"v"}}',
  );
  check(
    absent.meta is Absent<String?> &&
        !widgetCodec.encode(absent).contains('"meta"'),
    'absent distinct from null',
  );
  final branch = SecurePayload(vault: 'v');
  check(branch.kind == 'secure', 'required const literal getter');
  check(
    widgetPayloadCodec.encode(branch).contains('"kind":"secure"'),
    'native branch encodes without false cycle',
  );
  final cycle = WidgetNode(label: 'loop');
  cycle.child = Present(cycle);
  final cycleError = fails<CodecException>(() => widgetNodeCodec.encode(cycle));
  check(
    cycleError.kind == CodecFailureKind.conversion &&
        cycleError.instancePath == '/child',
    'real model cycle rejected',
  );
  var deep = WidgetNode(label: 'leaf');
  for (var i = 0; i < 140; i++) {
    deep = WidgetNode(label: 'branch', child: Present(deep));
  }
  check(
    fails<CodecException>(() => widgetNodeCodec.encode(deep)).kind ==
        CodecFailureKind.resourceLimit,
    'native conversion depth',
  );
  final list = WidgetList(items: [absent]);
  check(
    widgetListCodec.validate(widgetListCodec.toJson(list)).isValid,
    'valid mutable collection',
  );
  absent.extraFields['amount'] = const JsonBoolean(true);
  check(
    fails<CodecException>(() => widgetListCodec.encode(list)).kind ==
        CodecFailureKind.conversion,
    'nested mutation revalidation',
  );
}

Future<void> selectedWire() async {
  final recording = Recording([
    Fixture(
      method: 'POST',
      url: 'https://m2.example.test/api/v1/widgets',
      requestBody: '{"name":"alpha"}',
      text: widgetWire,
    ),
    Fixture(
      method: 'GET',
      url:
          'https://m2.example.test/api/v1/widgets?tag=a&tags=x&tags=y&labels=a%2Cb,c&limit=2',
      text: listWire,
    ),
    Fixture(
      method: 'GET',
      url:
          'https://m2.example.test/api/v1/widgets/a%2Fb%20%E9%9B%AA%21%27%28%29%2A',
      text: widgetWire,
    ),
    Fixture(
      method: 'PATCH',
      url: 'https://m2.example.test/api/v1/widgets/w1',
      requestBody: '{}',
      text: updatedWire,
    ),
    Fixture(
      method: 'GET',
      url: 'https://m2.example.test/api/v1/widgets',
      text: listWire,
    ),
  ]);
  final client = Client(transport: recording, credentials: credentials);
  final created = await client.createWidget(body: WidgetInput(name: 'alpha'));
  check(
    created.data.amount.token == '9007199254740993.000000000000000001' &&
        created.status == 200,
    'typed success',
  );
  check(
    created.response.body.length == 64 && created.response.truncated,
    'successful capture ceiling',
  );
  fails<UnsupportedError>(() {
    created.response.body[0] = 0;
  });
  fails<UnsupportedError>(() {
    created.response.headers['content-type']!.clear();
  });
  final list = await client.listWidgets(
    tag: const Present('a'),
    tags: const Present(['x', 'y']),
    labels: const Present(['a,b', 'c']),
    limit: Present(JsonInteger.fromInt(2)),
  );
  check(
    list.data.items.single.payload is SecurePayload,
    'typed collection and union',
  );
  await client.getWidget(widgetId: "a/b 雪!'()*");
  final updated = await client.updateWidget(
    widgetId: 'w1',
    body: WidgetPatch(),
  );
  check(switch (updated.data.meta) {
    Present(value: 'present') => true,
    _ => false,
  }, 'typed update');
  await client.listWidgets(tags: const Present([]), labels: const Present([]));
  check(recording.responsesClosed == 5, 'release every consumed response');
  await client.close();
  await client.close();
  check(recording.closes == 1, 'idempotent transport ownership');
  await failsAsync<ClientClosedException>(
    () => client.getWidget(widgetId: 'after-close'),
  );
  check(recording.requests.length == 5, 'closed client does not send');
}

Future<void> requestBoundaries() async {
  final recording = Recording([]);
  final client = Client(
    transport: recording,
    credentials: credentials,
    maxRequestBytes: 256,
  );
  await failsAsync<CodecException>(
    () => client.createWidget(body: WidgetInput(name: '')),
  );
  await failsAsync<CodecException>(
    () => client.listWidgets(limit: Present(JsonInteger.fromInt(1001))),
  );
  await failsAsync<ConfigurationException>(
    () => client.getWidget(widgetId: '..'),
  );
  await failsAsync<ConfigurationException>(
    () => client.getWidget(widgetId: '.'),
  );
  await failsAsync<ResourceLimitException>(
    () => client.getWidget(widgetId: '雪' * 100),
  );
  await failsAsync<ResourceLimitException>(
    () => client.listWidgets(tags: Present(List.filled(500, 'x'))),
  );
  await failsAsync<ResourceLimitException>(
    () => client.createWidget(body: WidgetInput(name: 'a' * 1000)),
  );
  final cancelled = CancellationToken()..cancel();
  await failsAsync<CancelledException>(
    () => client.getWidget(widgetId: 'never-send', cancellation: cancelled),
  );
  await failsAsync<ConfigurationException>(
    () => client.getWidget(widgetId: 'never-send', timeout: Duration.zero),
  );
  check(
    recording.requests.isEmpty,
    'invalid requests must not enter transport',
  );
  for (final token in [
    '',
    '=padding',
    'x y',
    'x\r\nHeader: leak',
    'x\u0000',
    '雪',
    'x=bad',
  ]) {
    final bad = Client(
      transport: recording,
      credentials: Credentials(apiKey: token),
    );
    final error = await failsAsync<ConfigurationException>(
      () => bad.getWidget(widgetId: 'x'),
    );
    check(
      !error.toString().contains('leak'),
      'credential validation is redacted',
    );
  }
  for (final uri in [
    'http://example.test',
    'ftp://example.test',
    'https://u:p@example.test',
    'https://example.test/?q=1',
    'https://example.test/#x',
    'https://example.test/api/%2fescape',
  ]) {
    fails<ConfigurationException>(
      () => Client(
        transport: recording,
        credentials: credentials,
        server: Uri.parse(uri),
      ),
    );
  }
  fails<ConfigurationException>(
    () => Client(
      transport: recording,
      credentials: credentials,
      maxResponseBytes: 0,
    ),
  );
  fails<ConfigurationException>(
    () => Client(
      transport: recording,
      credentials: credentials,
      maxCaptureBytes: 65,
    ),
  );
  fails<ConfigurationException>(
    () => Client(
      transport: recording,
      credentials: credentials,
      maxResponseBytes: 32,
      maxCaptureBytes: 64,
    ),
  );
  check(
    !credentials.toString().contains('m2-key'),
    'credentials debug is redacted',
  );
  await client.close();
  final override = Recording([
    Fixture(
      method: 'GET',
      url: 'https://override.test/base//widgets/x',
      text: widgetWire,
    ),
  ]);
  final overridden = Client(
    transport: override,
    credentials: credentials,
    server: Uri.parse('https://override.test/base//'),
  );
  await overridden.getWidget(widgetId: 'x');
  await overridden.close();
}

Future<void> responses() async {
  for (final media in [
    'Application/JSON; charset=utf-8',
    'application/json ; Charset="UTF-8"',
    'application/json; profile="a;b"',
    ' application/json\t; profile="escaped\\\"quote" ',
  ]) {
    final recording = Recording([
      Fixture(
        text: widgetWire,
        headers: {
          'CONTENT-TYPE': [media],
        },
      ),
    ]);
    final client = Client(transport: recording, credentials: credentials);
    check(
      (await client.getWidget(widgetId: 'x')).data.id == 'w1',
      'valid media grammar',
    );
    await client.close();
  }
  for (final headers in <Map<String, List<String>>>[
    {},
    {
      'content-type': ['application/jsonp'],
    },
    {
      'content-type': ['application/problem+json'],
    },
    {
      'content-type': ['text/plain'],
    },
    {
      'content-type': ['application/json', 'application/json'],
    },
    {
      'content-type': ['application/json'],
      'Content-Type': ['application/json'],
    },
    {
      'content-type': ['application/json;charset=utf-16'],
    },
    {
      'content-type': ['application/json;'],
    },
    {
      'content-type': ['application/json;charset="utf-8'],
    },
    {
      'content-type': ['application/json; p='],
    },
    {
      'content-type': ['application/json;charset=utf-8; CHARSET=utf-8'],
    },
    {
      'content-type': ['application/json,application/json'],
    },
    {
      'content-type': ['application/json\u00a0'],
    },
    {
      'content-type': ['application/json'],
      'content-encoding': ['gzip'],
    },
  ]) {
    final recording = Recording([Fixture(text: widgetWire, headers: headers)]);
    final client = Client(transport: recording, credentials: credentials);
    final error = await failsAsync<MediaTypeException>(
      () => client.getWidget(widgetId: 'x'),
    );
    check(
      error.response.body.length == 64 && error.response.truncated,
      'media capture bounded',
    );
    check(recording.responsesClosed == 1, 'media failure closes response');
    await client.close();
  }
  final fixtures = Recording([
    Fixture(status: 404, text: '{"message":"${'x' * 256}"}'),
    Fixture(
      status: 500,
      text: 'sensitive-' * 64,
      headers: const {
        'content-type': ['bad'],
      },
    ),
    Fixture(text: '{broken'),
    Fixture(text: '{"id":1}'),
    Fixture(
      chunks: const [
        [34, 0xed, 0xa0, 0x80, 34],
      ],
    ),
    Fixture(
      chunks: const [
        [256],
      ],
    ),
  ]);
  final client = Client(transport: fixtures, credentials: credentials);
  final api = await failsAsync<GetWidgetStatus404>(
    () => client.getWidget(widgetId: 'missing'),
  );
  check(
    api.data.message.length == 256 && api.status == 404,
    'declared error payload stays typed',
  );
  check(
    api.response.body.length == 64 &&
        api.response.truncated &&
        !api.toString().contains('xxxx'),
    'API error capture and redaction',
  );
  final unknown = await failsAsync<UnexpectedResponseException>(
    () => client.getWidget(widgetId: 'unknown'),
  );
  check(
    unknown.response.body.length == 64 &&
        !unknown.toString().contains('sensitive'),
    'unknown status precedes media and redacts',
  );
  final malformed = await failsAsync<InvalidResponseException>(
    () => client.getWidget(widgetId: 'malformed'),
  );
  check(
    malformed.codecFailure?.kind == CodecFailureKind.json,
    'malformed JSON classification',
  );
  final mismatch = await failsAsync<InvalidResponseException>(
    () => client.getWidget(widgetId: 'mismatch'),
  );
  check(
    mismatch.codecFailure?.kind == CodecFailureKind.invalid,
    'schema mismatch classification',
  );
  final unicode = await failsAsync<InvalidResponseException>(
    () => client.getWidget(widgetId: 'unicode'),
  );
  check(
    unicode.codecFailure?.kind == CodecFailureKind.json,
    'malformed UTF-8 classification',
  );
  await failsAsync<InvalidResponseException>(
    () => client.getWidget(widgetId: 'not-a-byte'),
  );
  check(
    fixtures.responsesClosed == 6,
    'all typed/untyped failure responses released',
  );
  await client.close();
  final thrown = Client(
    transport: ThrowingTransport(),
    credentials: credentials,
  );
  final transport = await failsAsync<TransportException>(
    () => thrown.getWidget(widgetId: 'secret-url'),
  );
  check(
    !transport.toString().contains('m2-key') &&
        !transport.toString().contains('secret-url'),
    'underlying exception is not interpolated',
  );
  await thrown.close();
}

Future<void> responseBudgets() async {
  for (final fixture in [
    Fixture(text: 'x' * 4096),
    Fixture(chunks: List.generate(80, (_) => utf8.encode('x'))),
    Fixture(chunks: List.generate(1100, (_) => <int>[])),
    Fixture(
      headers: const {
        'content-length': ['00000000000999999999999'],
      },
    ),
  ]) {
    final recording = Recording([fixture]);
    final client = Client(
      transport: recording,
      credentials: credentials,
      maxResponseBytes: 32,
      maxCaptureBytes: 8,
    );
    final error = await failsAsync<ResourceLimitException>(
      () => client.getWidget(widgetId: 'large'),
    );
    check(
      error.response != null &&
          error.response!.body.length <= 8 &&
          error.response!.truncated,
      'bounded oversized raw response',
    );
    check(recording.responsesClosed == 1, 'oversized stream released');
    await client.close();
  }
  for (final headers in <Map<String, List<String>>>[
    {
      'x-long': ['a' * 300],
    },
    {for (var i = 0; i < 100; i++) 'x-$i': <String>[]},
  ]) {
    final recording = Recording([Fixture(headers: headers)]);
    final client = Client(
      transport: recording,
      credentials: credentials,
      maxResponseHeaderBytes: 64,
    );
    await failsAsync<ResourceLimitException>(
      () => client.getWidget(widgetId: 'headers'),
    );
    check(
      recording.responsesClosed == 1,
      'header limit releases unread response',
    );
    await client.close();
  }
  for (final headers in <Map<String, List<String>>>[
    {
      'bad\nname': ['x'],
    },
    {
      'x': ['injected\r\nheader: x'],
    },
    {
      'content-length': ['1', '1'],
    },
    {
      'content-length': ['+1'],
    },
    {
      'content-length': ['1.0'],
    },
    {
      'content-length': ['1\u00a0'],
    },
    {
      'content-length': ['2'],
      'transfer-encoding': ['chunked'],
    },
    {
      'transfer-encoding': ['gzip, chunked'],
    },
    {
      'content-length': ['123'],
      'content-type': ['application/json'],
    },
  ]) {
    final recording = Recording([Fixture(headers: headers)]);
    final client = Client(transport: recording, credentials: credentials);
    await failsAsync<InvalidResponseException>(
      () => client.getWidget(widgetId: 'headers'),
    );
    check(
      recording.responsesClosed == 1,
      'invalid headers/length release response',
    );
    await client.close();
  }
}

Future<void> cancellationAndLifetime() async {
  var responseCloses = 0;
  late TransportResponse response;
  response = TransportResponse(
    status: 200,
    headers: const {},
    body: const Stream.empty(),
    onClose: () async {
      responseCloses++;
      unawaited(response.close());
    },
  );
  await response.close();
  check(
    responseCloses == 1,
    'response close memoizes before invoking callbacks',
  );
  final reentrant = ReentrantTransport();
  reentrant.client = Client(transport: reentrant, credentials: credentials);
  final stopped = failsAsync<ClientClosedException>(
    () => reentrant.client.getWidget(widgetId: 'reentrant'),
  );
  await reentrant.started.future;
  await reentrant.client.close();
  await stopped;
  check(
    reentrant.closes == 1,
    'client close memoizes before cancellation observers run',
  );
  final caller = CancellationToken();
  final recording = Recording([Fixture(text: widgetWire)]);
  final completed = Client(transport: recording, credentials: credentials);
  await completed.getWidget(widgetId: 'x', cancellation: caller);
  caller.cancel();
  check(
    !recording.requests.single.cancellation.isCancelled,
    'completed call detached from reusable caller token',
  );
  await completed.close();
  for (final mode in ['cancel', 'timeout', 'close']) {
    final stalled = StalledBody();
    final client = Client(
      transport: stalled,
      credentials: credentials,
      timeout: mode == 'timeout'
          ? const Duration(milliseconds: 30)
          : const Duration(seconds: 2),
    );
    final token = CancellationToken();
    final operation = client.getWidget(
      widgetId: 'stalled',
      cancellation: token,
    );
    final outcome = failsAsync<SdkException>(() => operation);
    await stalled.started.future;
    await Future<void>.delayed(Duration.zero);
    if (mode == 'cancel') {
      token.cancel();
    }
    if (mode == 'close') {
      await client.close();
    }
    final error = await outcome;
    check(switch ((mode, error)) {
      ('cancel', CancelledException()) => true,
      ('timeout', TimeoutException()) => true,
      ('close', ClientClosedException()) => true,
      _ => false,
    }, 'stop reasons remain distinct');
    check(
      stalled.cancelled == 1 && stalled.released == 1,
      'body subscription and response disposed once',
    );
    check(
      stalled.request!.cancellation.isCancelled,
      'adapter receives cancellation',
    );
    await client.close();
    check(stalled.closed == 1, 'owned transport closed once');
    await stalled.controller.close();
  }
  for (final lateError in [false, true]) {
    final late = LateTransport();
    final client = Client(transport: late, credentials: credentials);
    final token = CancellationToken();
    final operation = failsAsync<CancelledException>(
      () => client.getWidget(widgetId: 'pending-connect', cancellation: token),
    );
    await late.started.future;
    token.cancel();
    await operation;
    var closed = 0;
    if (lateError) {
      late.pending.completeError(StateError('late transport failure'));
    } else {
      late.pending.complete(
        TransportResponse(
          status: 200,
          headers: const {},
          body: const Stream.empty(),
          onClose: () async {
            closed++;
          },
        ),
      );
    }
    await Future<void>.delayed(const Duration(milliseconds: 1));
    check(
      lateError || closed == 1,
      'late response released after cancellation',
    );
    await client.close();
  }
}

Future<void> main() async {
  exactValues();
  modelsAndCodecs();
  await selectedWire();
  await requestBoundaries();
  await responses();
  await responseBudgets();
  await cancellationAndLifetime();
  print('DART_PORTABLE_M2_OK');
}
