<?php
declare(strict_types=1);

namespace __NAMESPACE__;

require_once __DIR__ . '/Transport.php';

/** Classified failures with bounded response evidence and stable source identity. */
class SdkError extends \RuntimeException
{
    public function __construct(
        public readonly string $kind,
        string $message,
        public readonly ?ResponseCapture $response = null,
        ?\Throwable $previous = null,
        public readonly ?string $operationId = null,
        public readonly ?string $source = null,
    ) { parent::__construct($message, 0, $previous); }

    /** Preserve the original cause without including it in default error formatting. @internal */
    public function at(string $operationId, string $source): self
    {
        return new self($this->kind, $this->getMessage(), $this->response, $this, $operationId, $source);
    }
    /** @internal */
    public function withCapture(?ResponseCapture $capture): self
    {
        return new self($this->kind, $this->getMessage(), $this->response ?? $capture, $this, $this->operationId, $this->source);
    }
    public function __toString(): string { return static::class . '[' . JsonError::display($this->kind) . ']: ' . JsonError::display($this->getMessage()); }
}

/** Base for generated operation- and exact-status-specific typed exceptions. */
abstract class ApiError extends \RuntimeException
{
    public readonly int $status;
    public function __construct(public readonly string $operationId, public readonly string $source, public readonly HttpResponse|StreamResponse $response)
    {
        $this->status = $response->status;
        parent::__construct('declared API status ' . $response->status . ' for ' . JsonError::display($operationId));
    }
    public function __toString(): string { return static::class . ': ' . $this->getMessage(); }
}

/** Explicit source scheme-name → native credentials or caller-owned OAuth/OIDC hooks. */
final readonly class Credentials
{
    /** Numeric PHP keys retain their decimal source scheme names.
     * @param array<array-key, string|BasicCredential|ApiKeyCredential|AuthorizationCredential|\Closure(CredentialRequest): AuthorizationCredential> $tokens
     */
    public function __construct(#[\SensitiveParameter] private array $tokens)
    {
        foreach ($tokens as $token) {
            if (!is_string($token)) {
                if (!$token instanceof BasicCredential && !$token instanceof ApiKeyCredential && !$token instanceof AuthorizationCredential && !$token instanceof \Closure) { throw new SdkError('credentials','unknown native credential representation'); }
                continue;
            }
            if (strlen($token) > 8192 || preg_match('/\A[A-Za-z0-9._~+\/-]+=*\z/D', $token) !== 1) {
                throw new SdkError('credentials', 'bearer token must contain a nonempty token followed by optional padding');
            }
        }
    }
    public function bearer(string $scheme): string
    {
        if (!array_key_exists($scheme, $this->tokens)) { throw new SdkError('credentials', 'missing explicit credential for the declared scheme'); }
        $value=$this->tokens[$scheme];
        if (!is_string($value)) { throw new SdkError('credentials','bearer scheme requires a token'); }
        return 'Bearer ' . $value;
    }
    public function has(string $scheme): bool { return array_key_exists($scheme,$this->tokens); }
    public function resolve(CredentialRequest $request): string|BasicCredential|ApiKeyCredential|AuthorizationCredential
    {
        $value=$this->tokens[$request->scheme] ?? throw new SdkError('credentials','missing explicit credential');
        if ($value instanceof \Closure) {
            try { $value=$value($request); }
            catch (\Throwable $error) { throw new SdkError('credentials','credential hook failed',previous:$error); }
            if (!$value instanceof AuthorizationCredential) { throw new SdkError('credentials','credential hook returned an invalid representation'); }
        }
        return $value;
    }
    /** @return array<string, string> */
    public function __debugInfo(): array { return ['tokens' => '[redacted]']; }
}

/** Cooperative synchronous cancellation; also usable from a signal or framework callback. */
final class CancellationToken
{
    private bool $cancelled = false;
    public function cancel(): void { $this->cancelled = true; }
    public function isCancelled(): bool { return $this->cancelled; }
    public function throwIfCancelled(): void
    {
        if ($this->cancelled) { throw new SdkError('cancelled', 'request was cancelled'); }
    }
}

/** Client settings are immutable; finite generated ceilings may only be lowered. */
final readonly class ClientOptions
{
    public function __construct(
        public ?string $serverUrl = null,
        public int $timeoutMilliseconds = 30000,
        public int $maxResponseBytes = RuntimeConfig::MAX_RESPONSE_BYTES,
        public int $maxRequestBytes = RuntimeConfig::MAX_REQUEST_BYTES,
        public int $maxHeaderBytes = RuntimeConfig::MAX_HEADER_BYTES,
        public int $maxCaptureBytes = RuntimeConfig::MAX_CAPTURE_BYTES,
        public int $serverIndex = 0,
        /** @var array<array-key,string> */
        public array $serverVariables = [],
        public ?string $serverBaseUrl = null,
        /** Full override of the automatic ua/v1 attribution header; an empty string suppresses the header entirely. */
        public ?string $userAgent = null,
        /** Replaces the SDK identity token in the automatic attribution header: `<name>` or `<name>/<version>` of RFC 9110 tokens. */
        public ?string $applicationId = null,
    ) {
        if ($timeoutMilliseconds < 1 || $timeoutMilliseconds > 2147483647
            || $maxResponseBytes < 1 || $maxResponseBytes > RuntimeConfig::MAX_RESPONSE_BYTES
            || $maxRequestBytes < 1 || $maxRequestBytes > RuntimeConfig::MAX_REQUEST_BYTES
            || $maxHeaderBytes < 1 || $maxHeaderBytes > RuntimeConfig::MAX_HEADER_BYTES
            || $maxCaptureBytes < 0 || $maxCaptureBytes > RuntimeConfig::MAX_CAPTURE_BYTES) {
            throw new SdkError('configuration', 'invalid timeout or generated byte ceiling');
        }
        if ($serverUrl !== null) { Wire::server($serverUrl); }
        if ($serverBaseUrl !== null) { Wire::serverOrigin($serverBaseUrl); }
        if ($serverIndex < 0) { throw new SdkError('configuration','server index must be nonnegative'); }
    }
}

/** Per-call limits may lower client settings. Zero capture explicitly disables raw body capture. */
final readonly class RequestOptions
{
    public function __construct(
        public ?int $timeoutMilliseconds = null,
        public ?int $maxResponseBytes = null,
        public ?CancellationToken $cancellation = null,
        public ?int $maxCaptureBytes = null,
        public ?int $securityAlternative = null,
    ) {}
}

/** One monotonic deadline covers preparation, transport, body conversion and completion. @internal */
final readonly class CallContext
{
    private float $deadline;
    public int $maxResponseBytes;
    public int $maxCaptureBytes;
    public CallControl $control;
    public function __construct(ClientOptions $client, ?RequestOptions $options)
    {
        $timeout = $options->timeoutMilliseconds ?? $client->timeoutMilliseconds;
        $response = $options->maxResponseBytes ?? $client->maxResponseBytes;
        $capture = $options->maxCaptureBytes ?? $client->maxCaptureBytes;
        if ($timeout < 1 || $timeout > $client->timeoutMilliseconds || $response < 1 || $response > $client->maxResponseBytes || $capture < 0 || $capture > $client->maxCaptureBytes) {
            throw new SdkError('configuration', 'call options may only lower client limits');
        }
        // Floating-point monotonic time is a local deadline, never a JSON number.
        $deadline = (float) hrtime(true) / 1000000 + $timeout;
        $this->deadline = $deadline;
        $cancellation = $options?->cancellation;
        $this->maxResponseBytes = $response;
        $this->maxCaptureBytes = min($capture, $response);
        // Capture scalar state and the token, not $this: no self/closure cycle
        // keeps a completed call alive until a later PHP cycle-collector run.
        $this->control = new CallControl(static function () use ($deadline, $cancellation): void {
            $cancellation?->throwIfCancelled();
            if ((float) hrtime(true) / 1000000 >= $deadline) { throw new SdkError('timeout', 'request deadline elapsed'); }
        });
        $this->check();
    }
    public function check(): void
    {
        $this->control->check();
    }
    public function remainingMilliseconds(): int
    {
        $this->check();
        return max(1, (int) ceil($this->deadline - (float) hrtime(true) / 1000000));
    }
}

/** Immutable encoded request for a custom transport; adapters must call check while reading. */
final readonly class HttpRequest
{
    /** @param array<array-key, string> $headers */
    public function __construct(
        public string $method,
        public string $url,
        public array $headers,
        public ?string $body,
        public int $timeoutMilliseconds,
        public int $maxResponseBytes,
        public int $maxHeaderBytes,
        public int $maxCaptureBytes,
        private CallControl $control,
    ) {}
    public function check(): void { $this->control->check(); }
    /** @return array<string, string|int> */
    public function __debugInfo(): array { return ['method' => $this->method, 'url' => '[redacted]', 'headers' => '[redacted]', 'body' => '[redacted]', 'timeoutMilliseconds' => $this->timeoutMilliseconds]; }
}

/** Bounded raw prefix for SDK-owned failures. Truncation is explicit, including aborted reads. */
final readonly class ResponseCapture
{
    /** @param array<array-key, list<string>> $headers */
    public function __construct(public int $status, public array $headers, public string $body, public bool $truncated)
    {
        if (strlen($body) > RuntimeConfig::MAX_CAPTURE_BYTES) { throw new SdkError('configuration', 'capture exceeds generated byte ceiling'); }
    }
}

/** Completed bounded body plus duplicate-preserving, case-normalized response headers. */
final readonly class HttpResponse
{
    /** @var array<array-key, list<string>> */
    public array $headers;
    public int $headerBytes;
    /** @param array<array-key, string|list<string>> $headers */
    public function __construct(public int $status, array $headers, public string $body)
    {
        if ($status < 100 || $status > 599) { throw new SdkError('transport', 'invalid HTTP status'); }
        $this->headers = Wire::headers($headers, RuntimeConfig::MAX_HEADER_BYTES);
        $this->headerBytes = Wire::headerBytes($this->headers);
    }
    /** @return list<string> */
    public function headerValues(string $name): array { return $this->headers[strtolower($name)] ?? []; }
    /** Convenient comma-joined header view; use headerValues for Set-Cookie and other repeated fields. */
    public function header(string $name): ?string
    {
        $values = $this->headerValues($name);
        return $values === [] ? null : implode(', ', $values);
    }
    public function capture(int $maxBytes, bool $truncated = false): ResponseCapture
    {
        return new ResponseCapture($this->status, $this->headers, substr($this->body, 0, $maxBytes), $truncated || strlen($this->body) > $maxBytes);
    }
}

/**
 * Framework/PSR-18 bridge seam. Send once; preserve header multiplicity and raw
 * content encoding. Honor check(), timeout and byte ceilings while reading.
 * Release transport-owned body/connection resources before returning or throwing.
 */

/** cURL with TLS verification, bounded callbacks, explicit credential policy and deterministic handle lifetime. */
final class CurlTransport implements StreamTransport
{
    public function open(HttpRequest $request): StreamResponse { return CurlBody::open($request); }
    public function send(HttpRequest $request): HttpResponse
    {
        if (!extension_loaded('curl') || !defined('CURLOPT_PROTOCOLS_STR')) { throw new SdkError('transport', 'CurlTransport requires ext-curl with libcurl 7.85 or later'); }
        $request->check(); $urlParts=Wire::serverOrigin($request->url);
        $curl = curl_init();
        if ($curl === false) { throw new SdkError('transport', 'unable to initialize cURL'); }
        $body = ''; $headers = []; $headerBytes = 0; $status = 0; $failure = null;
        $lines = [];
        foreach ($request->headers as $name => $value) { $lines[] = $name . ': ' . $value; }
        $lines[] = 'Expect:';
        $options = [
            CURLOPT_URL => $request->url, CURLOPT_CUSTOMREQUEST => $request->method,
            CURLOPT_REQUEST_TARGET => (isset($urlParts['path'])&&$urlParts['path']!==''?$urlParts['path']:'/').(isset($urlParts['query'])?'?'.$urlParts['query']:''),
            CURLOPT_HTTPHEADER => $lines, CURLOPT_FOLLOWLOCATION => false, CURLOPT_MAXREDIRS => 0,
            CURLOPT_NOBODY => $request->method === 'HEAD',
            CURLOPT_FORBID_REUSE => true, CURLOPT_FRESH_CONNECT => true,
            CURLOPT_PROXY => '', CURLOPT_NETRC => CURL_NETRC_IGNORED,
            CURLOPT_PROTOCOLS_STR => 'http,https', CURLOPT_REDIR_PROTOCOLS_STR => 'http,https',
            CURLOPT_SSL_VERIFYPEER => true, CURLOPT_SSL_VERIFYHOST => 2, CURLOPT_PATH_AS_IS => true,
            CURLOPT_HTTP_CONTENT_DECODING => false, CURLOPT_HTTP_TRANSFER_DECODING => true,
            CURLOPT_TIMEOUT_MS => $request->timeoutMilliseconds,
            CURLOPT_CONNECTTIMEOUT_MS => min($request->timeoutMilliseconds, 10000),
            CURLOPT_NOSIGNAL => true, CURLOPT_NOPROGRESS => false,
            CURLOPT_XFERINFOFUNCTION => static function (\CurlHandle $handle, float $downloadSize, float $downloaded, float $uploadSize, float $uploaded) use ($request, &$failure): int {
                try { $request->check(); return 0; }
                catch (SdkError $e) { $failure ??= [$e->kind, $e->getMessage()]; return 1; }
            },
            CURLOPT_WRITEFUNCTION => static function (\CurlHandle $handle, string $chunk) use (&$body, &$failure, $request): int {
                try { $request->check(); }
                catch (SdkError $e) { $failure ??= [$e->kind, $e->getMessage()]; return 0; }
                $remaining = $request->maxResponseBytes - strlen($body);
                if (strlen($chunk) > $remaining) {
                    $body .= substr($chunk, 0, $remaining);
                    $failure ??= ['resource_limit', 'response exceeds byte ceiling']; return 0;
                }
                $body .= $chunk; return strlen($chunk);
            },
            CURLOPT_HEADERFUNCTION => static function (\CurlHandle $handle, string $line) use (&$headers, &$headerBytes, &$status, &$failure, $request): int {
                try {
                    $request->check(); $headerBytes += strlen($line);
                    if ($headerBytes > $request->maxHeaderBytes) { throw new SdkError('resource_limit', 'response headers exceed byte ceiling'); }
                    if (str_starts_with($line, 'HTTP/')) {
                        if (preg_match('/\AHTTP\/[0-9.]+ ([0-9]{3})(?:[ \r\n]|$)/D', $line, $match) !== 1) { throw new SdkError('transport', 'invalid response status line'); }
                        $status = (int) $match[1]; $headers = [];
                    } elseif (trim($line) !== '') {
                        $colon = strpos($line, ':');
                        if ($colon === false || $line[0] === ' ' || $line[0] === "\t") { throw new SdkError('transport', 'invalid response header framing'); }
                        $name = strtolower(substr($line, 0, $colon));
                        $value = trim(substr($line, $colon + 1), " \t\r\n");
                        Wire::headerField($name, $value);
                        $headers[$name][] = $value;
                    }
                    return strlen($line);
                } catch (SdkError $e) { $failure ??= [$e->kind, $e->getMessage()]; return 0; }
            },
        ];
        if ($request->body !== null) { $options[CURLOPT_POSTFIELDS] = $request->body; }
        try {
            if (!curl_setopt_array($curl, $options)) { throw new SdkError('transport', 'unable to configure cURL'); }
            $ok = curl_exec($curl);
            if ($failure === null) {
                try { $request->check(); }
                catch (SdkError $e) { $failure = [$e->kind, $e->getMessage()]; }
            }
            if ($failure !== null || $ok === false) {
                $failure ??= [curl_errno($curl) === CURLE_OPERATION_TIMEDOUT ? 'timeout' : 'transport', 'HTTP transport failed'];
                $capture = $status >= 100 && $status <= 599 ? new ResponseCapture($status, $headers, substr($body, 0, $request->maxCaptureBytes), true) : null;
                // Construct the exception after callbacks have returned. With
                // zend.exception_ignore_args=0, a callback exception's trace
                // would otherwise retain its CurlHandle argument and resources.
                throw new SdkError($failure[0], $failure[1], $capture);
            }
            return new HttpResponse((int) curl_getinfo($curl, CURLINFO_RESPONSE_CODE), $headers, $body);
        } finally {
            // No pooled handle or cookie engine survives this call. Releasing the
            // CurlHandle works on both PHP 8.3 and 8.5 (curl_close is deprecated).
            unset($curl);
        }
    }
}

/** Shared bounded serializer and protocol checks. @internal */
final class Wire
{
    private const TOKEN = "!#$%&'*+-.^_`|~0123456789abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ";

    public static function server(string $url): void
    {
        $parts = self::serverOrigin($url);
        if (isset($parts['query'])) { throw new SdkError('configuration', 'server URL cannot contain a query'); }
        foreach (explode('/', $parts['path'] ?? '') as $segment) {
            // Encoded dots/slashes are source URI data, not literal path operators.
            if ($segment === '.' || $segment === '..') { throw new SdkError('configuration', 'server prefix contains an unresolved literal dot segment'); }
        }
    }
    /** @return array{scheme?: string, host?: string, port?: int, user?: string, pass?: string, path?: string, query?: string, fragment?: string} */
    public static function serverOrigin(string $url): array
    {
        if (strlen($url) > RuntimeConfig::MAX_REQUEST_BYTES || preg_match('/[^\x21-\x7e]|\\\\|%(?![0-9A-Fa-f]{2})/', $url) === 1) { throw new SdkError('configuration', 'invalid URL bytes'); }
        $parts = parse_url($url);
        if ($parts === false || !isset($parts['scheme'], $parts['host']) || $parts['host'] === '' || isset($parts['user']) || isset($parts['pass']) || isset($parts['fragment']) || (isset($parts['port']) && ($parts['port'] < 1 || $parts['port'] > 65535))) { throw new SdkError('configuration', 'expected absolute URL without userinfo or fragment'); }
        $scheme = strtolower($parts['scheme']);
        $secure = $scheme === 'https';
        $loopback = $scheme === 'http' && in_array(strtolower($parts['host']), ['127.0.0.1', '[::1]', 'localhost'], true);
        if (!$secure && !$loopback) { throw new SdkError('configuration', 'server must be HTTPS or an explicit HTTP loopback fixture'); }
        return $parts;
    }
    public static function headerField(string $name, string $value): void
    {
        if ($name === '' || strspn($name, self::TOKEN) !== strlen($name) || preg_match('/[\x00-\x08\x0a-\x1f\x7f]/', $value) === 1) { throw new SdkError('transport', 'invalid header name or value'); }
    }
    /** @param array<array-key, string|list<string>> $headers
     * @return array<array-key, list<string>>
     */
    public static function headers(array $headers, int $maxBytes): array
    {
        $normalized = []; $bytes = 0;
        foreach ($headers as $name => $values) {
            $name = strtolower((string) $name);
            if (is_string($values)) { $values = [$values]; }
            if (!array_is_list($values) || $values === []) { throw new SdkError('transport', 'header values must be nonempty lists'); }
            foreach ($values as $value) {
                $bytes += strlen($name) + strlen($value) + 4;
                if ($bytes > $maxBytes) { throw new SdkError('resource_limit', 'headers exceed byte ceiling'); }
                self::headerField($name, $value); $normalized[$name][] = $value;
            }
        }
        return $normalized;
    }
    /** @param array<array-key, list<string>> $headers */
    public static function headerBytes(array $headers): int
    {
        $bytes = 0;
        foreach ($headers as $name => $values) { foreach ($values as $value) { $bytes += strlen((string) $name) + strlen($value) + 4; } }
        return $bytes;
    }
    public static function segment(string $value, int $maxBytes = RuntimeConfig::MAX_REQUEST_BYTES): string
    {
        return $value === '.' || $value === '..' ? str_replace('.', '%2E', $value) : self::encoded($value, $maxBytes);
    }
    public static function scalar(JsonValue $value): string
    {
        return match ($value->kind) {
            JsonKind::String => $value->asString(), JsonKind::Number => $value->asNumber()->token,
            JsonKind::Boolean => $value->asBool() ? 'true' : 'false',
            default => throw new SdkError('request_validation', 'parameter has no admitted scalar serialization'),
        };
    }
    /** @param list<string> $query */
    public static function query(array &$query, int &$bytes, string $name, JsonValue $value, bool $array, bool $explode, int $maxBytes): void
    {
        $key = self::encoded($name, $maxBytes);
        $append = static function (string $part) use (&$query, &$bytes, $maxBytes): void {
            $cost = strlen($part) + ($query === [] ? 0 : 1);
            if ($cost > $maxBytes - $bytes) { throw new SdkError('resource_limit', 'query exceeds request byte ceiling'); }
            $bytes += $cost; $query[] = $part;
        };
        if (!$array) { $append($key . '=' . self::encoded(self::scalar($value), $maxBytes)); }
        elseif ($explode) { foreach ($value->asArray() as $item) { $append($key . '=' . self::encoded(self::scalar($item), $maxBytes)); } }
        else {
            $part = $key . '='; $comma = false;
            foreach ($value->asArray() as $item) {
                $piece = ($comma ? ',' : '') . self::encoded(self::scalar($item), $maxBytes); $comma = true;
                if (strlen($piece) > $maxBytes - strlen($part)) { throw new SdkError('resource_limit', 'query exceeds request byte ceiling'); }
                $part .= $piece;
            }
            $append($part);
        }
    }
    private static function encoded(string $value, int $maxBytes): string
    {
        if (strlen($value) > $maxBytes) { throw new SdkError('resource_limit', 'parameter exceeds request ceiling'); }
        $encoded = rawurlencode($value);
        if (strlen($encoded) > $maxBytes) { throw new SdkError('resource_limit', 'encoded parameter exceeds request ceiling'); }
        return $encoded;
    }
    public static function contentEncoding(HttpResponse $response, int $capture): void
    {
        $encodings = $response->headerValues('content-encoding');
        if ($encodings !== [] && (count($encodings) !== 1 || strtolower(trim($encodings[0])) !== 'identity')) { throw new SdkError('unexpected_encoding', 'only identity content encoding is admitted', $response->capture($capture)); }
    }
    public static function jsonResponse(HttpResponse $response, int $capture): void
    {
        $values = $response->headerValues('content-type');
        $valid = count($values) === 1 && self::jsonMedia($values[0]);
        if (!$valid) { throw new SdkError('unexpected_media', 'expected one declared application/json content type with UTF-8 encoding', $response->capture($capture)); }
    }
    private static function jsonMedia(string $value): bool
    {
        $semi = strpos($value, ';');
        if (strtolower(trim(substr($value, 0, $semi === false ? strlen($value) : $semi))) !== 'application/json') { return false; }
        if ($semi === false) { return true; }
        $at = $semi; $length = strlen($value); $seen = [];
        while ($at < $length) {
            if ($value[$at++] !== ';') { return false; }
            while ($at < $length && str_contains(" \t", $value[$at])) { ++$at; }
            $start = $at;
            while ($at < $length && str_contains(self::TOKEN, $value[$at])) { ++$at; }
            $name = strtolower(substr($value, $start, $at - $start));
            if ($name === '' || isset($seen[$name])) { return false; } $seen[$name] = true;
            while ($at < $length && str_contains(" \t", $value[$at])) { ++$at; }
            if (($value[$at++] ?? '') !== '=') { return false; }
            while ($at < $length && str_contains(" \t", $value[$at])) { ++$at; }
            $parameter = '';
            if (($value[$at] ?? '') === '"') {
                ++$at; $closed = false;
                while ($at < $length) {
                    $byte = $value[$at++];
                    if ($byte === '"') { $closed = true; break; }
                    if ($byte === '\\') { if ($at === $length) { return false; } $byte = $value[$at++]; }
                    $parameter .= $byte;
                }
                if (!$closed) { return false; }
            } else {
                $start = $at;
                while ($at < $length && str_contains(self::TOKEN, $value[$at])) { ++$at; }
                if ($start === $at) { return false; }
                $parameter = substr($value, $start, $at - $start);
            }
            if ($name === 'charset' && strtolower($parameter) !== 'utf-8') { return false; }
            while ($at < $length && str_contains(" \t", $value[$at])) { ++$at; }
        }
        return true;
    }
}
