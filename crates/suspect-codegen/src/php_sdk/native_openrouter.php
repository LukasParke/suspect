<?php
declare(strict_types=1);
require __DIR__ . '/vendor/autoload.php';

use FixtureSdk\{Client, Credentials, ClientOptions, JsonNumber, Absent,
    CreateKeysInput, UpdateKeysInput, ListContainerFilesInput, GetContainerFileInput,
    __CREATE__, __UPDATE__};

function check(bool $condition, string $message): void
{
    if (!$condition) { throw new RuntimeException($message); }
}
$base = $argv[1] ?? throw new RuntimeException('provide a loopback URL');
$client = new Client(new Credentials(['apiKey' => 'test-key']), options: new ClientOptions(serverUrl: $base));
$patch = new __UPDATE__();
check($patch->toJson() === '{}', 'patch omission');
$patch->limit = null;
check($patch->toJson() === '{"limit":null}', 'patch explicit null');
$patch->limit = JsonNumber::fromString('75.50');
check($patch->toJson() === '{"limit":75.50}', 'patch exact value');
$patch->limit = Absent::Value;
check($patch->toJson() === '{}', 'patch omission restored');
$credits = $client->getCredits();
check($credits->body->data->totalCredits->token === '100.50000000000000001', 'credits precision');
$created = $client->createKeys(new CreateKeysInput(body: new __CREATE__(name: 'Native Test Key', limit: JsonNumber::fromString('50.25'), limitReset: null)));
check($created->response->status === 201, 'source create status');
check($created->body->data->limit instanceof JsonNumber && $created->body->data->limit->token === '50.250', 'create precision');
check($created->body->data->updatedAt === null, 'required nullable response');
$updated = $client->updateKeys(new UpdateKeysInput(hash: 'fixture-hash', body: new __UPDATE__(name: 'Updated Native Key', limit: JsonNumber::fromString('75.50'), limitReset: null, disabled: true)));
check($updated->body->data->limit instanceof JsonNumber && $updated->body->data->limit->token === '75.50', 'update precision');
$listed = $client->listContainerFiles(new ListContainerFilesInput(containerId: 'sess_abc123', limit: JsonNumber::fromInt(2), after: 'a/b 雪'));
check(!$listed->body->hasMore, 'source list response');
$file = $client->getContainerFile(new GetContainerFileInput(containerId: 'sess_abc123', fileId: 'a/b 雪'));
check($file->body->bytes->toInt() === 123, 'integer file bytes');
echo 'five actual OpenRouter operations: native types, source names, presence, exact values and HTTP passed', PHP_EOL;
