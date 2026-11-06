<?php
declare(strict_types=1);

require __DIR__ . '/vendor/autoload.php';

use FixtureSdk\{Absent, Client, ClientOptions, Codecs, Credentials, EchoPetInput, HttpRequest,
    HttpResponse, JsonError, JsonNumber, ListPetsInput, Owner, Pet, PetMood, PetStatus, SdkError,
    Transport, ValidationError};

function check(bool $condition, string $message): void
{
    if (!$condition) { throw new RuntimeException($message); }
}

/** @param Closure(): mixed $action */
function invalid(Closure $action): void
{
    try { $action(); }
    catch (JsonError|ValidationError|TypeError $e) { return; }
    throw new RuntimeException('invalid value accepted');
}

/** @param Closure(): mixed $action */
function failure(string $kind, Closure $action): void
{
    try { $action(); }
    catch (SdkError $error) { check($error->kind === $kind, 'wrong failure: ' . $error->kind . ' wanted ' . $kind); return; }
    throw new RuntimeException('expected ' . $kind);
}

// Scalar nullable: the same-object type gains null; the required key stays required.
$pet = Pet::fromJson('{"id":1,"name":"rex","tag":null,"owner":{"name":"ann"}}');
check($pet->tag === null, 'scalar nullable lost');
check($pet->owner instanceof Owner && $pet->owner->name === 'ann', 'reference target lost');
check($pet->nickname === Absent::Value && $pet->status === Absent::Value, 'omission lost');
check(str_contains($pet->toJson(), '"tag":null'), 'explicit null not written');
invalid(static fn () => Pet::fromJson('{"id":1,"name":"rex","owner":null}'));
invalid(static fn () => Pet::fromJson('{"id":1,"name":null,"tag":"t","owner":null}'));

// Object nullable through a Reference Object whose siblings (type: string,
// readOnly) are ignored: null and objects are accepted, strings are not.
$orphan = Pet::fromJson('{"id":2,"name":"rex","tag":"t","owner":null}');
check($orphan->owner === null, 'object nullable lost');
check(Codecs::decodeOwner('null') === null, 'nullable object codec');
invalid(static fn () => Pet::fromJson('{"id":2,"name":"rex","tag":"t","owner":"ann"}'));

// A $ref'd nullable scalar is nullable through its target and keeps its constraints.
$named = Pet::fromJson('{"id":3,"name":"rex","tag":"t","owner":null,"nickname":null}');
check($named->nickname === null, 'referenced nullable scalar lost');
check(Codecs::decodeNickname('"r"') === 'r', 'referenced scalar value');
invalid(static fn () => Pet::fromJson('{"id":3,"name":"rex","tag":"t","owner":null,"nickname":""}'));

// nullable cannot bypass an enum without null; an enum listing null admits it.
$moody = Pet::fromJson('{"id":4,"name":"rex","tag":"t","owner":null,"status":"sold","mood":null}');
check($moody->status === PetStatus::Sold && $moody->mood === null, 'enum carriers');
check(Codecs::decodePetMood('"calm"') === PetMood::Calm, 'enum value');
invalid(static fn () => Pet::fromJson('{"id":4,"name":"rex","tag":"t","owner":null,"status":null}'));

// nullable without a same-object type changes nothing: arbitrary JSON.
$anything = Pet::fromJson('{"id":5,"name":"rex","tag":"t","owner":null,"anything":[1,"x",null]}');
check($anything->anything !== Absent::Value, 'arbitrary JSON lost');

// Boolean exclusive bounds modify the adjacent bound: 0 < id < 100, 0 < weight <= 50.
foreach (['0', '100', '-1'] as $id) {
    invalid(static fn () => Pet::fromJson('{"id":' . $id . ',"name":"rex","tag":"t","owner":null}'));
}
check(Pet::fromJson('{"id":99,"name":"rex","tag":"t","owner":null,"weight":50}')->weight instanceof JsonNumber, 'inclusive maximum rejected');
invalid(static fn () => Pet::fromJson('{"id":99,"name":"rex","tag":"t","owner":null,"weight":0}'));
$mutated = new Pet(id: JsonNumber::fromInt(1), name: 'rex', owner: null, tag: null);
$mutated->id = JsonNumber::fromInt(100);
invalid(static fn () => $mutated->toJson());

// The same semantics hold over the generated client.
$transport = new class implements Transport {
    /** @var list<HttpRequest> */
    public array $requests = [];
    public function send(HttpRequest $request): HttpResponse
    {
        $request->check();
        $this->requests[] = $request;
        if (str_ends_with($request->url, '/photo')) {
            return new HttpResponse(200, ['Content-Type' => 'application/octet-stream'], "\x00\xff");
        }
        if (str_contains($request->url, '/pets?') || str_ends_with($request->url, '/pets') && $request->method === 'GET') {
            return new HttpResponse(200, ['Content-Type' => 'application/json'], '[{"id":1,"name":"rex","tag":null,"owner":null}]');
        }
        return new HttpResponse(200, ['Content-Type' => 'application/json'], $request->body ?? '');
    }
};
$client = new Client(new Credentials(['bearer' => 'oas30-token']), $transport, new ClientOptions(serverUrl: 'https://example.test/api/v1'));
$echo = $client->echoPet(new EchoPetInput(body: new Pet(id: JsonNumber::fromInt(7), name: 'rex', owner: new Owner(name: 'ann'), tag: null, nickname: null)));
check($echo->body->tag === null && $echo->body->nickname === null && $echo->body->owner?->name === 'ann', 'nullable wire round trip');
$listed = $client->listPets(new ListPetsInput(limit: JsonNumber::fromInt(100)));
check(count($listed->body) === 1, 'list response');
foreach ($listed->body as $item) { check($item->owner === null && $item->tag === null, 'list item nullable'); }
$urls = array_map(static fn (HttpRequest $request): string => $request->url, $transport->requests);
check(count(array_filter($urls, static fn (string $url): bool => str_ends_with($url, '/pets?limit=100'))) === 1, 'inclusive maximum query');
failure('request_validation', static fn () => $client->listPets(new ListPetsInput(limit: JsonNumber::fromInt(0))));
failure('request_validation', static fn () => $client->echoPet(new EchoPetInput(body: new Pet(id: JsonNumber::fromInt(0), name: 'rex', owner: null, tag: null))));
$photo = $client->getPhoto();
check($photo->body->value === "\x00\xff", 'OAS 3.0 binary bytes');
check(count($transport->requests) === 3, 'invalid requests reached the transport');
echo 'OAS 3.0 nullable, exclusive bounds, reference-only siblings and binary passed', PHP_EOL;
