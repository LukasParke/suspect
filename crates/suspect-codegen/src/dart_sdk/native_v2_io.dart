import 'dart:io' show Platform;
import 'package:generated_sdk/generated_sdk_io.dart';
import 'portable.dart' as portable;

Future<void> main() async {
  await portable.exercisePortable();
  final client = Client(
    transport: IoTransport(),
    server: Uri.parse(Platform.environment['DART_V2_BASE']!),
  );
  try {
    final result = await client.scopedEcho(body: portable.nativeEnvelope());
    portable.check(
      envelopeCodec.encode(result.data) == portable.wireEnvelope,
      'socket native response',
    );
    await client.optionalPatch();
    await client.optionalPatch(body: const Present<Patch?>(null));
    final extras = await client.mixedExtras(body: portable.nativeMixed());
    portable.check(
      mixedExtrasCodec.encode(extras.data) == portable.wireMixed,
      'socket patterned extras',
    );
    try {
      await client.scopedEcho(body: portable.nativeEnvelope());
      throw StateError('invalid remote object admitted');
    } on InvalidResponseException catch (error) {
      portable.check(
        error.codecFailure?.kind == CodecFailureKind.invalid,
        'remote v2 rejection',
      );
    }
    var count = 0;
    try {
      await for (final row in client.scopedRows()) {
        count++;
        portable.check(row.data.stamp == null, 'socket stream item');
      }
      throw StateError('invalid item admitted');
    } on InvalidResponseException catch (error) {
      portable.check(
        error.codecFailure?.kind == CodecFailureKind.invalid,
        'remote item rejection',
      );
    }
    portable.check(count == 1, 'remote valid item before failure');
  } finally {
    await client.close();
  }
  print('DART_V2_NATIVE_IO_OK');
}
