
import { defineRule, r } from "@suspect/rules-sdk";

export default defineRule({
  meta: { id: "facts-only", description: "unsupported target", targets: ["facts"] },
  given: r.operation,
  check() {},
});
