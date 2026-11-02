**Section-links spike source.** Committed by the section-links spike harness. Relative links name a file beside this one; absolute ones name it by its path on the machine that ran the harness.

Every link is labelled `<destination>-<target>-<form><heading>`: target `P` plain, `I` `<a id>` anchors, `N` `<a name>` anchors; form `S` `#<slug>`, `U` `#user-content-<slug>` (`A`/`B` the same two against an absolute path). Each names the heading it must land on.

## Links to the plain target

Target: `./target.md` relative to this file; forms A and B name it by its absolute path `/home/nick/.onevcs/workspaces/github.com-nickderobertis-onetaskgraph-9c94c67fcf19/pool/1/worktree/scripts/spikes/section-links/local/target.md`

Form `#<slug>`:

- [LM-P-S1 What](./target.md#what) — must land on `## What`
- [LM-P-S2 Budgets](./target.md#budgets) — must land on `## Budgets`
- [LM-P-S3 Budget demo-quota-seconds](./target.md#budget-demo-quota-seconds) — must land on `### Budget demo-quota-seconds`
- [LM-P-S4 Step 2.b: retry (once)](./target.md#step-2b-retry-once) — must land on `### Step 2.b: retry (once)`
- [LM-P-S5 Budgets](./target.md#budgets-1) — must land on `## Budgets (the second heading with this text, at the end)`

Form `#user-content-<slug>`:

- [LM-P-U1 What](./target.md#user-content-what) — must land on `## What`
- [LM-P-U2 Budgets](./target.md#user-content-budgets) — must land on `## Budgets`
- [LM-P-U3 Budget demo-quota-seconds](./target.md#user-content-budget-demo-quota-seconds) — must land on `### Budget demo-quota-seconds`
- [LM-P-U4 Step 2.b: retry (once)](./target.md#user-content-step-2b-retry-once) — must land on `### Step 2.b: retry (once)`
- [LM-P-U5 Budgets](./target.md#user-content-budgets-1) — must land on `## Budgets (the second heading with this text, at the end)`

Form `<absolute path>#<slug>`:

- [LM-P-A1 What](/home/nick/.onevcs/workspaces/github.com-nickderobertis-onetaskgraph-9c94c67fcf19/pool/1/worktree/scripts/spikes/section-links/local/target.md#what) — must land on `## What`
- [LM-P-A2 Budgets](/home/nick/.onevcs/workspaces/github.com-nickderobertis-onetaskgraph-9c94c67fcf19/pool/1/worktree/scripts/spikes/section-links/local/target.md#budgets) — must land on `## Budgets`
- [LM-P-A3 Budget demo-quota-seconds](/home/nick/.onevcs/workspaces/github.com-nickderobertis-onetaskgraph-9c94c67fcf19/pool/1/worktree/scripts/spikes/section-links/local/target.md#budget-demo-quota-seconds) — must land on `### Budget demo-quota-seconds`
- [LM-P-A4 Step 2.b: retry (once)](/home/nick/.onevcs/workspaces/github.com-nickderobertis-onetaskgraph-9c94c67fcf19/pool/1/worktree/scripts/spikes/section-links/local/target.md#step-2b-retry-once) — must land on `### Step 2.b: retry (once)`
- [LM-P-A5 Budgets](/home/nick/.onevcs/workspaces/github.com-nickderobertis-onetaskgraph-9c94c67fcf19/pool/1/worktree/scripts/spikes/section-links/local/target.md#budgets-1) — must land on `## Budgets (the second heading with this text, at the end)`

Form `<absolute path>#user-content-<slug>`:

- [LM-P-B1 What](/home/nick/.onevcs/workspaces/github.com-nickderobertis-onetaskgraph-9c94c67fcf19/pool/1/worktree/scripts/spikes/section-links/local/target.md#user-content-what) — must land on `## What`
- [LM-P-B2 Budgets](/home/nick/.onevcs/workspaces/github.com-nickderobertis-onetaskgraph-9c94c67fcf19/pool/1/worktree/scripts/spikes/section-links/local/target.md#user-content-budgets) — must land on `## Budgets`
- [LM-P-B3 Budget demo-quota-seconds](/home/nick/.onevcs/workspaces/github.com-nickderobertis-onetaskgraph-9c94c67fcf19/pool/1/worktree/scripts/spikes/section-links/local/target.md#user-content-budget-demo-quota-seconds) — must land on `### Budget demo-quota-seconds`
- [LM-P-B4 Step 2.b: retry (once)](/home/nick/.onevcs/workspaces/github.com-nickderobertis-onetaskgraph-9c94c67fcf19/pool/1/worktree/scripts/spikes/section-links/local/target.md#user-content-step-2b-retry-once) — must land on `### Step 2.b: retry (once)`
- [LM-P-B5 Budgets](/home/nick/.onevcs/workspaces/github.com-nickderobertis-onetaskgraph-9c94c67fcf19/pool/1/worktree/scripts/spikes/section-links/local/target.md#user-content-budgets-1) — must land on `## Budgets (the second heading with this text, at the end)`

## Links to the anchor-id target

Target: `./target-anchor-id.md` relative to this file; forms A and B name it by its absolute path `/home/nick/.onevcs/workspaces/github.com-nickderobertis-onetaskgraph-9c94c67fcf19/pool/1/worktree/scripts/spikes/section-links/local/target-anchor-id.md`

Form `#<slug>`:

- [LM-I-S1 What](./target-anchor-id.md#what) — must land on `## What`
- [LM-I-S2 Budgets](./target-anchor-id.md#budgets) — must land on `## Budgets`
- [LM-I-S3 Budget demo-quota-seconds](./target-anchor-id.md#budget-demo-quota-seconds) — must land on `### Budget demo-quota-seconds`
- [LM-I-S4 Step 2.b: retry (once)](./target-anchor-id.md#step-2b-retry-once) — must land on `### Step 2.b: retry (once)`
- [LM-I-S5 Budgets](./target-anchor-id.md#budgets-1) — must land on `## Budgets (the second heading with this text, at the end)`

Form `#user-content-<slug>`:

- [LM-I-U1 What](./target-anchor-id.md#user-content-what) — must land on `## What`
- [LM-I-U2 Budgets](./target-anchor-id.md#user-content-budgets) — must land on `## Budgets`
- [LM-I-U3 Budget demo-quota-seconds](./target-anchor-id.md#user-content-budget-demo-quota-seconds) — must land on `### Budget demo-quota-seconds`
- [LM-I-U4 Step 2.b: retry (once)](./target-anchor-id.md#user-content-step-2b-retry-once) — must land on `### Step 2.b: retry (once)`
- [LM-I-U5 Budgets](./target-anchor-id.md#user-content-budgets-1) — must land on `## Budgets (the second heading with this text, at the end)`

Form `<absolute path>#<slug>`:

- [LM-I-A1 What](/home/nick/.onevcs/workspaces/github.com-nickderobertis-onetaskgraph-9c94c67fcf19/pool/1/worktree/scripts/spikes/section-links/local/target-anchor-id.md#what) — must land on `## What`
- [LM-I-A2 Budgets](/home/nick/.onevcs/workspaces/github.com-nickderobertis-onetaskgraph-9c94c67fcf19/pool/1/worktree/scripts/spikes/section-links/local/target-anchor-id.md#budgets) — must land on `## Budgets`
- [LM-I-A3 Budget demo-quota-seconds](/home/nick/.onevcs/workspaces/github.com-nickderobertis-onetaskgraph-9c94c67fcf19/pool/1/worktree/scripts/spikes/section-links/local/target-anchor-id.md#budget-demo-quota-seconds) — must land on `### Budget demo-quota-seconds`
- [LM-I-A4 Step 2.b: retry (once)](/home/nick/.onevcs/workspaces/github.com-nickderobertis-onetaskgraph-9c94c67fcf19/pool/1/worktree/scripts/spikes/section-links/local/target-anchor-id.md#step-2b-retry-once) — must land on `### Step 2.b: retry (once)`
- [LM-I-A5 Budgets](/home/nick/.onevcs/workspaces/github.com-nickderobertis-onetaskgraph-9c94c67fcf19/pool/1/worktree/scripts/spikes/section-links/local/target-anchor-id.md#budgets-1) — must land on `## Budgets (the second heading with this text, at the end)`

Form `<absolute path>#user-content-<slug>`:

- [LM-I-B1 What](/home/nick/.onevcs/workspaces/github.com-nickderobertis-onetaskgraph-9c94c67fcf19/pool/1/worktree/scripts/spikes/section-links/local/target-anchor-id.md#user-content-what) — must land on `## What`
- [LM-I-B2 Budgets](/home/nick/.onevcs/workspaces/github.com-nickderobertis-onetaskgraph-9c94c67fcf19/pool/1/worktree/scripts/spikes/section-links/local/target-anchor-id.md#user-content-budgets) — must land on `## Budgets`
- [LM-I-B3 Budget demo-quota-seconds](/home/nick/.onevcs/workspaces/github.com-nickderobertis-onetaskgraph-9c94c67fcf19/pool/1/worktree/scripts/spikes/section-links/local/target-anchor-id.md#user-content-budget-demo-quota-seconds) — must land on `### Budget demo-quota-seconds`
- [LM-I-B4 Step 2.b: retry (once)](/home/nick/.onevcs/workspaces/github.com-nickderobertis-onetaskgraph-9c94c67fcf19/pool/1/worktree/scripts/spikes/section-links/local/target-anchor-id.md#user-content-step-2b-retry-once) — must land on `### Step 2.b: retry (once)`
- [LM-I-B5 Budgets](/home/nick/.onevcs/workspaces/github.com-nickderobertis-onetaskgraph-9c94c67fcf19/pool/1/worktree/scripts/spikes/section-links/local/target-anchor-id.md#user-content-budgets-1) — must land on `## Budgets (the second heading with this text, at the end)`

## Links to the anchor-name target

Target: `./target-anchor-name.md` relative to this file; forms A and B name it by its absolute path `/home/nick/.onevcs/workspaces/github.com-nickderobertis-onetaskgraph-9c94c67fcf19/pool/1/worktree/scripts/spikes/section-links/local/target-anchor-name.md`

Form `#<slug>`:

- [LM-N-S1 What](./target-anchor-name.md#what) — must land on `## What`
- [LM-N-S2 Budgets](./target-anchor-name.md#budgets) — must land on `## Budgets`
- [LM-N-S3 Budget demo-quota-seconds](./target-anchor-name.md#budget-demo-quota-seconds) — must land on `### Budget demo-quota-seconds`
- [LM-N-S4 Step 2.b: retry (once)](./target-anchor-name.md#step-2b-retry-once) — must land on `### Step 2.b: retry (once)`
- [LM-N-S5 Budgets](./target-anchor-name.md#budgets-1) — must land on `## Budgets (the second heading with this text, at the end)`

Form `#user-content-<slug>`:

- [LM-N-U1 What](./target-anchor-name.md#user-content-what) — must land on `## What`
- [LM-N-U2 Budgets](./target-anchor-name.md#user-content-budgets) — must land on `## Budgets`
- [LM-N-U3 Budget demo-quota-seconds](./target-anchor-name.md#user-content-budget-demo-quota-seconds) — must land on `### Budget demo-quota-seconds`
- [LM-N-U4 Step 2.b: retry (once)](./target-anchor-name.md#user-content-step-2b-retry-once) — must land on `### Step 2.b: retry (once)`
- [LM-N-U5 Budgets](./target-anchor-name.md#user-content-budgets-1) — must land on `## Budgets (the second heading with this text, at the end)`

Form `<absolute path>#<slug>`:

- [LM-N-A1 What](/home/nick/.onevcs/workspaces/github.com-nickderobertis-onetaskgraph-9c94c67fcf19/pool/1/worktree/scripts/spikes/section-links/local/target-anchor-name.md#what) — must land on `## What`
- [LM-N-A2 Budgets](/home/nick/.onevcs/workspaces/github.com-nickderobertis-onetaskgraph-9c94c67fcf19/pool/1/worktree/scripts/spikes/section-links/local/target-anchor-name.md#budgets) — must land on `## Budgets`
- [LM-N-A3 Budget demo-quota-seconds](/home/nick/.onevcs/workspaces/github.com-nickderobertis-onetaskgraph-9c94c67fcf19/pool/1/worktree/scripts/spikes/section-links/local/target-anchor-name.md#budget-demo-quota-seconds) — must land on `### Budget demo-quota-seconds`
- [LM-N-A4 Step 2.b: retry (once)](/home/nick/.onevcs/workspaces/github.com-nickderobertis-onetaskgraph-9c94c67fcf19/pool/1/worktree/scripts/spikes/section-links/local/target-anchor-name.md#step-2b-retry-once) — must land on `### Step 2.b: retry (once)`
- [LM-N-A5 Budgets](/home/nick/.onevcs/workspaces/github.com-nickderobertis-onetaskgraph-9c94c67fcf19/pool/1/worktree/scripts/spikes/section-links/local/target-anchor-name.md#budgets-1) — must land on `## Budgets (the second heading with this text, at the end)`

Form `<absolute path>#user-content-<slug>`:

- [LM-N-B1 What](/home/nick/.onevcs/workspaces/github.com-nickderobertis-onetaskgraph-9c94c67fcf19/pool/1/worktree/scripts/spikes/section-links/local/target-anchor-name.md#user-content-what) — must land on `## What`
- [LM-N-B2 Budgets](/home/nick/.onevcs/workspaces/github.com-nickderobertis-onetaskgraph-9c94c67fcf19/pool/1/worktree/scripts/spikes/section-links/local/target-anchor-name.md#user-content-budgets) — must land on `## Budgets`
- [LM-N-B3 Budget demo-quota-seconds](/home/nick/.onevcs/workspaces/github.com-nickderobertis-onetaskgraph-9c94c67fcf19/pool/1/worktree/scripts/spikes/section-links/local/target-anchor-name.md#user-content-budget-demo-quota-seconds) — must land on `### Budget demo-quota-seconds`
- [LM-N-B4 Step 2.b: retry (once)](/home/nick/.onevcs/workspaces/github.com-nickderobertis-onetaskgraph-9c94c67fcf19/pool/1/worktree/scripts/spikes/section-links/local/target-anchor-name.md#user-content-step-2b-retry-once) — must land on `### Step 2.b: retry (once)`
- [LM-N-B5 Budgets](/home/nick/.onevcs/workspaces/github.com-nickderobertis-onetaskgraph-9c94c67fcf19/pool/1/worktree/scripts/spikes/section-links/local/target-anchor-name.md#user-content-budgets-1) — must land on `## Budgets (the second heading with this text, at the end)`
