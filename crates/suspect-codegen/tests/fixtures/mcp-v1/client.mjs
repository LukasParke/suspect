// Portable acceptance harness for the generated TypeScript MCP server.
//
// Drives the built stdio server with the matching official MCP client and
// writes one JSON line per observation to stdout for the Rust test to assert.
// Protocol era is pinned explicitly to the SDK default negotiation mode
// ('legacy', the 2025-11-25 connect sequence) so the exchange under test is
// one fixed era rather than whatever a future client default becomes.
//
// Everything this process talks to is loopback: a python3 HTTP fixture.
import { Client } from '@modelcontextprotocol/client';
import { StdioClientTransport } from '@modelcontextprotocol/client/stdio';

const [entry, baseUrl, apiKey] = process.argv.slice(2);
const stderrChunks = [];

function emit(record) {
    process.stdout.write(`${JSON.stringify(record)}\n`);
}

const transport = new StdioClientTransport({
    command: process.execPath,
    args: [entry],
    env: { WIDGET_SERVER_URL: baseUrl, WIDGET_API_KEY: apiKey, PATH: process.env.PATH ?? '' },
    stderr: 'pipe',
});
const client = new Client(
    { name: 'suspect-mcp-acceptance', version: '0.0.0' },
    { versionNegotiation: { mode: 'legacy' } },
);

async function call(label, name, args, options) {
    try {
        const result = await client.callTool({ name, arguments: args }, options);
        emit({ case: label, outcome: 'result', isError: result.isError === true, content: result.content, structuredContent: result.structuredContent });
    } catch (error) {
        emit({ case: label, outcome: 'threw', name: error?.name ?? null, message: String(error?.message ?? error) });
    }
}

try {
    await client.connect(transport);
    transport.stderr?.on('data', (chunk) => stderrChunks.push(chunk.toString('utf8')));

    const first = await client.listTools();
    const again = await client.listTools({}, { cache: 'no-store' });
    emit({
        case: 'tools',
        names: first.tools.map((tool) => tool.name),
        repeated: again.tools.map((tool) => tool.name),
        tools: first.tools,
    });

    // Exact tokens in, exact tokens out. false, the empty string and an
    // explicit null are all distinct from omission.
    await call('read', 'read_widget', {
        id: 'w-1',
        verbose: false,
        limit: '9007199254740993',
        label: '',
        trace: 't-9',
    });
    // Omitted optional properties must stay omitted on the wire.
    await call('read_minimal', 'read_widget', { id: 'w-1' });
    // Schema validation failure: a numeric property is not a free-form string.
    await call('bad_input', 'read_widget', { id: 'w-1', limit: 'not-a-number' });
    // Undeclared properties are refused by the projected schema.
    await call('bad_extra', 'read_widget', { id: 'w-1', invented: true });
    // A token the projected schema admits but the source schema does not: the
    // generated codec enforces integrality, and the rejection must read as the
    // client's argument to fix rather than a condition at the API.
    await call('bad_codec', 'read_widget', { id: 'w-1', limit: '1.5' });
    // A declared upstream failure surfaces as an isError tool result.
    await call('upstream_failure', 'read_widget', { id: 'missing' });
    // A finite JSON body with a large integer, false, an explicit null and an
    // empty array, projected through the generated codec.
    await call('create', 'create_widget', {
        widget: {
            name: 'alpha',
            amount: '9007199254740993',
            active: false,
            note: null,
            grade: 'high',
            tags: [],
        },
    });
    await call('create_denied', 'create_widget', {
        widget: { name: 'deny', amount: '1' },
    });
    // A response with no declared content still returns a tool result.
    await call('purge', 'purge_widgets', { scope: 'staging' });
    // Client-side cancellation reaches the running upstream call.
    await call('cancel', 'read_widget', { id: 'slow' }, { signal: AbortSignal.timeout(1500) });

    await client.close();
} catch (error) {
    emit({ case: 'fatal', message: String(error?.message ?? error) });
} finally {
    try {
        await transport.close();
    } catch {
        // The child may already be gone; the harness still reports stderr.
    }
    await new Promise((resolve) => setTimeout(resolve, 250));
    emit({ case: 'stderr', text: stderrChunks.join('') });
}
