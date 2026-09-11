import 'dart:convert';
import 'package:generated_sdk/generated_sdk.dart';

void check(bool value, String label) {
  if (!value) throw StateError(label);
}

final class Replies implements HttpTransport {
  Replies(this.responses);
  final List<(int, String, String)> responses;
  int calls = 0;
  int releases = 0;
  @override
  Future<TransportResponse> send(TransportRequest request) async {
    check(request.method == 'GET' && request.url.path == '/mixed', 'wire identity');
    final (status, media, text) = responses[calls++];
    final bytes = utf8.encode(text);
    return TransportResponse(
      status: status,
      headers: {'content-type': [media]},
      body: Stream.fromIterable([for (final byte in bytes) [byte]]),
      onClose: () async { releases++; },
    );
  }
  @override
  Future<void> close() async {}
}

Future<void> main() async {
  final replies = Replies([
    (200, 'application/json', '{"message":"complete"}'),
    (200, 'text/event-stream', 'data: {"text":"hello"}\n\ndata: [DONE]\n\n'),
    (204, 'application/json', ''),
    (400, 'application/json', '{"message":"failed"}'),
    (200, 'application/json', '{}'),
  ]);
  final client = Client(transport: replies);
  final pending = client.mixed();
  check(replies.calls == 0, 'lazy execution');
  final complete = await pending.toList();
  check(complete.length == 1, 'one finite result');
  final response = complete.single;
  check(response is MixedStatus200, 'finite response status');
  if (response is MixedStatus200) {
    final data = response.data;
    check(data is __JSON_VARIANT__, 'finite media variant');
    if (data is __JSON_VARIANT__) check(data.value.message == 'complete', 'typed JSON value');
  }
  final events = await client.mixed().toList();
  check(events.length == 2, 'two parsed events');
  final values = <String>[];
  for (final event in events) {
    check(event is MixedStatus200, 'event response status');
    if (event is MixedStatus200) {
      final data = event.data;
      check(data is __SSE_VARIANT__, 'stream media variant');
      if (data is __SSE_VARIANT__) values.add(data.value.data);
    }
  }
  check(values.join('|') == '{"text":"hello"}|[DONE]', 'raw data includes sentinel');
  final empty = await client.mixed().toList();
  check(empty.length == 1 && empty.single is MixedStatus204, 'one no-content result');
  try {
    await client.mixed().toList();
    throw StateError('error response accepted');
  } on MixedStatus400 catch (error) {
    check(error.data.message == 'failed', 'typed bounded error');
  }
  try {
    await client.mixed().toList();
    throw StateError('invalid finite response accepted');
  } on InvalidResponseException { /* Complete finite data still runs its codec. */ }
  check(replies.calls == 5 && replies.releases == 5, 'response ownership');
  await client.close();

  final oversized = Replies([(200, 'application/json', '{"message":"too large"}')]);
  final bounded = Client(transport: oversized, maxResponseBytes: 8, maxCaptureBytes: 4);
  try {
    await bounded.mixed().toList();
    throw StateError('finite byte ceiling bypassed');
  } on ResourceLimitException { /* Mixed media retains finite-body limits. */ }
  check(oversized.releases == 1, 'bounded response released');
  await bounded.close();
  print('mixed JSON/SSE/no-content/error and ownership checks passed');
}
