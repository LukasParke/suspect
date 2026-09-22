<?php
declare(strict_types=1);
require __DIR__ . '/vendor/autoload.php';

use FixtureSdk\{Client, ClientInterface, Credentials, ClientOptions, RequestOptions, CancellationToken,
    HttpRequest, HttpResponse, Transport, SdkError, JsonNumber, WidgetInput, WidgetPatch,
    CreateWidgetInput, CreateWidgetStatus422Error, ListWidgetsInput, GetWidgetInput, UpdateWidgetInput};

function check(bool $condition, string $message): void
{
    if (!$condition) { throw new RuntimeException($message); }
}
/** @param Closure(): mixed $action */
function failure(string $kind, Closure $action): SdkError
{
    try { $action(); }
    catch (SdkError $error) { check($error->kind === $kind, 'wrong failure: ' . $error->kind . ' wanted ' . $kind); return $error; }
    throw new RuntimeException('expected ' . $kind);
}

$base = $argv[1] ?? throw new RuntimeException('provide loopback URL');
function clientInterface(ClientInterface $client): ClientInterface { return $client; }
$client = clientInterface(new Client(new Credentials(['apiKey' => 'test-key']), options: new ClientOptions(serverUrl: $base)));
check($client->createWidget(new CreateWidgetInput(body: new WidgetInput(name: 'alpha')))->body->amount->token === '9007199254740993.000000000000000001', 'create wire');
check(count($client->listWidgets(new ListWidgetsInput(tag: 'a/b 雪', tags: ['x', 'y'], labels: ['a,b', 'c'], limit: JsonNumber::fromInt(2)))->body->items) === 1, 'list wire');
$client->getWidget(new GetWidgetInput(widgetId: "a/b 雪!'()*"));
$client->updateWidget(new UpdateWidgetInput(widgetId: 'w1', body: new WidgetPatch()));
$client->getWidget(new GetWidgetInput(widgetId: '..'));
failure('request_validation', static fn () => $client->listWidgets(new ListWidgetsInput(limit: JsonNumber::fromInt(0))));
failure('request_validation', static fn () => $client->createWidget(new CreateWidgetInput(body: new WidgetInput(name: ''))));
try { $client->createWidget(new CreateWidgetInput(body: new WidgetInput(name: 'deny'))); throw new RuntimeException('documented error became success'); }
catch (CreateWidgetStatus422Error $error) {
    check($error->body->message === 'denied-private-body' && $error->status === 422, 'typed error lost');
    check($error->operationId === 'createWidget' && !str_contains((string) $error, 'denied-private-body'), 'error identity or formatting');
}
foreach (['media' => 'unexpected_media', 'duplicate' => 'unexpected_media', 'charset' => 'unexpected_media', 'encoding' => 'unexpected_encoding', 'redirect' => 'unexpected_status', 'invalid' => 'response_validation'] as $path => $kind) {
    $error = failure($kind, static fn () => $client->getWidget(new GetWidgetInput(widgetId: $path)));
    check($error->operationId === 'getWidget' && $error->source !== null, 'missing operation/source identity');
    check($error->response !== null && $error->response->status >= 200, 'missing failure capture');
}
$small = new Client(new Credentials(['apiKey' => 'test-key']), options: new ClientOptions(serverUrl: $base, maxResponseBytes: 8, maxCaptureBytes: 4));
$limited = failure('resource_limit', static fn () => $small->getWidget(new GetWidgetInput(widgetId: 'large')));
check($limited->response !== null && $limited->response->body === '{"id' && $limited->response->truncated && $limited->response->status === 200, 'response capture/ceiling');
$headers = new Client(new Credentials(['apiKey' => 'test-key']), options: new ClientOptions(serverUrl: $base, maxHeaderBytes: 256, maxCaptureBytes: 4));
failure('resource_limit', static fn () => $headers->getWidget(new GetWidgetInput(widgetId: 'headers')));
failure('timeout', static fn () => $client->getWidget(new GetWidgetInput(widgetId: 'timeout'), new RequestOptions(timeoutMilliseconds: 35)));
failure('timeout', static fn () => $client->getWidget(new GetWidgetInput(widgetId: 'split'), new RequestOptions(timeoutMilliseconds: 170)));
$cancel = new CancellationToken(); $cancel->cancel();
failure('cancelled', static fn () => $client->getWidget(new GetWidgetInput(widgetId: 'never-sent'), new RequestOptions(cancellation: $cancel)));
foreach (['====', '', "secret\r\nInjected: value", 'non ascii 雪'] as $token) { failure('credentials', static fn () => new Credentials(['apiKey' => $token])); }
foreach (['https://user:secret@example.test/api', 'http://example.test/api', 'http://127.0.0.1:bad/api', 'http://127.0.0.1:0/api', 'https://example.test/api?query', 'https://example.test/api#fragment', 'https://example.test/../api', 'https://example.test/%2e%2e/api', 'https://example.test/api/%2fsecret'] as $url) { failure('configuration', static fn () => new ClientOptions(serverUrl: $url)); }
check((new ClientOptions(serverUrl: 'HTTPS://example.test/api'))->serverUrl === 'HTTPS://example.test/api', 'URI scheme case must be accepted');
failure('configuration', static fn () => $client->getWidget(new GetWidgetInput(widgetId: 'not-sent'), new RequestOptions(timeoutMilliseconds: 30001)));

// A blocking custom adapter must honor the same deadline. The outer exchange
// also detects an adapter that returns after its deadline.
$custom = new class implements Transport {
    public bool $closed = false;
    public function send(HttpRequest $request): HttpResponse
    {
        try { usleep(50000); return new HttpResponse(200, ['content-type' => 'application/json'], '{}'); }
        finally { $this->closed = true; }
    }
};
$injected = new Client(new Credentials(['apiKey' => 'test-key']), transport: $custom, options: new ClientOptions(serverUrl: $base));
failure('timeout', static fn () => $injected->getWidget(new GetWidgetInput(widgetId: 'fixture'), new RequestOptions(timeoutMilliseconds: 10)));
check($custom->closed, 'adapter cleanup lost');
$thrower = new class implements Transport {
    public function send(HttpRequest $request): HttpResponse { throw new RuntimeException('private-transport-message'); }
};
$error = failure('transport', static fn () => (new Client(new Credentials(['apiKey' => 'test-key']), transport: $thrower))->getWidget(new GetWidgetInput(widgetId: 'fixture')));
check($error->getPrevious() !== null && !str_contains((string) $error, 'private-transport-message'), 'transport cause/redaction');

// Cancel during a real cURL body callback using an explicit process-local signal.
if (function_exists('pcntl_fork')) {
    $token = new CancellationToken(); pcntl_async_signals(true);
    pcntl_signal(SIGUSR1, static function () use ($token): void { $token->cancel(); });
    $pid = pcntl_fork();
    if ($pid === 0) { usleep(110000); posix_kill(posix_getppid(), SIGUSR1); exit(0); }
    if ($pid === -1) { throw new RuntimeException('fork failed'); }
    try { failure('cancelled', static fn () => $client->getWidget(new GetWidgetInput(widgetId: 'split'), new RequestOptions(cancellation: $token))); }
    finally { pcntl_waitpid($pid, $status); pcntl_signal(SIGUSR1, SIG_DFL); }
}
echo 'independent cURL wire, typed failures, header/capture/encoding limits, deadlines, cancellation and cleanup passed', PHP_EOL;
