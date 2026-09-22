import 'dart:async' hide TimeoutException;
import 'dart:convert' show utf8;
import 'package:generated_sdk/generated_sdk.dart';

void check(bool value, String message) {
  if (!value) {
    throw StateError(message);
  }
}

T fails<T extends Object>(void Function() action) {
  try {
    action();
  } on Object catch (error) {
    if (error is T) {
      return error;
    }
    rethrow;
  }
  throw StateError('expected $T');
}

Future<T> failsAsync<T extends Object>(
  Future<Object?> Function() action,
) async {
  try {
    await action();
  } on Object catch (error) {
    if (error is T) {
      return error;
    }
    if (error is TransportException) {
      print('Unexpected fixture transport cause: ${error.cause}');
    }
    rethrow;
  }
  throw StateError('expected $T');
}

final class Fixture {
  Fixture({
    this.method,
    this.url,
    this.requestBody,
    this.status = 200,
    this.text = '{}',
    this.headers = const {
      'content-type': ['application/json'],
    },
    this.chunks,
  });
  final String? method;
  final String? url;
  final String? requestBody;
  final int status;
  final String text;
  final Map<String, List<String>> headers;
  final Iterable<List<int>>? chunks;
}

final class Recording implements HttpTransport {
  Recording(this.fixtures, {this.token = 'm2-key'});
  final List<Fixture> fixtures;
  final String token;
  final List<TransportRequest> requests = [];
  int responsesClosed = 0;
  int streamsCancelled = 0;
  int closes = 0;
  @override
  Future<TransportResponse> send(TransportRequest request) async {
    requests.add(request);
    check(fixtures.isNotEmpty, 'unplanned HTTP exchange');
    final fixture = fixtures.removeAt(0);
    if (fixture.method != null) {
      check(request.method == fixture.method, 'method');
    }
    if (fixture.url != null) {
      check(request.url.toString() == fixture.url, 'wire URL: ${request.url}');
    }
    if (fixture.method != null) {
      check(
        request.body == null
            ? fixture.requestBody == null
            : utf8.decode(request.body!) == fixture.requestBody,
        'wire body',
      );
      check(
        request.headers['content-type'] ==
            (fixture.requestBody == null ? null : 'application/json'),
        'request media',
      );
    }
    check(
      request.headers['authorization'] == 'Bearer $token',
      'declared bearer',
    );
    check(request.headers['accept'] == 'application/json', 'accept');
    check(
      request.headers['accept-encoding'] == 'identity',
      'bounded content encoding',
    );
    fails<UnsupportedError>(() {
      request.headers['authorization'] = 'overwritten';
    });
    if (request.body != null && request.body!.isNotEmpty) {
      fails<UnsupportedError>(() {
        request.body![0] = 0;
      });
    }
    return TransportResponse(
      status: fixture.status,
      headers: fixture.headers,
      body: Stream<List<int>>.fromIterable(
        fixture.chunks ?? [utf8.encode(fixture.text)],
      ),
      onClose: () async {
        responsesClosed++;
      },
    );
  }

  @override
  Future<void> close() async {
    closes++;
  }
}

final class StalledBody implements HttpTransport {
  final Completer<void> started = Completer<void>();
  final StreamController<List<int>> controller = StreamController<List<int>>();
  int cancelled = 0;
  int released = 0;
  int closed = 0;
  TransportRequest? request;
  StalledBody() {
    controller.onListen = () {
      controller.add(utf8.encode('{"i'));
      started.complete();
    };
    controller.onCancel = () {
      cancelled++;
    };
  }
  @override
  Future<TransportResponse> send(TransportRequest request) async {
    this.request = request;
    return TransportResponse(
      status: 200,
      headers: const {
        'content-type': ['application/json'],
      },
      body: controller.stream,
      onClose: () async {
        released++;
      },
    );
  }

  @override
  Future<void> close() async {
    closed++;
  }
}

/// Deliberately non-cooperative adapter: late values/errors still need handlers.
final class LateTransport implements HttpTransport {
  final Completer<void> started = Completer<void>();
  final Completer<TransportResponse> pending = Completer<TransportResponse>();
  TransportRequest? request;
  int closed = 0;
  @override
  Future<TransportResponse> send(TransportRequest request) {
    this.request = request;
    started.complete();
    return pending.future;
  }

  @override
  Future<void> close() async {
    closed++;
  }
}

final class ThrowingTransport implements HttpTransport {
  @override
  Future<TransportResponse> send(TransportRequest request) async {
    throw StateError('sensitive upstream request: ${request.headers}');
  }

  @override
  Future<void> close() async {}
}

final class ReentrantTransport implements HttpTransport {
  late Client client;
  final Completer<void> started = Completer<void>();
  int closes = 0;
  @override
  Future<TransportResponse> send(TransportRequest request) {
    final pending = Completer<TransportResponse>();
    request.cancellation.onCancel(() {
      unawaited(client.close());
      pending.completeError(const CancelledException());
    });
    started.complete();
    return pending.future;
  }

  @override
  Future<void> close() async {
    closes++;
  }
}
