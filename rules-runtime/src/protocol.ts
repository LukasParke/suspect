/**
 * Wire protocol between the suspect host (Rust) and the TS rule worker.
 *
 * Transport: NDJSON over the worker's stdio — one JSON object per line.
 * Every frame carries a `t` discriminator. Versioned at `hello`; the host
 * kills and restarts the worker on any protocol violation.
 */

export const PROTOCOL_VERSION = 1;

/** Host → worker: handshake with the rule files to load. */
export interface HelloFrame {
  t: "hello";
  protocol: number;
  sdk_version: string;
  workspace_root: string;
  rule_files: string[];
}

/** Worker → host: rules loaded; metadata lets the host pre-select nodes. */
export interface ReadyFrame {
  t: "ready";
  bun: string;
  sdk: string;
  rules: RuleMeta[];
}

export interface RuleMeta {
  /** Unique rule id from `meta.id`. */
  id: string;
  /** File the rule was loaded from. */
  file: string;
  /** Which document kinds the rule evaluates. */
  targets: TargetKind[];
  /** JSONPath `given` selector (point rules). */
  given?: string;
  /** `point` (per-node check) or `walk` (visitors over the document). */
  shape: "point" | "walk";
  /** Visitor keys declared by walk rules (host informational). */
  visitors?: string[];
  /** True when the rule file imports zod (host informational). */
  usesZod?: boolean;
}

export type TargetKind = "spec" | "facts" | "union";

/** Host → worker: run rules over one shipped document. */
export interface EvaluateFrame {
  t: "evaluate";
  run_id: number;
  timeout_ms: number;
  doc_kind: TargetKind;
  doc_uri?: string;
  /** The document, shipped once for the whole run. */
  document: { value: unknown };
  /** Per-point-rule selected pointers, resolved against `document`. */
  selections: Record<string, string[]>;
  /** Per-walk-rule walk entries. */
  walks: Record<string, { root_pointer: string }>;
}

/** Worker → host: batched findings for one run. */
export interface FindingsBatchFrame {
  t: "findings_batch";
  run_id: number;
  findings: FindingFrame[];
}

/** Worker → host: one violation (legacy single-frame path). */
export interface FindingFrame {
  t: "finding";
  run_id: number;
  rule_id: string;
  pointer: string;
  message: string;
  severity?: "error" | "warning" | "info" | "hint";
  /** Optional structured quick-fix intent (host executes it). */
  fix?: FixIntent;
}

export interface FixIntent {
  kind: "insert-doc" | "document-in-spec" | "note";
  template?: string;
  message?: string;
}

/** Worker → host: a run finished. */
export interface DoneFrame {
  t: "done";
  run_id: number;
  ms: number;
  findings: number;
}

/** Worker → host: a rule threw; host disables it for the session. */
export interface RuleErrorFrame {
  t: "rule_error";
  run_id: number;
  rule_id: string;
  message: string;
}

/** Worker → host: fatal worker problem (protocol/bun level). */
export interface FatalFrame {
  t: "fatal";
  message: string;
}

/** Host → worker: re-import changed rule files. */
export interface ReloadFrame {
  t: "reload";
  files: string[];
}

/** Liveness probes. */
export interface PingFrame {
  t: "ping";
}
export interface PongFrame {
  t: "pong";
}

export type HostFrame = HelloFrame | EvaluateFrame | ReloadFrame | PingFrame;
export type WorkerFrame =
  | ReadyFrame
  | FindingsBatchFrame
  | FindingFrame
  | DoneFrame
  | RuleErrorFrame
  | FatalFrame
  | PongFrame;
