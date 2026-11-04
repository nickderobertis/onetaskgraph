import "../../test-setup.mjs";
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { readdirSync, readFileSync } from "node:fs";
import { relative, resolve } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";
import { fixtureEntryEnvironment } from "../../fixture-environment.mjs";

const root = fileURLToPath(new URL("../../../", import.meta.url));
function entryProblems(files, runners) {
  const problems = [];
  for (const [file, text] of Object.entries(files)) {
    let setup = relative(
      resolve(root, file, ".."),
      resolve(root, ".github/test-setup.mjs"),
    ).replaceAll("\\", "/");
    if (!setup.startsWith(".")) setup = `./${setup}`;
    if (text.split(/\r?\n/, 1)[0] !== `import ${JSON.stringify(setup)};`)
      problems.push(`${file}: load test-setup before any helper`);
  }
  for (const [name, project] of Object.entries(runners))
    for (const [key, expected] of Object.entries(fixtureEntryEnvironment))
      if (project.targets.test.options.env?.[key] !== expected)
        problems.push(`${name}: test entry does not isolate ${key}`);
  return problems;
}
function testFiles(directory) {
  return readdirSync(directory, { withFileTypes: true }).flatMap((entry) => {
    const path = resolve(directory, entry.name);
    if (entry.isDirectory())
      return ["node_modules", "fixtures", ".telemetry"].includes(entry.name) ? [] : testFiles(path);
    return /\.(?:test|spec)\.mjs$/.test(entry.name) ? [path] : [];
  });
}
const files = Object.fromEntries(
  testFiles(resolve(root, ".github")).map((file) => [
    relative(root, file).replaceAll("\\", "/"),
    readFileSync(file, "utf8"),
  ]),
);
const runnerFiles = execFileSync("git", ["ls-files", "--", "**/project.json"], {
  cwd: root,
  encoding: "utf8",
})
  .trim()
  .split("\n")
  .filter((file) => !file.includes("/fixtures/"));
const runners = Object.fromEntries(
  runnerFiles.flatMap((file) => {
    const project = JSON.parse(readFileSync(resolve(root, file), "utf8"));
    return (project.name === "workspace" || project.tags?.includes("lang:bash")) &&
      project.targets?.test?.executor === "nx:run-commands"
      ? [[project.name, project]]
      : [];
  }),
);

test("every CI Node test and shell/Python guard runner isolates its process entry", () => {
  assert.deepEqual(entryProblems(files, runners), []);
  assert.deepEqual(
    entryProblems(
      Object.fromEntries(
        Object.entries(files).map(([file, text]) => [file, text.replaceAll("\n", "\r\n")]),
      ),
      runners,
    ),
    [],
  );
});
test("a new test without setup, or a runner inheriting one cache field, is refused", () => {
  assert.deepEqual(
    entryProblems(
      {
        ".github/tests/new.test.mjs":
          'import { spawnSync } from "node:child_process";\nspawnSync("bash", ["script.sh"], {env: process.env});\n',
      },
      {},
    ),
    [".github/tests/new.test.mjs: load test-setup before any helper"],
  );
  for (const name of Object.keys(runners))
    for (const key of Object.keys(fixtureEntryEnvironment)) {
      const changed = structuredClone(runners);
      delete changed[name].targets.test.options.env[key];
      assert.deepEqual(entryProblems({}, changed), [`${name}: test entry does not isolate ${key}`]);
    }
});
