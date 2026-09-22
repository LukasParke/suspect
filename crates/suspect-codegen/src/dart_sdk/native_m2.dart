import 'dart:async' hide TimeoutException;
import 'dart:io' show File, Platform;
import 'package:generated_sdk/generated_sdk_io.dart';
import 'portable.dart' as portable;
import 'support.dart';

Future<void> marker(String name) async {
  final root = Platform.environment['SUSPECT_DART_HTTP_MARKERS']!;
  final file = File('$root/$name');
  print('DART_IO_WAIT $name');
  for (var i = 0; i < 300 && !file.existsSync(); i++) {
    await Future<void>.delayed(const Duration(milliseconds: 5));
  }
  check(file.existsSync(), 'independent socket fixture did not observe $name');
  print('DART_IO_OBSERVED $name');
}

Future<void> main() async {
  try {
    await run();
  } on TransportException catch (error) {
    print('Independent fixture transport cause: ${error.cause}');
    rethrow;
  }
}

Future<void> run() async {
  await portable.main();
  final base = Uri.parse(Platform.environment['SUSPECT_DART_HTTP_BASE']!);
  final client = Client(
    transport: IoTransport(),
    credentials: portable.credentials,
    server: base,
  );
  final created = await client.createWidget(body: WidgetInput(name: 'alpha'));
  check(
    created.data.amount.token == '9007199254740993.000000000000000001',
    'VM real POST response exactness',
  );
  final list = await client.listWidgets(
    tag: const Present('a'),
    tags: const Present(['x', 'y']),
    labels: const Present(['a,b', 'c']),
    limit: Present(JsonInteger.fromInt(2)),
  );
  check(
    list.data.items.single.payload is SecurePayload,
    'VM real GET list payload',
  );
  await client.getWidget(widgetId: "a/b 雪!'()*");
  await client.updateWidget(
    widgetId: 'w1',
    body: WidgetPatch(
      amount: Present(JsonNumber.parse('0.0000000000000000001')),
    ),
  );
  final redirect = await failsAsync<UnexpectedResponseException>(
    () => client.getWidget(widgetId: 'redirect'),
  );
  check(
    redirect.response.status == 302,
    'redirect remains an unexpected status',
  );
  final denied = await failsAsync<GetWidgetStatus404>(
    () => client.getWidget(widgetId: 'declared-large'),
  );
  check(
    denied.data.message.length == 256 &&
        denied.response.body.length == 64 &&
        denied.response.truncated,
    'VM typed API error and bounded capture',
  );
  final unknown = await failsAsync<UnexpectedResponseException>(
    () => client.getWidget(widgetId: 'unknown'),
  );
  check(
    unknown.response.body.length == 64 && unknown.response.truncated,
    'VM unknown response capture',
  );
  final small = Client(
    transport: IoTransport(),
    credentials: portable.credentials,
    server: base,
    maxResponseBytes: 32,
    maxCaptureBytes: 8,
  );
  await failsAsync<ResourceLimitException>(
    () => small.getWidget(widgetId: 'chunked-large'),
  );
  await small.close();
  final token = CancellationToken();
  final pending = failsAsync<CancelledException>(
    () => client.getWidget(widgetId: 'body-cancel', cancellation: token),
  );
  await marker('body-cancel-started');
  print('DART_IO_CANCEL');
  token.cancel();
  await pending;
  await marker('body-cancel-closed');
  check(
    (await client.getWidget(widgetId: 'after-cancel')).data.id == 'w1',
    'one cancellation keeps other calls usable',
  );
  await failsAsync<TimeoutException>(
    () => client.getWidget(
      widgetId: 'body-timeout',
      timeout: const Duration(milliseconds: 60),
    ),
  );
  await marker('body-timeout-closed');
  final opening = CancellationToken();
  final noHeaders = failsAsync<CancelledException>(
    () => client.getWidget(widgetId: 'header-cancel', cancellation: opening),
  );
  await marker('header-cancel-started');
  opening.cancel();
  await noHeaders;
  await marker('header-cancel-closed');
  final afterClose = failsAsync<ClientClosedException>(
    () => client.getWidget(widgetId: 'body-close'),
  );
  await marker('body-close-started');
  await client.close();
  await client.close();
  await afterClose;
  await marker('body-close-closed');
  print('DART_NATIVE_M2_IO_OK');
}
