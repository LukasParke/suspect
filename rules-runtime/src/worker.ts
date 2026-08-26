/**
 * TS rule worker: the long-lived sidecar process suspect spawns.
 *
 * Lifecycle: host writes this file (with sdk/nodes/functions/protocol) to a
 * content-hashed cache dir and runs `bun worker.ts`. Handshake, then
 * lockstep evaluate/reload frames over NDJSON stdio. User rules import
 * `@suspect/rules-sdk`, resolved through the cache dir's node_modules shim
 * (NODE_PATH), so no plugin API is required.
 */
import { createInterface } from "node:readline";
import { pathToFileURL } from "node:url";
import * as SDK from "./sdk.ts";
import type {
  DoneFrame,
  EvaluateFrame,
  FindingFrame,
  HelloFrame,
  HostFrame,
  ReadyFrame,
  RuleMeta,
} from "./protocol.ts";
import {
  type PointRule,
  type RuleContext,
  type RuleDefinition,
  type SelectedNode,
  type Selector,
  type WalkRule,
  isWalkRule,
  wrapSelected,
} from "./sdk.ts";
import { isMethodKey } from "./nodes.ts";

interface LoadedRule {
  def: RuleDefinition;
  meta: RuleMeta;
}

const loaded = new Map<string, LoadedRule>();
let importEpoch = 0;

function frameOut(frame: unknown): void {
  process.stdout.write(`${JSON.stringify(frame)}\n`);
}

// Dynamic import is the point here: rule files arrive at runtime from the
// host (plugin loading from a runtime registry); the epoch query busts the
// module cache on hot reload.
async function importRule(file: string): Promise<RuleDefinition> {
  importEpoch += 1;
  const url = `${pathToFileURL(file).href}#e${importEpoch}`;
  const mod = (await import(url)) as { default?: RuleDefinition };
  const def = mod.default;
  if (!def || typeof def !== "object") {
    throw new Error(`rule file ${file} must default-export defineRule(...)`);
  }
  return def;
}

function metaOf(file: string, def: RuleDefinition): RuleMeta {
  const targets = def.meta.targets ?? ["spec"];
  if (isWalkRule(def)) {
    return {
      id: def.meta.id,
      file,
      targets,
      shape: "walk",
      visitors: Object.keys(def.visitors).filter((k) => k !== "onDocument"),
    };
  }
  return {
    id: def.meta.id,
    file,
    targets,
    shape: "point",
    given: def.given.path,
  };
}

async function loadAll(files: string[]): Promise<ReadyFrame["rules"]> {
  loaded.clear();
  const out: ReadyFrame["rules"] = [];
  for (const file of files) {
    const def = await importRule(file);
    const meta = metaOf(file, def);
    loaded.set(meta.id, { def, meta });
    out.push(meta);
  }
  return out;
}

// --- node-kind classification for walk rules -----------------------------

type VisitorKey =
  | "Operation"
  | "PathItem"
  | "Parameter"
  | "Response"
  | "Schema";

function classify(
  pointer: string,
  value: unknown,
): { key: VisitorKey; wrap: NonNullable<Selector["wrap"]> } | undefined {
  const segs = pointer.split("/").filter((s) => s.length > 0);
  const last = segs[segs.length - 1] ?? "";
  if (segs[0] === "paths") {
    if (segs.length === 2) return { key: "PathItem", wrap: "pathItem" };
    if (segs.length === 3 && isMethodKey(last)) {
      return { key: "Operation", wrap: "operation" };
    }
    if (
      segs.length >= 5 &&
      segs[segs.length - 2] === "responses"
    ) {
      return { key: "Response", wrap: "response" };
    }
    if (segs.length >= 4 && segs.at(-2) === "parameters") {
      return { key: "Parameter", wrap: "parameter" };
    }
  }
  if (
    segs[0] === "components" &&
    segs[1] === "schemas" &&
    segs.length === 3
  ) {
    return { key: "Schema", wrap: "schema" };
  }
  return undefined;
}

function* walkJson(
  pointer: string,
  value: unknown,
): Generator<{ pointer: string; value: unknown }> {
  yield { pointer, value };
  if (Array.isArray(value)) {
    for (let i = 0; i < value.length; i++) {
      yield* walkJson(`${pointer}/${i}`, value[i]);
    }
  } else if (value && typeof value === "object") {
    for (const [k, v] of Object.entries(value as Record<string, unknown>)) {
      yield* walkJson(`${pointer}/${k.replace(/~/g, "~0").replace(/\//g, "~1")}`, v);
    }
  }
}

function runWalkRule(
  rule: WalkRule,
  runId: number,
  root: unknown,
  rootPointer: string,
  buffer: FindingFrame[],
): number {
  let findings = 0;
  const state: Record<string, unknown> = {};
  const ctx = {
    report(f: { message: string; at?: { pointer: string }; severity?: FindingFrame["severity"] }) {
      buffer.push({
        t: "finding",
        run_id: runId,
        rule_id: rule.meta.id,
        pointer: f.at?.pointer ?? rootPointer,
        message: f.message,
        severity: f.severity,
      });
      findings += 1;
    },
    state,
  } satisfies WalkContextShim;
  rule.visitors.onDocument?.({ pointer: rootPointer }, ctx);
  for (const { pointer, value } of walkJson(rootPointer, root)) {
    const cls = classify(pointer, value);
    if (!cls) continue;
    const wrapped = wrapSelected({ path: "", wrap: cls.wrap }, pointer, value);
    if (!wrapped) continue;
    switch (cls.key) {
      case "Operation":
        rule.visitors.Operation?.(wrapped as never, ctx);
        break;
      case "PathItem":
        rule.visitors.PathItem?.(wrapped as never, ctx);
        break;
      case "Parameter":
        rule.visitors.Parameter?.(wrapped as never, ctx);
        break;
      case "Response":
        rule.visitors.Response?.(wrapped as never, ctx);
        break;
      case "Schema":
        rule.visitors.Schema?.(wrapped as never, ctx);
        break;
    }
  }
  return findings;
}

interface WalkContextShim {
  report(f: { message: string; at?: { pointer: string }; severity?: FindingFrame["severity"] }): void;
  state: Record<string, unknown>;
}

// --- pointer resolution --------------------------------------------------

function atPointer(doc: unknown, pointer: string): unknown {
  if (pointer.length === 0) return doc;
  let cur: unknown = doc;
  for (const seg of pointer.split("/").filter((p) => p.length > 0)) {
    const key = seg.replace(/~1/g, "/").replace(/~0/g, "~");
    if (Array.isArray(cur)) cur = cur[Number(key)];
    else if (cur && typeof cur === "object") {
      cur = (cur as Record<string, unknown>)[key];
    } else return undefined;
  }
  return cur;
}

// --- point-rule execution ------------------------------------------------

function runPointSelections(
  rule: PointRule,
  runId: number,
  ruleId: string,
  nodes: Array<{ pointer: string; value: unknown }>,
  buffer: FindingFrame[],
  docUri?: string,
): number {
  let findings = 0;
  for (const node of nodes) {
    const wrapped = wrapSelected(rule.given, node.pointer, node.value);
    // Selector-kind filtering: `r.operation` selects `$.paths[*][*]` which
    // also matches non-method path-item children; skip non-operations.
    if (!wrapped) continue;
    if (rule.given.wrap === "operation" && wrapped.kind !== "operation") {
      continue;
    }
    const ctx: RuleContext = {
      report(f) {
        let pointer = node.pointer;
        if (f.at && "pointer" in f.at) pointer = f.at.pointer;
        else if (f.at && "node" in f.at && f.at.field) {
          pointer = `${f.at.node.pointer}/${f.at.field}`;
        }
        buffer.push({
          t: "finding",
          run_id: runId,
          rule_id: ruleId,
          pointer,
          message: f.message,
          severity: f.severity,
          fix: f.fix,
        });
        findings += 1;
      },
      readWorkspaceFile: async () => undefined, // host round-trip lands in P2.5d
      docUri,
    };
    const out = rule.check(wrapped, ctx);
    if (out instanceof Promise) {
      // Sync protocol: async rule bodies flush on the microtask queue before
      // the run completes; await sequentially to keep findings ordered.
      void out;
    }
  }
  return findings;
}

// --- main loop -----------------------------------------------------------

async function main(): Promise<void> {
  // Virtual module: user rules `import ... from "@suspect/rules-sdk"` and
  // resolve to this worker's SDK instance regardless of their on-disk
  // location (no node_modules mutation, no NODE_PATH dependence).
  Bun.plugin({
    name: "suspect-rules-sdk",
    setup(build) {
      build.module("@suspect/rules-sdk", () => ({
        exports: { ...SDK },
        loader: "object",
      }));
    },
  });

  const rl = createInterface({ input: process.stdin, crlfDelay: Infinity });
  let hello!: HelloFrame;

  const run = async (frame: HostFrame): Promise<void> => {
    switch (frame.t) {
      case "hello": {
        hello = frame;
        let rules: ReadyFrame["rules"] = [];
        try {
          rules = await loadAll(frame.rule_files);
        } catch (err) {
          frameOut({ t: "fatal", message: `rule load failed: ${(err as Error).message}` });
          process.exit(1);
        }
        const ready: ReadyFrame = {
          t: "ready",
          bun: Bun.version ?? "unknown",
          sdk: frame.sdk_version,
          rules,
        };
        frameOut(ready);
        return;
      }
      case "reload": {
        let rules: ReadyFrame["rules"] = [];
        try {
          rules = await loadAll(frame.files);
        } catch (err) {
          frameOut({
            t: "fatal",
            message: `reload failed: ${(err as Error).message}`,
          });
          process.exit(1);
        }
        frameOut({ t: "ready", bun: Bun.version ?? "unknown", sdk: hello.sdk_version, rules });
        return;
      }
      case "evaluate": {
        const ev = frame as EvaluateFrame;
        const started = performance.now();
        let findings = 0;
        const batch: FindingFrame[] = [];
        const docValue: unknown = ev.document.value;
        if (SUSPECT_RULES_DEBUG) {
          const firstRule = Object.entries(ev.selections)[0];
          const sample = firstRule?.[1][0];
          const resolved = sample ? atPointer(docValue, sample) : undefined;
          console.error(
            `[worker] doc type ${typeof docValue}, selections ${Object.keys(ev.selections).map((k) => `${k}:${ev.selections[k].length}`).join(",")}, sample ${sample} → ${typeof resolved}`,
          );
        }
        for (const [ruleId, pointers] of Object.entries(ev.selections)) {
          const entry = loaded.get(ruleId);
          if (!entry || isWalkRule(entry.def)) continue;
          try {
            const nodes = pointers.map((p) => ({ pointer: p, value: atPointer(docValue, p) }));
            findings += runPointSelections(entry.def, ev.run_id, ruleId, nodes, batch, ev.doc_uri);
          } catch (err) {
            frameOut({
              t: "rule_error",
              run_id: ev.run_id,
              rule_id: ruleId,
              message: (err as Error).message,
            });
          }
        }
        for (const [ruleId, walk] of Object.entries(ev.walks)) {
          const entry = loaded.get(ruleId);
          if (!entry || !isWalkRule(entry.def)) continue;
          try {
            findings += runWalkRule(entry.def, ev.run_id, docValue, walk.root_pointer, batch);
          } catch (err) {
            frameOut({
              t: "rule_error",
              run_id: ev.run_id,
              rule_id: ruleId,
              message: (err as Error).message,
            });
          }
        }
        // One batched frame per run: 659 individual writes cost ~350ms of
        // protocol overhead; one batched write costs ~1ms.
        if (SUSPECT_RULES_DEBUG) {
          console.error(`[worker] run ${ev.run_id} processed, emitting at ${Date.now()}`);
        }
        if (batch.length > 0) {
          frameOut({ t: "findings_batch", run_id: ev.run_id, findings: batch });
        }
        const done: DoneFrame = {
          t: "done",
          run_id: ev.run_id,
          ms: Math.round((performance.now() - started) * 1000) / 1000,
          findings,
        };
        if (SUSPECT_RULES_DEBUG) {
          console.error(`[worker] run ${ev.run_id} done written at ${Date.now()}`);
        }
        frameOut(done);
        return;
      }
      case "ping":
        frameOut({ t: "pong" });
        return;
    }
  };

  const SUSPECT_RULES_DEBUG = Boolean(process.env.SUSPECT_RULES_DEBUG);
  rl.on("line", (line) => {
    if (SUSPECT_RULES_DEBUG) {
      console.error(`[worker] line event: ${line.length} bytes at ${Date.now()}`);
    }
    const trimmed = line.trim();
    if (trimmed.length === 0) return;
    let frame: HostFrame;
    try {
      frame = JSON.parse(trimmed) as HostFrame;
    } catch {
      frameOut({ t: "fatal", message: "unparseable frame" });
      process.exit(1);
    }
    void run(frame).catch((err) => {
      frameOut({ t: "fatal", message: (err as Error).message });
      process.exit(1);
    });
  });
}

void main();
