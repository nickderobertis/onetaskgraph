// Read the effective inputs and outputs through the same Nx graph its runner uses.
import { createRequire } from "node:module";
import { resolve } from "node:path";

process.env.NX_DAEMON = "false";
process.env.NX_WORKSPACE_ROOT_PATH = process.cwd();
const require = createRequire(resolve("package.json"));
const { createProjectGraphAsync } = require("nx/src/project-graph/project-graph.js");
const graph = await createProjectGraphAsync({ exitOnError: true });
process.stdout.write(
  JSON.stringify(
    Object.fromEntries(Object.entries(graph.nodes).map(([name, node]) => [name, node.data])),
  ),
);
