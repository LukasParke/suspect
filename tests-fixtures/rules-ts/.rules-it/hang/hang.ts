
import { defineRule, r } from "@suspect/rules-sdk";

export default defineRule({
  meta: { id: "hang-forever", description: "test watchdog" },
  given: r.operation,
  check(_op, _ctx) {
    // Busy-loop: the host watchdog must kill the worker.
    const start = Date.now();
    while (Date.now() - start < 60_000) {}
  },
});
