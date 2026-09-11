// Strict typed consumer: documented requests through an injected fetch, exact
// decimal fidelity, source-tagged oneOf, recursion and negative type cases.
import { operations, createClient, type models } from '__PACKAGE__';

const WIDGET =
  '{"id":"w1","amount":9007199254740993.000000000000000001,"meta":null,"child":{"label":"root"},"payload":{"kind":"standard","text":"plain"}}';

const options: operations.ClientOptions = {
  auth: { apiKey: 'm2-key' },
  serverURL: 'https://m2.example.test/api/v1',
  fetch: async (input, init) => {
    if (String(input) !== 'https://m2.example.test/api/v1/widgets') throw new Error('incorrect documented request');
    if (init?.method !== 'POST') throw new Error('incorrect method');
    if (new Headers(init.headers).get('authorization') !== 'Bearer m2-key') throw new Error('incorrect auth');
    return new Response(WIDGET, { status: 200, headers: { 'content-type': 'application/json' } });
  },
};

const client = createClient(options);
// OpenAPI's default object openness remains usable in native types.
export const openInput: operations.CreateWidgetInput = {body:{name:'alpha',extra:1}};
const created: operations.CreateWidgetSuccess = await client.createWidget({ body: { name: 'alpha' } });
const precise: string = created.data.amount.toString();
if (precise !== '9007199254740993.000000000000000001') throw new Error('rounded the response');
if (created.data.payload.kind !== 'standard') throw new Error('source-tagged oneOf differs');
if (created.data.payload.kind === 'standard' && created.data.payload.text !== 'plain') throw new Error('variant payload differs');
if (created.data.child?.label !== 'root' || created.data.child?.child !== undefined) throw new Error('recursive optional member differs');
if (created.data.meta !== null) throw new Error('null is distinct from absent');

if (false) {
  // @ts-expect-error: an undeclared operation-input slot cannot be sent
  void client.createWidget({ body: { name: 'a' }, unknown: 1 });
  // @ts-expect-error: the required body cannot be omitted
  void client.createWidget({});
  // @ts-expect-error: null is different from an absent request body
  void client.createWidget({ body: null });
  // @ts-expect-error: exact decimals never become JavaScript numbers
  const rounded: number = created.data.amount;
  // @ts-expect-error: an optional absent member is not null
  const absent: null = created.data.child;
  // @ts-expect-error: a tagged alternative retains its own required payload
  const invalid: models.WidgetPayload = {kind:'standard',vault:'vlt'};
}
