import { readFileSync } from "node:fs";
import { parse } from "yaml";

// Parse Bun's trailing-comma lock syntax here; Python validates the extracted versions
// through product_versions.py's authoritative grammar before accepting this comparison.
const args = process.argv.slice(2);
if (args.length > 1 || (args.length === 1 && args[0] !== "--parse-only")) {
  console.error(
    "usage: semantic-json.mjs [--parse-only]; next: pass only the supported parser mode",
  );
  process.exit(1);
}
const normalized = parse(readFileSync(0, "utf8"));
if (process.argv.includes("--parse-only")) {
  console.log(JSON.stringify(normalized));
  process.exit(0);
}
const sdk = normalized.workspaces?.["sdks/typescript"];
if (sdk?.name !== "@onetaskgraph/sdk") throw Error("Missing SDK lock entry");
const versions = [];
for (const [node, key] of [
  [sdk, "version"],
  [sdk.optionalDependencies, "@onetaskgraph/cli"],
]) {
  if (typeof node?.[key] !== "string") throw Error("Missing locked product version");
  versions.push(node[key]);
  node[key] = "<version>";
}
console.log(JSON.stringify({ normalized, versions }));
