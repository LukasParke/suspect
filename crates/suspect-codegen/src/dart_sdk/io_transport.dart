/// Standard-library VM adapter with per-exchange connection ownership.
///
/// Each exchange owns its HttpClient and cancellable connection task. This
/// avoids shared cookie/auth state and makes cancelling one call independent of
/// other calls. Redirects, proxies, decompression and automatic auth are disabled.
/// HTTPS uses the platform's normal certificate verification. No insecure TLS
/// override is installed. Applications can supply another [HttpTransport].
final class IoTransport implements HttpTransport {
  bool _closed = false;
  final Set<_IoExchange> _active = {};
  final Set<_ExactIoExchange> _exact = {};

  @override
  Future<TransportResponse> send(TransportRequest request) async {
    if (_closed) {
      throw const ClientClosedException();
    }
    request.cancellation.throwIfCancelled();
    if (request.method != request.method.toUpperCase()) {
      return _exactRequest(request, _exact, () => _closed);
    }
    final exchange = _IoExchange(request.timeout);
    _active.add(exchange);
    void release() {
      exchange.close();
      _active.remove(exchange);
    }

    exchange.detach = request.cancellation.onCancel(release);
    try {
      final outgoing = await exchange.client.openUrl(
        request.method,
        request.url,
      );
      exchange.request = outgoing;
      // Observe done even if setting headers or cancellation throws before close.
      unawaited(
        outgoing.done.then<void>((_) {}, onError: (Object _, StackTrace __) {}),
      );
      request.cancellation.throwIfCancelled();
      if (_closed || exchange.closed) {
        throw const ClientClosedException();
      }
      outgoing.followRedirects = false;
      outgoing.maxRedirects = 0;
      outgoing.persistentConnection = false;
      outgoing.headers.clear();
      for (final header in request.headers.entries) {
        outgoing.headers.set(header.key, header.value);
      }
      // Clearing HttpClient's defaults also removes its mandatory Host field.
      // The transport owns this authority, including any non-default port.
      outgoing.headers.set(HttpHeaders.hostHeader, request.url.authority);
      outgoing.contentLength = request.body?.length ?? 0;
      if (request.body != null) {
        outgoing.add(request.body!);
      }
      final response = await outgoing.close();
      request.cancellation.throwIfCancelled();
      final headers = <String, List<String>>{};
      response.headers.forEach((name, values) {
        headers[name] = values;
      });
      return TransportResponse(
        status: response.statusCode,
        headers: headers,
        body: response,
        onClose: () async {
          release();
        },
      );
    } on Object {
      release();
      rethrow;
    }
  }

  @override
  Future<void> close() async {
    if (_closed) {
      return;
    }
    _closed = true;
    for (final exchange in _active.toList()) {
      exchange.close();
    }
    _active.clear();
    for (final exchange in _exact.toList()) { exchange.close(); }
    _exact.clear();
  }
}

final class _IoExchange {
  _IoExchange(Duration timeout) {
    client.autoUncompress = false;
    client.findProxy = null;
    client.userAgent = null;
    client.connectionTimeout = timeout;
    client.connectionFactory = (url, proxyHost, proxyPort) async {
      // A custom factory bypasses HttpClient's own TLS connection selection.
      final ConnectionTask<Socket> pending = url.isScheme('https')
          ? await SecureSocket.startConnect(url.host, url.port)
          : await Socket.startConnect(url.host, url.port);
      connection = pending;
      if (closed) {
        pending.cancel();
      }
      return pending;
    };
  }
  final HttpClient client = HttpClient();
  HttpClientRequest? request;
  ConnectionTask<Socket>? connection;
  void Function()? detach;
  bool closed = false;
  void close() {
    if (closed) {
      return;
    }
    closed = true;
    detach?.call();
    connection?.cancel();
    request?.abort();
    client.close(force: true);
  }
}
