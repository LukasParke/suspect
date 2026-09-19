/** Strict RFC 3986 components. This module performs no acquisition or URL repair. */
export interface UriReference {
    readonly scheme: string | undefined;
    readonly authority: string | undefined;
    readonly path: string;
    readonly query: string | undefined;
    readonly fragment: string | undefined;
}
const pchar = /^(?:[A-Za-z0-9._~!$&'()*+,;=:@-]|%[0-9A-Fa-f]{2})*$/;
const hostCharacters = /^(?:[A-Za-z0-9._~!$&'()*+,;=-]|%[0-9A-Fa-f]{2})*$/;
const userCharacters = /^(?:[A-Za-z0-9._~!$&'()*+,;=:-]|%[0-9A-Fa-f]{2})*$/;

function authority(value: string): string {
    const at = value.lastIndexOf('@');
    const user = at < 0 ? '' : value.slice(0, at);
    if (!userCharacters.test(user)) throw new TypeError('invalid URI userinfo');
    const hostPort = value.slice(at + 1);
    let host: string, port: string;
    if (hostPort.startsWith('[')) {
        const end = hostPort.indexOf(']');
        if (end < 0) throw new TypeError('invalid URI IP literal');
        host = hostPort.slice(0, end + 1); port = hostPort.slice(end + 1);
        const ip = host.slice(1, -1);
        if (!/^v[0-9A-Fa-f]+\.[A-Za-z0-9._~!$&'()*+,;=:-]+$/i.test(ip)) {
            // Used only to validate IPv6 syntax. Keep the original literal;
            // native URL's compressed host/path spelling is never substituted.
            if (!ip.includes(':')) throw new TypeError('invalid URI IP literal');
            new URL(`http://${host}/`);
        }
    } else {
        const colon = hostPort.indexOf(':');
        host = colon < 0 ? hostPort : hostPort.slice(0, colon);
        port = colon < 0 ? '' : hostPort.slice(colon);
        if (!hostCharacters.test(host)) throw new TypeError('invalid URI host');
    }
    if (port !== '' && !/^:[0-9]*$/.test(port)) throw new TypeError('invalid URI port');
    return (at < 0 ? '' : `${user}@`) + host.toLowerCase() + port;
}

/** Parse components without converting percent-encoded dots/slashes into syntax. */
export function parseUriReference(value: string): UriReference {
    if (typeof value !== 'string') throw new TypeError('URI reference must be a string');
    // RFC 3986 references are ASCII; non-ASCII data must be percent-encoded.
    // This also excludes final line terminators before any regexp $ anchor.
    if (/[^\x21-\x7e]/.test(value)) throw new TypeError('invalid RFC 3986 characters');
    const match = /^(?:([A-Za-z][A-Za-z0-9+.-]*):)?(?:\/\/([^/?#]*))?([^?#]*)(?:\?([^#]*))?(?:#([^#]*))?$/.exec(value);
    if (match === null) throw new TypeError('invalid RFC 3986 URI reference');
    const [, scheme, host, path = '', query, fragment] = match;
    if (!path.split('/').every(segment => pchar.test(segment)) ||
        query !== undefined && !query.split(/[/?]/).every(segment => pchar.test(segment)) ||
        fragment !== undefined && !fragment.split(/[/?]/).every(segment => pchar.test(segment))) throw new TypeError('invalid RFC 3986 characters or percent escapes');
    if (scheme === undefined && host === undefined && path.split('/', 1)[0]!.includes(':')) throw new TypeError('a relative first path segment cannot contain a colon');
    return { scheme: scheme?.toLowerCase(), authority: host === undefined ? undefined : authority(host), path, query, fragment };
}

function removeDotSegments(path: string): string {
    let input = path, output = '';
    while (input !== '') {
        if (input.startsWith('../')) input = input.slice(3);
        else if (input.startsWith('./')) input = input.slice(2);
        else if (input.startsWith('/./')) input = input.slice(2);
        else if (input === '/.') input = '/';
        else if (input.startsWith('/../') || input === '/..') {
            input = input === '/..' ? '/' : input.slice(3);
            output = output.slice(0, Math.max(output.lastIndexOf('/'), 0));
        } else if (input === '.' || input === '..') input = '';
        else {
            const next = input.indexOf('/', input.startsWith('/') ? 1 : 0);
            const end = next < 0 ? input.length : next;
            output += input.slice(0, end); input = input.slice(end);
        }
    }
    return output;
}

function document(reference: UriReference): string {
    if (reference.scheme === undefined || reference.authority === undefined && reference.path.startsWith('//')) throw new TypeError('URI resolution is not representable as an absolute identifier');
    return `${reference.scheme}:${reference.authority === undefined ? '' : `//${reference.authority}`}${reference.path}${reference.query === undefined ? '' : `?${reference.query}`}`;
}

/** RFC 3986 §5.2 document resolution, preserving encoded octets and query spelling. */
export function resolveUriDocument(base: string, value: string): string {
    const parent = parseUriReference(base), reference = parseUriReference(value);
    if (parent.scheme === undefined || parent.fragment !== undefined) throw new TypeError('URI base must be absolute and fragment-free');
    if (reference.scheme !== undefined) return document({ ...reference, path: removeDotSegments(reference.path) });
    if (reference.authority !== undefined) return document({ ...reference, scheme: parent.scheme, path: removeDotSegments(reference.path) });
    if (reference.path === '') return document({ ...parent, query: reference.query ?? parent.query });
    const prefix = parent.authority !== undefined && parent.path === '' ? '/' : parent.path.slice(0, parent.path.lastIndexOf('/') + 1);
    return document({ ...reference, scheme: parent.scheme, authority: parent.authority, path: removeDotSegments(reference.path.startsWith('/') ? reference.path : prefix + reference.path) });
}

/** Encode a decoded pointer/name once as a URI fragment, without its # delimiter. */
export function encodeUriFragment(value: string): string {
    let result = '';
    for (const character of value) result += /^[A-Za-z0-9._~!$&'()*+,;=:@/?-]$/.test(character) ? character : encodeURIComponent(character);
    return result;
}

/** Checked resource alias identity; an empty fragment names its primary resource. */
export function uriKey(value: string): string {
    const parsed = parseUriReference(value);
    const fragment = decodeURIComponent(parsed.fragment ?? '');
    return document(parsed) + (fragment === '' ? '' : `#${encodeUriFragment(fragment)}`);
}
