const assert = require('node:assert/strict');
const fs = require('node:fs/promises');
const os = require('node:os');
const path = require('node:path');
const { test } = require('node:test');
const { availableSdkProfiles, generationArgs, runGeneration, readSdkSessionIdentity } = require('../dist/generation.js');

const binary = process.env.SUSPECT_TEST_BINARY;
if (!binary || !path.isAbsolute(binary)) throw new Error('Set SUSPECT_TEST_BINARY to the freshly built CLI');

test('every advertised native profile accepts an expanded read-only plan in its canonical artifact directory', async (t) => {
  const profiles = await availableSdkProfiles(binary);
  for (const required of ['typescript-http', 'rust-http', 'python-http', 'go-http', 'swift-http', 'ruby-http', 'csharp-http', 'java-http']) {
    assert.ok(profiles.some((entry) => entry.profile === required), `Fresh default CLI is missing ${required}`);
  }
  const packages = {
    'typescript-http': ['@fixture/discovered-sdk'],
    'rust-http': ['fixture-discovered-sdk'],
    'python-http': ['fixture-discovered-sdk', 'discovered_sdk'],
    'go-http': ['example.com/fixture/discovered-sdk'],
    'swift-http': ['DiscoveredSDK', 'DiscoveredClient'],
    'ruby-http': ['fixture-discovered-sdk', 'DiscoveredSDK'],
    'csharp-http': ['Fixture.Discovered', 'Fixture.Discovered.Client'],
    'java-http': ['com.example:discovered-sdk', 'com.example.discovered'],
    'kotlin-http': ['com.example:discovered-sdk', 'com.example.discovered'],
    'php-http': ['example/discovered-sdk', 'Fixture\\Discovered'],
    'dart-http': ['fixture_discovered_sdk'],
    'cpp-http': ['fixture_discovered_sdk', 'fixture_discovered'],
  };
  const root = await fs.mkdtemp(path.join(process.env.SUSPECT_TEST_TMPDIR ?? os.tmpdir(), 'suspect advertised profiles '));
  try {
    const spec = path.join(root, 'api input.json');
    const source = JSON.stringify({
      openapi: '3.1.0', info: { title: 'Registry-backed package fixture', version: 'API-version' },
      servers: [{ url: 'https://example.com/v1' }], security: [{ Bearer: [] }],
      components: { securitySchemes: { Bearer: { type: 'http', scheme: 'bearer' } } },
      paths: {
        '/things': { get: { operationId: 'getThings', responses: {
          '200': { description: 'Things', content: { 'application/json': { schema: { type: 'string' } } } },
        } } },
        '/health': { get: { operationId: 'getHealth', security: [], responses: { '204': { description: 'empty' } } } },
      },
    }, null, 2);
    await fs.writeFile(spec, source);
    for (const { profile, directory } of profiles) {
      await t.test(profile, async () => {
        assert.ok(packages[profile], `Add a native package identity fixture for ${profile}`);
        const [packageName, importName] = packages[profile];
        const out = path.join(root, 'preview output', profile);
        await assert.rejects(runGeneration(binary, generationArgs(spec, out, {
          kind: profile, packageName, packageVersion: '1.2.3', importName, operationIds: [], check: true,
        }), root), (error) => {
          const report = JSON.parse(error.message.slice(error.message.indexOf('{\n'), error.message.lastIndexOf('}') + 1));
          assert.equal(report.profile, profile);
          assert.equal(report.status, 'drift', JSON.stringify(report.diagnostics));
          assert.equal(report.releaseReady, false);
          assert.deepEqual(report.compatibilityProfiles, []);
          assert.deepEqual(report.operations.map((operation) => operation.operationId).sort(), ['getHealth', 'getThings']);
          assert.ok(report.artifacts.includes(`${directory}/README.md`));
          assert.ok(report.artifacts.every((artifact) => artifact.startsWith(`${directory}/`)), 'all artifacts use the discovered canonical directory');
          return true;
        });
        await assert.rejects(fs.stat(out), { code: 'ENOENT' });
      });
    }
    await assert.rejects(fs.stat(path.join(root, 'preview output')), { code: 'ENOENT' });
    assert.equal(await fs.readFile(spec, 'utf8'), source);
  } finally { await fs.rm(root, { recursive: true, force: true }); }
});

test('editor discovers compiled profiles and generates the advertised Ruby package with explicit namespace', async () => {
  const profiles = await availableSdkProfiles(binary);
  assert.ok(profiles.some((item) => item.profile === 'ruby-http' && item.directory === 'ruby'));
  assert.equal(new Set(profiles.map((item) => item.profile)).size, profiles.length);
  const root = await fs.mkdtemp(path.join(process.env.SUSPECT_TEST_TMPDIR ?? os.tmpdir(), 'suspect profile discovery '));
  try {
    const spec = path.join(root, 'api.json');
    const out = path.join(root, 'output');
    await fs.writeFile(spec, JSON.stringify({
      openapi: '3.1.0', info: { title: 'Profile discovery', version: '1' },
      servers: [{ url: 'https://example.com/v1' }], security: [{ Bearer: [] }],
      components: { securitySchemes: { Bearer: { type: 'http', scheme: 'bearer' } } },
      paths: { '/things': { get: { operationId: 'getThings', responses: {
        '200': { description: 'Things', content: { 'application/json': { schema: { type: 'string' } } } },
      } } } },
    }));
    const generation = { kind: 'ruby-http', packageName: 'profile-demo', packageVersion: '1.0.0', importName: 'ProfileDemo', operationIds: ['getThings'] };
    await runGeneration(binary, generationArgs(spec, out, generation), root);
    assert.ok((await fs.stat(path.join(out, 'ruby', 'lib', 'profile_demo.rb'))).isFile());
    const before = await fs.stat(path.join(out, 'ruby', 'README.md'));
    await runGeneration(binary, generationArgs(spec, out, { ...generation, check: true }), root);
    const after = await fs.stat(path.join(out, 'ruby', 'README.md'));
    assert.equal(after.mtimeMs, before.mtimeMs);
    assert.equal(after.ino, before.ino);
  } finally {
    await fs.rm(root, { recursive: true, force: true });
  }
});

test('pinned session identity follows its lexical manifest path', async () => {
  const root = await fs.mkdtemp(path.join(process.env.SUSPECT_TEST_TMPDIR ?? os.tmpdir(), 'suspect pinned identity '));
  try {
    const config = path.join(root, 'session.json');
    await fs.writeFile(config, JSON.stringify({ pins: { manifest: 'pins/source.json', cache_dir: 'cache' }, targets: [] }));
    const identity = await readSdkSessionIdentity(config, 'generated');
    assert.equal(identity.sourcePath, path.join(root, 'pins', 'source.json'));
    assert.equal(identity.sourceRoot, path.join(root, 'pins'));
    assert.equal(identity.outDirectory, path.join(root, 'generated'));
  } finally {
    await fs.rm(root, { recursive: true, force: true });
  }
});
