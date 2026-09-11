<?php
declare(strict_types=1);
namespace __NAMESPACE__;

/** Framework transport. Honor request checks/limits and release owned resources. */
interface Transport { public function send(HttpRequest $request): HttpResponse; }
/** One owned readable body: a nonempty chunk or null at EOF. close must be idempotent. */
interface BodyReader { public function read(): ?string; public function close(): void; }
/** Pull-based response transport, with no eager whole-body buffering. */
interface StreamTransport extends Transport { public function open(HttpRequest $request): StreamResponse; }
