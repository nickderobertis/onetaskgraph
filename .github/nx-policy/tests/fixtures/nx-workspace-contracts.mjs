// Read the effective inputs and outputs through the same Nx graph its runner uses.

import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";
import { resolve } from "node:path";

process.env.NX_DAEMON = "false";
process.env.NX_WORKSPACE_ROOT_PATH = process.cwd();
const require = createRequire(resolve("package.json"));
const { createProjectGraphAsync } = require("nx/src/project-graph/project-graph.js");
const graph = await createProjectGraphAsync({ exitOnError: true });
if (process.argv[2]) {
  const ids = JSON.parse(readFileSync(process.argv[2], "utf8"));
  assert.ok(
    Array.isArray(ids) &&
      ids.every((id) => typeof id === "string" && /^[\w@./-]+:[\w.-]+$/.test(id)),
  );
  const { createTaskGraph } = require("nx/src/tasks-runner/create-task-graph.js");
  const { createTaskHasher } = require("nx/src/hasher/create-task-hasher.js");
  const { readNxJson } = require("nx/src/config/nx-json.js");
  const hasher = createTaskHasher(graph, readNxJson(process.cwd()));
  const taskGraph = { roots: ids, tasks: {}, dependencies: {}, continuousDependencies: {} };
  for (const id of ids) {
    const [project, target] = id.split(":");
    const selected = createTaskGraph(graph, {}, [project], [target], undefined, {});
    assert.ok(selected.tasks[id], `${id}: task exists`);
    for (const key of ["tasks", "dependencies", "continuousDependencies"])
      Object.assign(taskGraph[key], selected[key]);
  }
  // The runner hashes a batch together: shared runtime inputs execute once per batch.
  const batch = await hasher.hashTasks(
    ids.map((id) => taskGraph.tasks[id]),
    taskGraph,
    process.env,
    undefined,
    true,
  );
  const tasks = Object.fromEntries(
    ids.map((id, index) => {
      const [project, target] = id.split(":");
      const hash = batch[index];
      return [
        id,
        {
          hash: hash.value,
          inputs: hash.inputs,
          nodes: hash.details.nodes,
          definition: graph.nodes[project].data.targets[target],
        },
      ];
    }),
  );
  process.stdout.write(JSON.stringify(tasks));
} else {
  process.stdout.write(
    JSON.stringify(
      Object.fromEntries(Object.entries(graph.nodes).map(([name, node]) => [name, node.data])),
    ),
  );
}
