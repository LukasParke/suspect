/**
 * Typed node wrappers and the rule-author surface model.
 *
 * Nodes wrap raw JSON values selected by a rule's `given` selector. Every
 * node carries its RFC 6901 pointer and (when the host resolved it) the byte
 * span into the source document — the same lossless-CST provenance native
 * rules use. Fact nodes additionally carry handler provenance so TS rules
 * can report findings inside the code that implements the API.
 */
import type { FixIntent, TargetKind } from "./protocol.ts";

/** Byte range into the source document (lossless CST provenance). */
export interface Span {
  start: number;
  end: number;
}

/** Handler provenance carried by fact-space nodes. */
export interface HandlerProvenance {
  file: string;
  start: number;
  end: number;
  line: number;
  column: number;
  confidence: "exact" | "inferred" | "partial" | "unknown";
  adapterId: string;
}

export interface NodeBase {
  /** RFC 6901 pointer to this node from the document root. */
  readonly pointer: string;
  /** Byte span in the source document, when the host resolved it. */
  readonly span?: Span;
  /** The raw JSON value at this node. */
  readonly value: unknown;
  /** Source text of this node (host-provided on demand; usually absent). */
  readonly source?: string;
  /** Read the value at a sub-pointer relative to this node. */
  at(sub: string): unknown;
}

export type HttpMethod =
  | "get"
  | "put"
  | "post"
  | "delete"
  | "options"
  | "head"
  | "patch"
  | "trace";

const METHODS: readonly string[] = [
  "get",
  "put",
  "post",
  "delete",
  "options",
  "head",
  "patch",
  "trace",
];

/** True when a path-item key names an HTTP method (host-side too). */
export function isMethodKey(key: string): key is HttpMethod {
  return METHODS.includes(key);
}

function makeBase(pointer: string, value: unknown, span?: Span): NodeBase {
  return {
    pointer,
    span,
    value,
    at(sub: string): unknown {
      const segs = sub.replace(/^\//, "").split("/").filter((s) => s.length);
      let cur: unknown = value;
      for (const seg of segs) {
        const unescaped = seg.replace(/~1/g, "/").replace(/~0/g, "~");
        if (cur && typeof cur === "object" && !Array.isArray(cur)) {
          cur = (cur as Record<string, unknown>)[unescaped];
        } else if (Array.isArray(cur)) {
          cur = cur[Number(unescaped)];
        } else {
          return undefined;
        }
      }
      return cur;
    },
  };
}

/** `$.paths[*][*]` node that is an HTTP operation. */
export interface OperationNode extends NodeBase {
  readonly kind: "operation";
  readonly method: HttpMethod;
  /** The path template this operation is mounted under, e.g. `/pets/{id}`. */
  readonly path: string;
  readonly operationId?: string;
  readonly summary?: string;
  readonly description?: string;
  readonly tags?: string[];
  readonly deprecated?: boolean;
  readonly responses: Record<string, unknown>;
}

/** A path item: `/pets/{id}` and everything mounted under it. */
export interface PathItemNode extends NodeBase {
  readonly kind: "pathItem";
  readonly path: string;
}

/** A parameter object (inline or resolved). */
export interface ParameterNode extends NodeBase {
  readonly kind: "parameter";
  readonly name?: string;
  readonly in?: string;
  readonly required?: boolean;
  readonly description?: string;
}

/** A response object under an operation. */
export interface ResponseNode extends NodeBase {
  readonly kind: "response";
  readonly status: string;
  readonly description?: string;
}

/** A component schema. */
export interface SchemaNode extends NodeBase {
  readonly kind: "schema";
  readonly name: string;
  readonly type?: string;
  readonly description?: string;
}

/** Fact-space route node (from code extraction). */
export interface FactRouteNode extends NodeBase {
  readonly kind: "factRoute";
  readonly method?: string;
  readonly path?: string;
  readonly handler?: string;
  readonly confidence?: string;
  readonly adapterId?: string;
  readonly provenance?: HandlerProvenance;
}

/** Fact-space response-signal node (what a handler constructs). */
export interface FactResponseNode extends NodeBase {
  readonly kind: "factResponse";
  readonly status?: string;
  readonly handler?: string;
  readonly provenance?: HandlerProvenance;
}

export type AnyNode =
  | OperationNode
  | PathItemNode
  | ParameterNode
  | ResponseNode
  | SchemaNode
  | FactRouteNode
  | FactResponseNode;

/** Build an operation wrapper; `undefined` when the value is not one. */
export function wrapOperation(
  pointer: string,
  value: unknown,
  span?: Span,
): OperationNode | undefined {
  const segs = pointer.split("/").filter((s) => s.length > 0);
  // /paths/~1pets/get → ["paths", "/pets", "get"]
  const method = segs[segs.length - 1];
  const path = segs[segs.length - 2]?.replace(/~1/g, "/").replace(/~0/g, "~");
  if (!method || !path || !isMethodKey(method)) return undefined;
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    return undefined;
  }
  const v = value as Record<string, unknown>;
  const base = makeBase(pointer, value, span);
  return {
    ...base,
    kind: "operation",
    method,
    path,
    operationId: typeof v.operationId === "string" ? v.operationId : undefined,
    summary: typeof v.summary === "string" ? v.summary : undefined,
    description: typeof v.description === "string" ? v.description : undefined,
    tags: Array.isArray(v.tags) ? (v.tags as string[]) : undefined,
    deprecated: v.deprecated === true,
    responses: isRecord(v.responses) ? v.responses : {},
  };
}

/** Build a path-item wrapper. */
export function wrapPathItem(
  pointer: string,
  value: unknown,
  span?: Span,
): PathItemNode | undefined {
  const segs = pointer.split("/").filter((s) => s.length > 0);
  const path = segs[segs.length - 1]?.replace(/~1/g, "/").replace(/~0/g, "~");
  if (!path) return undefined;
  return { ...makeBase(pointer, value, span), kind: "pathItem", path };
}

/** Build a parameter wrapper. */
export function wrapParameter(
  pointer: string,
  value: unknown,
  span?: Span,
): ParameterNode | undefined {
  if (!isRecord(value)) return undefined;
  return {
    ...makeBase(pointer, value, span),
    kind: "parameter",
    name: typeof value.name === "string" ? value.name : undefined,
    in: typeof value.in === "string" ? value.in : undefined,
    required: value.required === true,
    description:
      typeof value.description === "string" ? value.description : undefined,
  };
}

/** Build a response wrapper; pointer must end at the status key. */
export function wrapResponse(
  pointer: string,
  value: unknown,
  span?: Span,
): ResponseNode | undefined {
  const segs = pointer.split("/").filter((s) => s.length > 0);
  const status = segs[segs.length - 1];
  if (!status || !isRecord(value)) return undefined;
  return {
    ...makeBase(pointer, value, span),
    kind: "response",
    status,
    description:
      typeof value.description === "string" ? value.description : undefined,
  };
}

/** Build a component-schema wrapper. */
export function wrapSchema(
  pointer: string,
  value: unknown,
  span?: Span,
): SchemaNode | undefined {
  const segs = pointer.split("/").filter((s) => s.length > 0);
  const name = segs[segs.length - 1];
  if (!name || !isRecord(value)) return undefined;
  return {
    ...makeBase(pointer, value, span),
    kind: "schema",
    name,
    type: typeof value.type === "string" ? value.type : undefined,
    description:
      typeof value.description === "string" ? value.description : undefined,
  };
}

/** Build a fact-route wrapper (fact-space). */
export function wrapFactRoute(
  pointer: string,
  value: unknown,
  span?: Span,
): FactRouteNode | undefined {
  if (!isRecord(value)) return undefined;
  return {
    ...makeBase(pointer, value, span),
    kind: "factRoute",
    method: typeof value.method === "string" ? value.method : undefined,
    path: typeof value.path === "string" ? value.path : undefined,
    handler: typeof value.handler === "string" ? value.handler : undefined,
    confidence:
      typeof value.confidence === "string" ? value.confidence : undefined,
    adapterId: typeof value.adapter_id === "string" ? value.adapter_id : undefined,
    provenance: wrapProvenance(value.provenance),
  };
}

/** Build a fact-response wrapper (fact-space). */
export function wrapFactResponse(
  pointer: string,
  value: unknown,
  span?: Span,
): FactResponseNode | undefined {
  if (!isRecord(value)) return undefined;
  return {
    ...makeBase(pointer, value, span),
    kind: "factResponse",
    status: typeof value.status === "string" ? value.status : undefined,
    handler: typeof value.handler === "string" ? value.handler : undefined,
    provenance: wrapProvenance(value.provenance),
  };
}

function wrapProvenance(v: unknown): HandlerProvenance | undefined {
  if (!isRecord(v)) return undefined;
  if (typeof v.file !== "string") return undefined;
  return {
    file: v.file,
    start: typeof v.start === "number" ? v.start : 0,
    end: typeof v.end === "number" ? v.end : 0,
    line: typeof v.line === "number" ? v.line : 1,
    column: typeof v.column === "number" ? v.column : 1,
    confidence:
      typeof v.confidence === "string"
        ? (v.confidence as HandlerProvenance["confidence"])
        : "unknown",
    adapterId: typeof v.adapter_id === "string" ? v.adapter_id : "unknown",
  };
}

function isRecord(v: unknown): v is Record<string, unknown> {
  return v !== null && typeof v === "object" && !Array.isArray(v);
}

export type { FixIntent, TargetKind };
