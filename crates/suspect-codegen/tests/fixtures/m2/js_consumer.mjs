// Independently specified recording HTTP fixture: a local node:http server
// answers with hand-authored exact bytes; no real network leaves loopback.
import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { operations, createClient } from '__PACKAGE__';

const WIDGET =
  '{"id":"w1","amount":9007199254740993.000000000000000001,"meta":null,"child":{"label":"root"},"payload":{"kind":"standard","text":"plain"}}';
const WIDGET_UPDATED_META =
  '{"id":"w1","amount":0.0000000000000000001,"meta":"present","payload":{"kind":"standard","text":"plain"}}';
const LIST =
  '{"items":[{"id":"w2","amount":0.0000000000000000001,"payload":{"kind":"secure","vault":"vlt-1"}}]}';

const observed = [];
const server = createServer(async (request, response) => {
  let body = '';
  for await (const chunk of request) body += chunk;
  observed.push({
    method: request.method,
    url: request.url,
    auth: request.headers.authorization,
    accept: request.headers.accept,
    body,
  });
  const fixture =
    request.method === 'GET' && request.url.startsWith('/api/v1/widgets?') ? LIST :
    request.method === 'PATCH' ? WIDGET_UPDATED_META : WIDGET;
  const denied=request.method==='POST' && body==='{"name":"deny"}';
  response.writeHead(denied ? 422 : 200, { 'content-type': 'application/json; charset=utf-8' });
  response.end(denied ? '{"message":"rejected"}' : fixture);
});
await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve));

try {
  const serverURL = `http://127.0.0.1:${server.address().port}/api/v1`;
  const client = createClient({ auth: { apiKey: 'm2-key' }, serverURL });

  const created = await client.createWidget({ body: { name: 'alpha' } });
  assert.equal(created.status, 200);
  assert.equal(created.contentType, 'application/json');
  assert.equal(created.data.amount.toString(), '9007199254740993.000000000000000001');
  assert.equal(created.data.payload.kind, 'standard');
  assert.equal(created.data.child.label, 'root');
  assert.equal(created.data.child.child, undefined);
  assert.equal(created.data.meta, null);

  const listed = await client.listWidgets({ tag: 'a', tags: ['x', 'y'], labels: ['a,b', 'c'], limit: 2 });
  assert.equal(listed.data.items[0].amount.toString(), '0.0000000000000000001');
  assert.equal(listed.data.items[0].payload.kind, 'secure');
  assert.equal(listed.data.items[0].payload.vault, 'vlt-1');

  const updated = await client.updateWidget({ widget_id: 'w1', body: {} });
  assert.equal(updated.data.amount.toString(), '0.0000000000000000001');
  assert.equal(updated.data.meta, 'present');

  const read = await client.getWidget({ widget_id: 'a/b 雪!\'()*' });
  assert.equal(read.data.id, 'w1');

  assert.deepEqual(observed, [
    { method: 'POST', url: '/api/v1/widgets', auth: 'Bearer m2-key', accept: 'application/json', body: '{"name":"alpha"}' },
    { method: 'GET', url: '/api/v1/widgets?tag=a&tags=x&tags=y&labels=a%2Cb,c&limit=2', auth: 'Bearer m2-key', accept: 'application/json', body: '' },
    { method: 'PATCH', url: '/api/v1/widgets/w1', auth: 'Bearer m2-key', accept: 'application/json', body: '{}' },
    { method: 'GET', url: '/api/v1/widgets/a%2Fb%20%E9%9B%AA%21%27%28%29%2A', auth: 'Bearer m2-key', accept: 'application/json', body: '' },
  ]);

  for (const invalid of [
    () => client.createWidget({ body: { name: '' } }),
    () => client.createWidget({ body: {} }),
    () => client.getWidget({ widget_id: '' }),
    () => client.listWidgets({ tag: 'a', unknown: 1 }),
  ]) {
    await assert.rejects(invalid(), (error) =>
      operations.isSdkError(error) &&
      ['request-validation', 'request-representation'].includes(error.kind),
    );
  }
  assert.equal(observed.length, 4, 'invalid input never reaches transport');
  const mutable={name:'alpha'};mutable.name='';
  await assert.rejects(client.createWidget({body:mutable}),error=>operations.isSdkError(error) && error.kind==='request-validation');
  await assert.rejects(client.createWidget({body:{name:'deny'}}),error=>operations.isCreateWidgetApiError(error) && error.response.status===422 && error.response.data.message==='rejected');
  assert.deepEqual(observed[4],{method:'POST',url:'/api/v1/widgets',auth:'Bearer m2-key',accept:'application/json',body:'{"name":"deny"}'});
  const controller=new AbortController();controller.abort();
  await assert.rejects(client.getWidget({widget_id:'w1'},{signal:controller.signal}),error=>operations.isSdkError(error) && error.kind==='cancelled');
  assert.equal(observed.length,5,'mutated invalid inputs and pre-aborted calls never send');
} finally {
  await new Promise((resolve) => server.close(resolve));
}
