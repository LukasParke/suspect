/**
 * `@suspect/rules-sdk` — the rule-author surface.
 *
 * User rules import from this module (resolved via the worker's
 * `node_modules` shim). Everything here is pure TS against the wire
 * protocol in `protocol.ts`; heavy operations are host calls that arrive
 * as `native` frames (P2.5d).
 */
import type {
  FixIntent,
  TargetKind,
} from "./protocol.ts";
import {
  type AnyNode,
  type FactResponseNode,
  type FactRouteNode,
  type OperationNode,
  type ParameterNode,
  type PathItemNode,
  type ResponseNode,
  type SchemaNode,
  type Span,
  wrapOperation,
  wrapParameter,
  wrapPathItem,
  wrapResponse,
  wrapSchema,
  wrapFactResponse,
  wrapFactRoute,
} from "./nodes.ts";

export type Severity = "error" | "warning" | "info" | "hint";

/** Selector DSL entries. Strings are RFC-agnostic JSONPath understood by
 * the Rust host (`suspect-jsonpath`). */
export interface Selector {
  /** JSONPath evaluated by the host against the target document. */
  path: string;
  /** Optional worker-side wrapper applied to each selected node. */
  wrap?: "operation" | "pathItem" | "parameter" | "response" | "schema"
    | "factRoute" | "factResponse";
}

function sel(path: string, wrap?: Selector["wrap"]): Selector {
  return { path, wrap };
}

/** Selector DSL: `given: r.operation` and friends. */
export const r = {
  /** Every HTTP operation: `$.paths[*][*]` filtered to method keys. */
  operation: sel("$.paths[*][*]", "operation"),
  /** Every path item: `$.paths[*]`. */
  pathItem: sel("$.paths[*]", "pathItem"),
  /** Every parameter object anywhere operations declare them. */
  parameter: sel("$.paths[*][*].parameters[*]", "parameter"),
  /** Every response object: `$.paths[*][*].responses[*]`. */
  response: sel("$.paths[*][*].responses[*]", "response"),
  /** Every component schema: `$.components.schemas[*]`. */
  schema: sel("$.components.schemas[*]", "schema"),
  /** Fact-space route facts (code extraction). */
  factRoute: sel("$.operations[*]", "factRoute"),
  /** Fact-space response-signal facts. */
  factResponse: sel("$.responses[*]", "factResponse"),
  /** Raw JSONPath escape hatch, no wrapper. */
  custom: (path: string) => sel(path),
} as const;

/** A selected node passed to `check`, already wrapped per the selector. */
export type SelectedNode =
  | OperationNode
  | PathItemNode
  | ParameterNode
  | ResponseNode
  | SchemaNode
  | FactRouteNode
  | FactResponseNode;

/** Reporting context handed to rule bodies. */
export interface RuleContext {
  /** Report a finding. `at` accepts a wrapped node, a sub-field of one, or
   * an explicit pointer. */
  report(f: {
    message: string;
    at?:
      | { pointer: string; span?: Span }
      | { node: SelectedNode; field?: string };
    severity?: Severity;
    fix?: FixIntent;
  }): void;
  /** Read a file inside the workspace (jailed host-side). */
  readWorkspaceFile(rel: string): Promise<string | undefined>;
  /** Document uri under evaluation, when known. */
  docUri?: string;
}

export interface RuleMetaInput {
  /** Stable unique id, e.g. `"operation-summary"`. */
  id: string;
  description: string;
  /** Document kinds this rule evaluates. Default `["spec"]`. */
  targets?: TargetKind[];
  /** Default severity when the config does not override. */
  severity?: Severity;
  /** Docs link. */
  url?: string;
}

/** Point rule: `given` selects nodes, `check` runs per node. */
export interface PointRule {
  meta: RuleMetaInput;
  given: Selector;
  check(node: SelectedNode, ctx: RuleContext): void | Promise<void>;
}

/** Walk-rule visitor map: keys are node kinds, plus `onDocument` for
 * per-run setup. `ctx.state` is a per-run scratch object. */
export interface WalkVisitors {
  onDocument?(doc: { pointer: string }, ctx: WalkContext): void;
  Operation?(op: OperationNode, ctx: WalkContext): void;
  PathItem?(item: PathItemNode, ctx: WalkContext): void;
  Parameter?(p: ParameterNode, ctx: WalkContext): void;
  Response?(res: ResponseNode, ctx: WalkContext): void;
  Schema?(s: SchemaNode, ctx: WalkContext): void;
}

export interface WalkContext {
  report(f: {
    message: string;
    at?: { pointer: string; span?: Span };
    severity?: Severity;
    fix?: FixIntent;
  }): void;
  state: Record<string, unknown>;
  docUri?: string;
}

/** Walk rule: visitors traverse the whole document with shared state. */
export interface WalkRule {
  meta: RuleMetaInput;
  visitors: WalkVisitors;
}

export type RuleDefinition = PointRule | WalkRule;

export function isWalkRule(d: RuleDefinition): d is WalkRule {
  return "visitors" in d;
}

/** Identity helper giving rule authors full type inference. */
export function defineRule<T extends RuleDefinition>(rule: T): T {
  return rule;
}

/** Internal: wrap a raw selected node per the selector's `wrap` hint. */
export function wrapSelected(
  selector: Selector,
  pointer: string,
  value: unknown,
  span?: Span,
): SelectedNode | undefined {
  switch (selector.wrap) {
    case "operation":
      return wrapOperation(pointer, value, span);
    case "pathItem":
      return wrapPathItem(pointer, value, span);
    case "parameter":
      return wrapParameter(pointer, value, span);
    case "response":
      return wrapResponse(pointer, value, span);
    case "schema":
      return wrapSchema(pointer, value, span);
    case "factRoute":
      return wrapFactRoute(pointer, value, span);
    case "factResponse":
      return wrapFactResponse(pointer, value, span);
    default: {
      const base = {
        pointer,
        span,
        value,
        at(sub: string): unknown {
          const segs = sub.replace(/^\//, "").split("/").filter(Boolean);
          let cur: unknown = value;
          for (const seg of segs) {
            const key = seg.replace(/~1/g, "/").replace(/~0/g, "~");
            cur =
              cur && typeof cur === "object" && !Array.isArray(cur)
                ? (cur as Record<string, unknown>)[key]
                : undefined;
          }
          return cur;
        },
      };
      return base as SelectedNode;
    }
  }
}

export * from "./functions.ts";
export type {
  AnyNode,
  FactResponseNode,
  FactRouteNode,
  HandlerProvenance,
  OperationNode,
  ParameterNode,
  PathItemNode,
  ResponseNode,
  SchemaNode,
  Span,
} from "./nodes.ts";
export type { FixIntent, TargetKind } from "./protocol.ts";
