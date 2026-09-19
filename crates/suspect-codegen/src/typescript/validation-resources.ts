import type { ValidationProgram, ValidationSource } from './validation.js';
import { encodeUriFragment, parseUriReference, resolveUriDocument, uriKey } from './uri.js';

/** Checked physical resource identity and logical, metadata-only URI names. */
export interface ValidationResource {
    readonly source: ValidationSource;
    readonly kind: 'document' | 'openApiDocument' | 'schema';
    readonly canonicalUri: string;
    readonly baseUri: string;
    readonly aliases: readonly string[];
    readonly declarationSource: ValidationSource | null;
    readonly dynamicAnchors: readonly (readonly [string, ValidationSource, number])[];
}
/** V3-only indexed scope; bindings never authorize schema lookup or acquisition. */
export interface ValidationResourceContext {
    readonly resources: readonly ValidationResource[];
    readonly nodeScopes: readonly (readonly [number, ValidationSource, string])[];
}
const key = (source: ValidationSource) => JSON.stringify([source.document, source.pointer]);
const child = (source: ValidationSource, name: string): ValidationSource => ({ document: source.document, pointer: `${source.pointer}/${name.replace(/~/g, '~0').replace(/\//g, '~1')}` });
const contains = (parent: ValidationSource, source: ValidationSource) => parent.document === source.document &&
    (parent.pointer === source.pointer || source.pointer.startsWith(`${parent.pointer}/`));
const anchorName = (name: string) => typeof name === 'string' && /^[A-Za-z_]/.test(name) && !/[^A-Za-z0-9_.-]/.test(name);

function checkSource(source: ValidationSource): void {
    const uri = parseUriReference(source.document);
    if (uri.scheme === undefined || uri.fragment !== undefined || typeof source.pointer !== 'string' ||
        source.pointer !== '' && !source.pointer.startsWith('/') || /~(?![01])/.test(source.pointer)) throw new TypeError('invalid physical source identity');
    encodeUriFragment(source.pointer); // Reject unpaired UTF-16 without rewriting the pointer.
}

/** Portable v3 invariants, without rediscovering any source-schema semantics. */
export function checkResourceContext(program: ValidationProgram): ValidationResourceContext {
    const context = program.resourceContext;
    if (context === undefined || context === null || !Array.isArray(context.resources) || !Array.isArray(context.nodeScopes) || context.nodeScopes.length !== program.nodes.length) throw new TypeError('v3 resource context must align one-to-one with nodes');
    const identities = new Set<string>(), aliases = new Map<string, number>();
    for (const [index, resource] of context.resources.entries()) {
        checkSource(resource.source);
        if (identities.has(key(resource.source))) throw new TypeError('duplicate physical resource identity');
        identities.add(key(resource.source));
        if (!['document', 'openApiDocument', 'schema'].includes(resource.kind)) throw new TypeError('unknown resource kind');
        const canonical = uriKey(resource.canonicalUri), base = uriKey(resource.baseUri);
        if (parseUriReference(resource.baseUri).fragment !== undefined || resolveUriDocument(resource.baseUri, resource.canonicalUri) !== resource.baseUri || resource.kind !== 'openApiDocument' && canonical !== base) throw new TypeError('inconsistent resource canonical/base URI');
        if (resource.declarationSource !== null) {
            checkSource(resource.declarationSource);
            if (resource.kind === 'document' || key(resource.declarationSource) !== key(child(resource.source, resource.kind === 'schema' ? '$id' : '$self'))) throw new TypeError('identifier declaration is not at its resource boundary');
        } else if (resource.source.pointer !== '' || canonical !== uriKey(resource.source.document)) throw new TypeError('undeclared resource lost its physical retrieval identity');
        if (!Array.isArray(resource.aliases) || new Set(resource.aliases).size !== resource.aliases.length) throw new TypeError('invalid or duplicate resource aliases');
        const names = new Set<string>();
        for (const alias of resource.aliases) {
            const name = uriKey(alias), previous = aliases.get(name);
            if (previous !== undefined && previous !== index) throw new TypeError('resource alias identifies multiple physical resources');
            aliases.set(name, index); names.add(name);
        }
        if (!names.has(canonical) || !names.has(base)) throw new TypeError('aliases must include canonical and base URI');
    }
    const used = new Set<number>();
    for (const [index, scope] of context.nodeScopes.entries()) {
        if (!Array.isArray(scope) || scope.length !== 3 || !Number.isSafeInteger(scope[0]) || scope[0] < 0) throw new TypeError('invalid indexed resource scope');
        const [resourceIndex, schemaRoot, address] = scope, node = program.nodes[index], resource = context.resources[resourceIndex];
        if (node === undefined || resource === undefined) throw new TypeError('v3 node/resource alignment is incomplete');
        checkSource(node.source); checkSource(schemaRoot);
        if (!contains(resource.source, schemaRoot) || !contains(schemaRoot, node.source) || resource.kind === 'schema' && key(schemaRoot) !== key(resource.source)) throw new TypeError('indexed scope does not contain its physical node');
        const suffix = node.source.pointer.slice(resource.source.pointer.length);
        if (address !== resource.baseUri + (suffix === '' ? '' : `#${encodeUriFragment(suffix)}`)) throw new TypeError('node canonical address is inconsistent');
        used.add(resourceIndex);
    }
    if (used.size !== context.resources.length) throw new TypeError('resource registry contains an unreferenced resource');
    for (const [resourceIndex, resource] of context.resources.entries()) {
        if (!Array.isArray(resource.dynamicAnchors)) throw new TypeError('missing dynamic bindings');
        const names = new Set<string>();
        for (const binding of resource.dynamicAnchors) {
            if (!Array.isArray(binding) || binding.length !== 3) throw new TypeError('invalid dynamic binding tuple');
            const [name, source, target] = binding;
            checkSource(source);
            if (!anchorName(name) || names.has(name) || !Number.isSafeInteger(target) || target < 0 || program.nodes[target] === undefined || context.nodeScopes[target]![0] !== resourceIndex || key(source) !== key(child(program.nodes[target]!.source, '$dynamicAnchor'))) throw new TypeError('invalid dynamic binding identity, target or resource');
            names.add(name);
        }
    }
    for (const node of program.nodes) for (const check of node!.checks) {
        if (check.op !== 'dynamicRef') continue;
        const { target, initialResource, anchor } = check;
        if (!Number.isSafeInteger(target) || target < 0 || program.nodes[target] === undefined || !Number.isSafeInteger(initialResource) || initialResource < 0 || context.nodeScopes[target]![0] !== initialResource) throw new TypeError('dynamic initial target/resource mismatch');
        if (anchor !== null && (!anchorName(anchor) || !context.resources[initialResource]!.dynamicAnchors.some(([name, , index]: ValidationResource['dynamicAnchors'][number]) => name === anchor && index === target))) throw new TypeError('dynamic name does not identify its initial indexed anchor');
    }
    return context;
}

/** Per-call ordered, first-distinct entered resources and exact context identity. */
export class ResourceScope {
    private readonly stack: number[] = [];
    private readonly entered = new Set<number>();
    private readonly contexts = new Map<number, Map<number, number>>();
    private next = 0;
    private current = 0;
    constructor(private readonly metadata: ValidationResourceContext, private readonly spend: (source: ValidationSource, path: string) => void) {}
    get identity(): number { return this.current; }
    /** Exact ordered resource indices, shared with the source-bound codec trace. */
    get signature(): string { return this.stack.join(','); }
    enter(node: number, source: ValidationSource, path: string): number | undefined {
        const resource = this.metadata.nodeScopes[node]![0];
        if (this.entered.has(resource)) return undefined;
        this.spend(source, path);
        const previous = this.current;
        let children = this.contexts.get(previous);
        if (children === undefined) { children = new Map<number, number>(); this.contexts.set(previous, children); }
        let identity = children.get(resource);
        if (identity === undefined) {
            identity = ++this.next;
            if (!Number.isSafeInteger(identity)) throw new RangeError('resource context identity capacity exhausted');
            children.set(resource, identity);
        }
        this.current = identity; this.entered.add(resource); this.stack.push(resource);
        return previous;
    }
    leave(previous: number | undefined): void {
        if (previous === undefined) return;
        this.entered.delete(this.stack.pop()!); this.current = previous;
    }
    target(fallback: number, anchor: string | null, source: ValidationSource, path: string): number {
        if (anchor !== null) for (const resource of this.stack) {
            this.spend(source, path);
            for (const [name, , target] of this.metadata.resources[resource]!.dynamicAnchors) {
                this.spend(source, path);
                if (name === anchor) return target;
            }
        }
        return fallback;
    }
}
