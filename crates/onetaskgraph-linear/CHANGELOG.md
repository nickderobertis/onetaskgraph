# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.3.2](https://github.com/nickderobertis/onetaskgraph/compare/onetaskgraph-linear-v0.3.1...onetaskgraph-linear-v0.3.2) - 2026-10-06

### Added

- *(projects)* render a project's description from a template, with provenance and stored answers ([#3334](https://github.com/nickderobertis/onetaskgraph/pull/3334))

## [0.3.1](https://github.com/nickderobertis/onetaskgraph/compare/onetaskgraph-linear-v0.3.0...onetaskgraph-linear-v0.3.1) - 2026-10-06

### Added

- *(engine)* let a long-lived Engine end a command so sources drop what a person can change ([#3307](https://github.com/nickderobertis/onetaskgraph/pull/3307))

## [0.3.0](https://github.com/nickderobertis/onetaskgraph/compare/onetaskgraph-linear-v0.2.58...onetaskgraph-linear-v0.3.0) - 2026-10-06

### Added

- *(status)* [**breaking**] scope status_mapping by item kind, refuse unmapped statuses, and drop Linear's by-type fallback ([#3262](https://github.com/nickderobertis/onetaskgraph/pull/3262))

## [0.2.56](https://github.com/nickderobertis/onetaskgraph/compare/onetaskgraph-linear-v0.2.55...onetaskgraph-linear-v0.2.56) - 2026-10-02

### Added

- *(linear)* bring the Linear source to parity with GitHub Projects ([#3160](https://github.com/nickderobertis/onetaskgraph/pull/3160))

## [0.2.52](https://github.com/nickderobertis/onetaskgraph/compare/onetaskgraph-linear-v0.2.51...onetaskgraph-linear-v0.2.52) - 2026-09-29

### Added

- *(github-projects)* answer text, metadata and origin queries with GitHub's own search ([#2983](https://github.com/nickderobertis/onetaskgraph/pull/2983))

## [0.2.50](https://github.com/nickderobertis/onetaskgraph/compare/onetaskgraph-linear-v0.2.49...onetaskgraph-linear-v0.2.50) - 2026-09-29

### Added

- *(query)* filter tasks by comment activity with --commented-since ([#2916](https://github.com/nickderobertis/onetaskgraph/pull/2916))

## [0.2.48](https://github.com/nickderobertis/onetaskgraph/compare/onetaskgraph-linear-v0.2.47...onetaskgraph-linear-v0.2.48) - 2026-09-28

### Added

- *(task)* add a targeted task update and classify budget exhaustion ([#2805](https://github.com/nickderobertis/onetaskgraph/pull/2805))

## [0.2.45](https://github.com/nickderobertis/onetaskgraph/compare/onetaskgraph-linear-v0.2.44...onetaskgraph-linear-v0.2.45) - 2026-09-26

### Added

- *(task)* add a first-class task priority and a field-setup verb ([#2613](https://github.com/nickderobertis/onetaskgraph/pull/2613))

## [0.2.44](https://github.com/nickderobertis/onetaskgraph/compare/onetaskgraph-linear-v0.2.43...onetaskgraph-linear-v0.2.44) - 2026-09-26

### Added

- *(task)* report the backend's short handle as a task's key ([#2585](https://github.com/nickderobertis/onetaskgraph/pull/2585))

## [0.2.35](https://github.com/nickderobertis/onetaskgraph/compare/onetaskgraph-linear-v0.2.34...onetaskgraph-linear-v0.2.35) - 2026-09-17

### Fixed

- *(linear)* page the live journey's document reads to exhaustion ([#1508](https://github.com/nickderobertis/onetaskgraph/pull/1508))

## [0.2.34](https://github.com/nickderobertis/onetaskgraph/compare/onetaskgraph-linear-v0.2.33...onetaskgraph-linear-v0.2.34) - 2026-09-16

### Added

- *(metadata)* add task, document and project metadata set verbs ([#1314](https://github.com/nickderobertis/onetaskgraph/pull/1314))

### Fixed

- *(linear)* tell a missing label apart from a duplicated one ([#1436](https://github.com/nickderobertis/onetaskgraph/pull/1436))
- *(linear)* wait for deleted documents to become unreadable ([#1383](https://github.com/nickderobertis/onetaskgraph/pull/1383))

## [0.2.32](https://github.com/nickderobertis/onetaskgraph/compare/onetaskgraph-linear-v0.2.31...onetaskgraph-linear-v0.2.32) - 2026-09-15

### Added

- *(status)* add a queued category, a status-only write, and a delivers relation ([#1145](https://github.com/nickderobertis/onetaskgraph/pull/1145))

## [0.2.31](https://github.com/nickderobertis/onetaskgraph/compare/onetaskgraph-linear-v0.2.30...onetaskgraph-linear-v0.2.31) - 2026-09-14

### Added

- *(cli)* comment on a task with task comment add, list, edit and delete across every plugin ([#1073](https://github.com/nickderobertis/onetaskgraph/pull/1073))

## [0.2.24](https://github.com/nickderobertis/onetaskgraph/compare/onetaskgraph-linear-v0.2.23...onetaskgraph-linear-v0.2.24) - 2026-09-06

### Fixed

- *(live)* stop the startup sweep touching a run in flight, and land it past this repository's own merge path ([#538](https://github.com/nickderobertis/onetaskgraph/pull/538))

## [0.2.22](https://github.com/nickderobertis/onetaskgraph/compare/onetaskgraph-linear-v0.2.21...onetaskgraph-linear-v0.2.22) - 2026-09-04

### Added

- *(github-projects)* account for what the live tests spend, reduce it, and refuse a run the account cannot afford ([#280](https://github.com/nickderobertis/onetaskgraph/pull/280))

## [0.2.18](https://github.com/nickderobertis/onetaskgraph/compare/onetaskgraph-linear-v0.2.17...onetaskgraph-linear-v0.2.18) - 2026-09-01

### Added

- *(linear)* hold documents as native Linear documents and report their locations ([#201](https://github.com/nickderobertis/onetaskgraph/pull/201))

### Fixed

- *(github-projects)* read GitHub's secondary rate limiter as one, and stop the copy outrunning it ([#173](https://github.com/nickderobertis/onetaskgraph/pull/173))

## [0.2.14](https://github.com/nickderobertis/onetaskgraph/compare/onetaskgraph-linear-v0.2.13...onetaskgraph-linear-v0.2.14) - 2026-09-01

### Added

- *(plugin-api)* give the source contract documents and locations ([#114](https://github.com/nickderobertis/onetaskgraph/pull/114))

## [0.2.13](https://github.com/nickderobertis/onetaskgraph/compare/onetaskgraph-linear-v0.2.12...onetaskgraph-linear-v0.2.13) - 2026-08-30

### Fixed

- make project copy atomic, publish the npm package, and provision the pre-push gate ([#87](https://github.com/nickderobertis/onetaskgraph/pull/87))

## [0.2.12](https://github.com/nickderobertis/onetaskgraph/compare/onetaskgraph-linear-v0.2.11...onetaskgraph-linear-v0.2.12) - 2026-08-29

### Fixed

- *(github-projects)* apply every predicate a query carries and declare it ([#65](https://github.com/nickderobertis/onetaskgraph/pull/65))

## [0.2.9](https://github.com/nickderobertis/onetaskgraph/compare/onetaskgraph-linear-v0.2.8...onetaskgraph-linear-v0.2.9) - 2026-08-28

### Added

- add draft to the status vocabulary and default local-md to backlog ([#54](https://github.com/nickderobertis/onetaskgraph/pull/54))

## [0.2.0](https://github.com/nickderobertis/onetaskgraph/compare/onetaskgraph-linear-v0.1.0...onetaskgraph-linear-v0.2.0) - 2026-08-26

### Added

- *(linear)* write tasks, projects, metadata and native dependency relations ([#31](https://github.com/nickderobertis/onetaskgraph/pull/31))
- *(api)* [**breaking**] carry custom metadata, repositories, and edges that leave the project ([#25](https://github.com/nickderobertis/onetaskgraph/pull/25))

## [0.1.0](https://github.com/nickderobertis/onetaskgraph/releases/tag/onetaskgraph-linear-v0.1.0) - 2026-08-25

### Added

- *(release)* automate versioning, publication and the proven end-user install path ([#15](https://github.com/nickderobertis/onetaskgraph/pull/15))
- *(linear)* draft the Linear source against its real GraphQL shapes, without live credentials ([#10](https://github.com/nickderobertis/onetaskgraph/pull/10))
- establish the onetaskgraph workspace, its gate, its CI and its plugin contract ([#1](https://github.com/nickderobertis/onetaskgraph/pull/1))
