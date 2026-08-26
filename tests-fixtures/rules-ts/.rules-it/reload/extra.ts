
import { defineRule, r } from "@suspect/rules-sdk";

export default defineRule({
  meta: { id: "throws", description: "test error handling" },
  given: r.operation,
  check() {
    throw new Error("boom from rule");
  },
});
