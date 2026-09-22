import 'dart:io' show Platform;
import 'package:generated_sdk/generated_sdk_io.dart';
import 'support.dart';

Future<void> main() async {
  check(
    __UPDATE_CODEC__.encode(__UPDATE_BODY__()) == '{}',
    'native update omission',
  );
  check(
    __UPDATE_CODEC__.encode(__UPDATE_BODY__(limit: const Present(null))) ==
        '{"limit":null}',
    'native update explicit null',
  );
  final client = Client(
    transport: IoTransport(),
    credentials: const Credentials(apiKey: 'test-management-token'),
    server: Uri.parse(Platform.environment['SUSPECT_DART_HTTP_BASE']!),
  );
  try {
    final credits = await client.getCredits();
    check(
      credits.data.data.totalCredits.token == '100.50000000000000001',
      'actual credit response preserves precision',
    );
    check(credits.data.data.totalUsage.token == '25.75', 'actual usage type');
    final denied = await failsAsync<GetCreditsStatus401>(
      () => client.getCredits(),
    );
    check(
      denied.data.error.message == 'Missing Authentication header' &&
          denied.data.error.code.token == '401',
      'actual typed 401 payload',
    );
    final created = await client.createKeys(
      body: __CREATE_BODY__(
        name: 'Native Test Key',
        limit: Present(JsonNumber.parse('50.25')),
        limitReset: const Present(null),
      ),
    );
    check(
      created.status == 201 && created.data.key == 'fixture-secret',
      'actual create operation result',
    );
    check(
      created.data.data.limit?.token == '50.250' &&
          created.data.data.updatedAt == null,
      'actual required-nullable exact response',
    );
    final updated = await client.updateKeys(
      hash: 'fixture-hash',
      body: __UPDATE_BODY__(
        name: const Present('Updated Native Key'),
        limit: Present(JsonNumber.parse('75.50')),
        limitReset: const Present(null),
        disabled: const Present(true),
      ),
    );
    check(
      updated.data.data.limit?.token == '75.50' && updated.data.data.disabled,
      'actual PATCH response exactness and Boolean',
    );
    final list = await client.listContainerFiles(
      containerId: 'sess_abc123',
      limit: Present(JsonInteger.fromInt(2)),
      after: const Present('a/b 雪'),
    );
    check(
      !list.data.hasMore &&
          list.data.data.single.bytes.toBigInt() == BigInt.from(123),
      'actual list uses native bool and exact integer',
    );
    final file = await client.getContainerFile(
      containerId: 'sess_abc123',
      fileId: "cfile_a/b 雪!'()*",
    );
    check(
      file.data.bytes.token == '123' && file.data.path == 'out/report.csv',
      'actual file operation',
    );
    await client.listContainerFiles(containerId: 'sess_abc123');
    check(
      created.response.body.length == 64 && created.response.truncated,
      'actual success raw capture bounded',
    );
  } finally {
    await client.close();
  }
  print('DART_FIVE_ACTUAL_OPENROUTER_OPERATIONS_OK');
}
