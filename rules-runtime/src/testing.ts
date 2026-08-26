/**
 * `testRule` — in-process rule harness for `bun test`.
 *
 * Runs a rule against an inline document the same way the worker would:
 * selects nodes per the rule's `given` (mini-evaluator covering the SDK's
 * documented selector patterns; raw custom selectors require explicit
 * pointers), wraps them, invokes check/visitors, and hands findings to the
 * assertion callback.
 */
import {
  type RuleDefinition,
  type SelectedNode,
  type Selector,
  isWalkRule,
  wrapSelected,
} from "./sdk.ts";
import { isMethodKey } from "./nodes.ts";

export interface CapturedFinding {
  pointer: string;
  message: string;
  severity?: string;
  fix?: unknown;
}

interface RawNode {
  pointer: string;
  value: unknown;
}

/** Minimal selector evaluator for the SDK's documented patterns. */
function evaluateSelector(selector: Selector, doc: unknown): RawNode[] {
  const out: RawNode[] = [];
  const root = doc as Record<string, unknown>;
  switch (selector.path) {
    case "$.paths[*][*]": {
      const paths = root.paths as Record<string, unknown> | undefined;
      if (!paths) return out;
      for (const [p, item] of Object.entries(paths)) {
        if (!item || typeof item !== "object") continue;
        for (const [key, op] of Object.entries(item as Record<string, unknown>)) {
          out.push({ pointer: `/paths/${enc(p)}/${key}`, value: op });
        }
      }
      return out;
    }
    case "$.paths[*]": {
      const paths = root.paths as Record<string, unknown> | undefined;
      if (!paths) return out;
      for (const [p, item] of Object.entries(paths)) {
        out.push({ pointer: `/paths/${enc(p)}`, value: item });
      }
      return out;
    }
    case "$.paths[*][*].parameters[*]": {
      const paths = root.paths as Record<string, unknown> | undefined;
      if (!paths) return out;
      for (const [p, item] of Object.entries(paths)) {
        if (!item || typeof item !== "object") continue;
        for (const [key, op] of Object.entries(item as Record<string, unknown>)) {
          const params = (op as Record<string, unknown>)?.parameters;
          if (!Array.isArray(params)) continue;
          params.forEach((param, i) => {
            out.push({ pointer: `/paths/${enc(p)}/${key}/parameters/${i}`, value: param });
          });
        }
      }
      return out;
    }
    case "$.paths[*][*].responses[*]": {
      const paths = root.paths as Record<string, unknown> | undefined;
      if (!paths) return out;
      for (const [p, item] of Object.entries(paths)) {
        if (!item || typeof item !== "object") continue;
        for (const [key, op] of Object.entries(item as Record<string, unknown>)) {
          const responses = (op as Record<string, unknown>)?.responses as
            | Record<string, unknown>
            | undefined;
          if (!responses) continue;
          for (const [status, res] of Object.entries(responses)) {
            out.push({ pointer: `/paths/${enc(p)}/${key}/responses/${status}`, value: res });
          }
        }
      }
      return out;
    }
    case "$.components.schemas[*]": {
      const schemas = (root.components as Record<string, unknown> | undefined)
        ?.schemas as Record<string, unknown> | undefined;
      if (!schemas) return out;
      for (const [name, schema] of Object.entries(schemas)) {
        out.push({ pointer: `/components/schemas/${enc(name)}`, value: schema });
      }
      return out;
    }
    default:
      throw new Error(
        `testRule: selector "${selector.path}" needs explicit pointers — pass nodes via ctx.nodes`,
      );
  }
}

function enc(s: string): string {
  return s.replace(/~/g, "~0").replace(/\//g, "~1");
}

export interface TestRuleOptions {
  /** Explicit node pointers, overriding selector evaluation (custom raw
   * JSONPath selectors require this). */
  nodes?: string[];
  /** Document uri reported in ctx. */
  docUri?: string;
}

/** Runs `rule` against `doc`, feeding each finding to `assert`. */
export async function testRule(
  rule: RuleDefinition,
  doc: unknown,
  assert: (finding: CapturedFinding) => void,
  options: TestRuleOptions = {},
): Promise<void> {
  const findings: CapturedFinding[] = [];
  const push = (f: CapturedFinding) => findings.push(f);

  if (isWalkRule(rule)) {
    const state: Record<string, unknown> = {};
    const ctx = {
      report: (f: { message: string; at?: { pointer: string }; severity?: string }) =>
        push({ pointer: f.at?.pointer ?? "", message: f.message, severity: f.severity }),
      state,
    };
    rule.visitors.onDocument?.({ pointer: "" }, ctx);
    for (const node of walkAll("", doc)) {
      // classify inline (mirrors worker)
      const segs = node.pointer.split("/").filter((s) => s.length > 0);
      const last = segs.at(-1) ?? "";
      if (segs[0] === "paths" && segs.length === 2) {
        const wrapped = wrapSelected({ path: "", wrap: "pathItem" }, node.pointer, node.value);
        if (wrapped) rule.visitors.PathItem?.(wrapped as never, ctx);
      } else if (segs[0] === "paths" && segs.length === 3 && isMethodKey(last)) {
        const wrapped = wrapSelected({ path: "", wrap: "operation" }, node.pointer, node.value);
        if (wrapped) rule.visitors.Operation?.(wrapped as never, ctx);
      } else if (segs[0] === "components" && segs[1] === "schemas" && segs.length === 3) {
        const wrapped = wrapSelected({ path: "", wrap: "schema" }, node.pointer, node.value);
        if (wrapped) rule.visitors.Schema?.(wrapped as never, ctx);
      }
    }
  } else {
    const pointers =
      options.nodes ?? evaluateSelector(rule.given, doc).map((n) => n.pointer);
    const byPointer = new Map(
      evaluateSelector(rule.given, doc).map((n) => [n.pointer, n.value] as const),
    );
    for (const pointer of pointers) {
      const value = byPointer.get(pointer) ?? atPointer(doc, pointer);
      const wrapped: SelectedNode | undefined = wrapSelected(rule.given, pointer, value);
      if (!wrapped) continue;
      if (rule.given.wrap === "operation" && wrapped.kind !== "operation") continue;
      const ctx = {
        report: (f: {
          message: string;
          at?: { pointer?: string; node?: SelectedNode; field?: string };
          severity?: string;
        }) => {
          let p = pointer;
          if (f.at?.pointer) p = f.at.pointer;
          else if (f.at?.node && f.at.field) p = `${f.at.node.pointer}/${f.at.field}`;
          push({ pointer: p, message: f.message, severity: f.severity });
        },
        readWorkspaceFile: async () => undefined,
        docUri: options.docUri,
      };
      await rule.check(wrapped, ctx);
    }
  }

  for (const f of findings) assert(f);
}

function atPointer(doc: unknown, pointer: string): unknown {
  let cur: unknown = doc;
  for (const seg of pointer.split("/").filter((s) => s.length > 0)) {
    const key = seg.replace(/~1/g, "/").replace(/~0/g, "~");
    if (Array.isArray(cur)) cur = cur[Number(key)];
    else if (cur && typeof cur === "object") cur = (cur as Record<string, unknown>)[key];
    else return undefined;
  }
  return cur;
}

function* walkAll(
  pointer: string,
  value: unknown,
): Generator<{ pointer: string; value: unknown }> {
  yield { pointer, value };
  if (Array.isArray(value)) {
    for (let i = 0; i < value.length; i++) {
      yield* walkAll(`${pointer}/${i}`, value[i]);
    }
  } else if (value && typeof value === "object") {
    for (const [k, v] of Object.entries(value as Record<string, unknown>)) {
      yield* walkAll(`${pointer}/${enc(k)}`, v);
    }
  }
}
