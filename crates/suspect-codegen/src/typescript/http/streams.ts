import { JsonNumber, stringifyJson, type WireJsonValue } from '../json.js';
import type { OperationDescriptor, RuntimeLimits, StreamPlan } from './types.js';
import { BodyReader, Control, RuntimeError, decodeJson, encodeJson, encodeUtf8, limitBytes, objectData, requestFailure, responseFailure, validatedWire } from './common.js';

interface Line { readonly text: string; readonly bytes: number }
async function* lines(reader: BodyReader, sse: boolean, maximum: number): AsyncGenerator<Line> {
    let pieces: Uint8Array[] = [], length = 0, skipLF = false;
    const take = (delimiterBytes: number): Line => {
        const bytes = new Uint8Array(length);
        let at = 0;
        for (const piece of pieces) { bytes.set(piece, at); at += piece.byteLength; }
        const line = { text: new TextDecoder('utf-8', { fatal: !sse, ignoreBOM: true }).decode(bytes), bytes: length + delimiterBytes };
        pieces = []; length = 0;
        return line;
    };
    const append = (bytes: Uint8Array) => {
        limitBytes(length + bytes.byteLength, maximum, reader.control.operation, reader.source, 'stream line buffer');
        if (bytes.byteLength !== 0) { pieces.push(bytes.slice()); length += bytes.byteLength; }
    };
    for (;;) {
        const chunk = await reader.read();
        if (chunk === undefined) break;
        let start = 0;
        for (let at = 0; at < chunk.byteLength; at++) {
            const byte = chunk[at]!;
            if (skipLF) {
                skipLF = false;
                if (byte === 10) { start = at + 1; continue; }
            }
            if (byte !== 10 && !(sse && byte === 13)) continue;
            append(chunk.subarray(start, at));
            const line = take(1);
            if (!sse && line.text.endsWith('\r')) yield { ...line, text: line.text.slice(0, -1) };
            else yield line;
            skipLF = sse && byte === 13;
            start = at + 1;
        }
        append(chunk.subarray(start));
    }
    // JSON Lines permits an unterminated final record. SSE requires an empty
    // line to dispatch an event and discards an incomplete block at EOF.
    if (!sse && length !== 0) yield take(0);
}

/** A single-consumer, pull-driven native iterator with eager abort/return cleanup. */
export function streamItems(response: Response, stream: StreamPlan, descriptor: OperationDescriptor<unknown>, limits: RuntimeLimits, control: Control): AsyncIterableIterator<unknown> {
    const source = stream.source.source;
    const maximum = Math.min(limits.streamItem, stream.max_item_bytes);
    const reader = new BodyReader(response, control, source, limits.response, limits.streamBuffer);
    let stopped = false;
    const stop = (reason?: unknown) => {
        if (stopped) return;
        stopped = true;
        reader.cancel(reason);
        control.signal.removeEventListener('abort', aborted);
        control.dispose();
    };
    const aborted = () => stop(control.signal.reason);
    control.signal.addEventListener('abort', aborted, { once: true });
    if (control.signal.aborted) aborted();
    async function* decode(): AsyncGenerator<unknown> {
        let count = 0, buffered = 0, first = true;
        let data: string[] = [];
        let fields: { [key: string]: WireJsonValue } = Object.create(null);
        const item = (text: string): unknown => {
            if (++count > limits.streamItems) throw new RuntimeError('resource-limit', 'stream item count exceeds its generated ceiling', descriptor.source, source);
            return decodeJson(text, descriptor, stream.item_codec, source);
        };
        try {
            control.check(source);
            for await (const line of lines(reader, stream.framing === 'server-sent-events', maximum)) {
                control.check(source);
                if (stream.framing === 'json-lines') {
                    limitBytes(line.bytes, maximum, descriptor.source, source, 'JSON Lines item');
                    if (line.text.trim() === '') throw new TypeError('blank JSON Lines records are not JSON values');
                    yield item(line.text);
                    continue;
                }
                let text = line.text;
                if (first && text.startsWith('\uFEFF')) text = text.slice(1);
                first = false;
                buffered += line.bytes;
                limitBytes(buffered, maximum, descriptor.source, source, 'SSE event buffer');
                if (text === '') {
                    if (data.length !== 0) {
                        fields.data = data.join('\n');
                        yield item(stringifyJson(fields));
                    }
                    data = []; fields = Object.create(null) as { [key: string]: WireJsonValue }; buffered = 0;
                    continue;
                }
                if (text.startsWith(':')) continue;
                const colon = text.indexOf(':');
                const name = colon < 0 ? text : text.slice(0, colon);
                let value = colon < 0 ? '' : text.slice(colon + 1);
                if (value.startsWith(' ')) value = value.slice(1);
                if (name === 'data') data.push(value);
                else if (name === 'event') fields.event = value;
                else if (name === 'id' && !value.includes('\0')) fields.id = value;
                else if (name === 'retry' && /^[0-9]+$/.test(value)) fields.retry = JsonNumber.parse(value.replace(/^0+(?=\d)/, ''));
                // Unknown fields, comments, invalid id/retry values and no-data
                // blocks are ignored by HTML framing. Data remains a string.
            }
        } catch (error) { throw responseFailure(error, descriptor.source, source); }
        finally { stop(); }
    }
    const iterator = decode();
    return {
        [Symbol.asyncIterator]() { return this; },
        next() { return iterator.next(); },
        async return() {
            // Cancel now, including before the first next() or while a next()
            // is blocked on a transport that ignores AbortSignal.
            if (!stopped) control.controller.abort(new Error('stream consumer returned early'));
            stop();
            return iterator.return(undefined);
        },
        async throw(error?: unknown) {
            if (!stopped) control.controller.abort(error);
            stop(error);
            return iterator.throw(error);
        },
    };
}

/** Finite request item sequences, bounded before sending any HTTP bytes. */
export async function encodeItems(value: unknown, stream: StreamPlan, descriptor: OperationDescriptor<unknown>, limits: RuntimeLimits, control: Control): Promise<Uint8Array> {
    const source = stream.source.source;
    const maximum = Math.min(limits.streamItem, stream.max_item_bytes);
    let iterator: AsyncIterator<unknown> | Iterator<unknown> | undefined;
    try {
        if (value === null || typeof value !== 'object') throw new TypeError('sequential request body requires Iterable or AsyncIterable items');
        if (Symbol.asyncIterator in value) iterator = (value as AsyncIterable<unknown>)[Symbol.asyncIterator]();
        else if (Symbol.iterator in value) iterator = (value as Iterable<unknown>)[Symbol.iterator]();
        else throw new TypeError('sequential request body requires Iterable or AsyncIterable items');
        const chunks: Uint8Array[] = [];
        let length = 0, count = 0;
        for (;;) {
            control.check(source);
            const next = await control.race(Promise.resolve(iterator.next()));
            if (next.done) break;
            if (++count > limits.streamItems) throw new RuntimeError('resource-limit', 'request stream item count exceeds its ceiling', descriptor.source, source);
            let text: string;
            if (stream.framing === 'json-lines') text = encodeJson(next.value, descriptor, stream.item_codec, source, maximum) + '\n';
            else {
                const wire = validatedWire(next.value, descriptor, stream.item_codec, maximum);
                const envelope = objectData(wire);
                if (typeof envelope.data !== 'string' || Object.keys(envelope).some(key => !['data', 'event', 'id', 'retry'].includes(key))) throw new TypeError('SSE request items require data and only standard envelope fields');
                const fields: string[] = [];
                for (const name of ['event', 'id'] as const) {
                    const field = envelope[name];
                    if (field === undefined) continue;
                    if (typeof field !== 'string' || /[\r\n]/.test(field) || name === 'id' && field.includes('\0')) throw new TypeError('SSE id/event cannot inject framing or ignored values');
                    fields.push(`${name}: ${field}\n`);
                }
                if (envelope.retry !== undefined) {
                    if (!JsonNumber.is(envelope.retry)) throw new TypeError('SSE retry must be an integer');
                    const retry = JsonNumber.prototype.toBigInt.call(envelope.retry, Math.min(maximum, 4096));
                    if (retry === undefined || retry < 0n) throw new TypeError('SSE retry must be a finite nonnegative integer');
                    fields.push(`retry: ${retry}\n`);
                }
                if (envelope.data.includes('\r')) throw new TypeError('SSE data containing CR cannot round-trip HTML line normalization');
                for (const line of envelope.data.split('\n')) fields.push(`data: ${line}\n`);
                text = fields.join('') + '\n';
            }
            const chunk = encodeUtf8(text, Math.min(maximum, limits.request - length), descriptor.source, source);
            length += chunk.byteLength; chunks.push(chunk);
        }
        const result = new Uint8Array(length);
        let offset = 0;
        for (const chunk of chunks) { result.set(chunk, offset); offset += chunk.byteLength; }
        return result;
    } catch (error) {
        if (iterator?.return !== undefined) void Promise.resolve(iterator.return()).catch(() => {});
        throw requestFailure(error, descriptor.source, source, 'sequential request body');
    }
}
