---
title: 'Placeholder: a task whose labels need quoting'
status: todo
priority: high
labels:
- id: 'needs: quoting'
  name: 'needs: quoting'
- id: 2026-q4
  name: 2026-Q4
  color: '#0e8a16'
- id: yes
  name: yes
  color: fbca04
- id: '1.0'
  name: '1.0'
- id: '#triage'
  name: '#triage'
- id: area/engine
  name: area/engine
  color: 1d76db
- id: '007'
  name: '007'
project: placeholder-project
depends_on:
- id: placeholder-project/plain-labels
  kind: blocks
  item: task
metadata:
  placeholder.id: quoted-labels
  placeholder.list:
  - first placeholder
  - 'second: placeholder'
  - 3
  - true
  - null
  placeholder.review:
    at: 2026-01-02T03:04:05+00:00
    by: placeholder-reviewer
    key: 0123456789abcdef
delivers:
- elsewhere:placeholder-ticket-1
---
## What

Placeholder text describing what the task does.

## Why

Placeholder text describing why.


<!-- onetaskgraph:template-answers
labels:
- 'needs: quoting'
- 2026-Q4
- yes
what: |-
  Placeholder text describing what the task does.

  A second placeholder paragraph.
why: Placeholder text describing why.
-->
