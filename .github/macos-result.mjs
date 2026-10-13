import { readFileSync } from "node:fs";

// Job outcomes are the GitHub boundary. Missing or cancelled proofs never become green.
try {
  const needs = JSON.parse(readFileSync(process.argv[2], "utf8"));
  if (needs["macos-lanes"]?.result !== "success")
    throw Error("macOS classification failed or is missing");
  const diagnostic = needs["macos-lanes"].outputs?.diagnostic;
  if (diagnostic !== undefined && !["true", "false"].includes(diagnostic))
    throw Error("Malformed macOS diagnostic output");
  const required =
    needs["macos-lanes"].outputs?.diagnostic === "true"
      ? "true"
      : needs["macos-lanes"].outputs?.macosRequired;
  const result = needs.macos?.result;
  if (required === "true" && result === "success")
    console.log("The combined macOS check and install proofs passed.");
  else if (required === "false" && result === "skipped")
    console.log(`macOS omitted: ${needs["macos-lanes"].outputs.reason}`);
  else throw Error(`Required macOS work did not succeed: required=${required}, result=${result}`);
} catch (error) {
  console.error(error.message);
  process.exitCode = 1;
}
