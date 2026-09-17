# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.2.34](https://github.com/nickderobertis/onetaskgraph/compare/onetaskgraph-core-v0.2.33...onetaskgraph-core-v0.2.34) - 2026-09-16

### Added

- *(metadata)* add task, document and project metadata set verbs ([#1314](https://github.com/nickderobertis/onetaskgraph/pull/1314))

## [0.2.33](https://github.com/nickderobertis/onetaskgraph/compare/onetaskgraph-core-v0.2.32...onetaskgraph-core-v0.2.33) - 2026-09-16

### Fixed

- *(local-md)* read the canonical in-progress word and resolve a file-relative root ([#1240](https://github.com/nickderobertis/onetaskgraph/pull/1240))

## [0.2.32](https://github.com/nickderobertis/onetaskgraph/compare/onetaskgraph-core-v0.2.31...onetaskgraph-core-v0.2.32) - 2026-09-15

### Added

- *(status)* add a queued category, a status-only write, and a delivers relation ([#1145](https://github.com/nickderobertis/onetaskgraph/pull/1145))

## [0.2.31](https://github.com/nickderobertis/onetaskgraph/compare/onetaskgraph-core-v0.2.30...onetaskgraph-core-v0.2.31) - 2026-09-14

### Added

- *(cli)* comment on a task with task comment add, list, edit and delete across every plugin ([#1073](https://github.com/nickderobertis/onetaskgraph/pull/1073))

## [0.2.30](https://github.com/nickderobertis/onetaskgraph/compare/onetaskgraph-core-v0.2.29...onetaskgraph-core-v0.2.30) - 2026-09-14

### Added

- *(copy)* copy only a project's named members, and report what it spent ([#904](https://github.com/nickderobertis/onetaskgraph/pull/904))

## [0.2.29](https://github.com/nickderobertis/onetaskgraph/compare/onetaskgraph-core-v0.2.28...onetaskgraph-core-v0.2.29) - 2026-09-14

### Added

- *(cli)* report a failed verb as a JSON document naming its retry class ([#903](https://github.com/nickderobertis/onetaskgraph/pull/903))

## [0.2.27](https://github.com/nickderobertis/onetaskgraph/compare/onetaskgraph-core-v0.2.26...onetaskgraph-core-v0.2.27) - 2026-09-07

### Added

- *(copy)* point a copied document's references at the destination's own records ([#774](https://github.com/nickderobertis/onetaskgraph/pull/774))

## [0.2.23](https://github.com/nickderobertis/onetaskgraph/compare/onetaskgraph-core-v0.2.22...onetaskgraph-core-v0.2.23) - 2026-09-05

### Fixed

- *(copy)* guard the copy path's pagination loops against a repeated cursor ([#397](https://github.com/nickderobertis/onetaskgraph/pull/397))

## [0.2.22](https://github.com/nickderobertis/onetaskgraph/compare/onetaskgraph-core-v0.2.21...onetaskgraph-core-v0.2.22) - 2026-09-04

### Added

- *(github-projects)* account for what the live tests spend, reduce it, and refuse a run the account cannot afford ([#280](https://github.com/nickderobertis/onetaskgraph/pull/280))

## [0.2.21](https://github.com/nickderobertis/onetaskgraph/compare/onetaskgraph-core-v0.2.20...onetaskgraph-core-v0.2.21) - 2026-09-02

### Fixed

- *(copy)* keep the destination's own origin when a copy comes back to it ([#252](https://github.com/nickderobertis/onetaskgraph/pull/252))

## [0.2.18](https://github.com/nickderobertis/onetaskgraph/compare/onetaskgraph-core-v0.2.17...onetaskgraph-core-v0.2.18) - 2026-09-01

### Fixed

- *(github-projects)* read GitHub's secondary rate limiter as one, and stop the copy outrunning it ([#173](https://github.com/nickderobertis/onetaskgraph/pull/173))

## [0.2.17](https://github.com/nickderobertis/onetaskgraph/compare/onetaskgraph-core-v0.2.16...onetaskgraph-core-v0.2.17) - 2026-09-01

### Added

- *(github-projects)* hold documents as design-titled issues and report their locations ([#161](https://github.com/nickderobertis/onetaskgraph/pull/161))

## [0.2.15](https://github.com/nickderobertis/onetaskgraph/compare/onetaskgraph-core-v0.2.14...onetaskgraph-core-v0.2.15) - 2026-09-01

### Added

- *(in-memory)* serve documents and both location shapes ([#138](https://github.com/nickderobertis/onetaskgraph/pull/138))

## [0.2.14](https://github.com/nickderobertis/onetaskgraph/compare/onetaskgraph-core-v0.2.13...onetaskgraph-core-v0.2.14) - 2026-09-01

### Added

- *(plugin-api)* give the source contract documents and locations ([#114](https://github.com/nickderobertis/onetaskgraph/pull/114))

## [0.2.13](https://github.com/nickderobertis/onetaskgraph/compare/onetaskgraph-core-v0.2.12...onetaskgraph-core-v0.2.13) - 2026-08-30

### Fixed

- make project copy atomic, publish the npm package, and provision the pre-push gate ([#87](https://github.com/nickderobertis/onetaskgraph/pull/87))

## [0.2.12](https://github.com/nickderobertis/onetaskgraph/compare/onetaskgraph-core-v0.2.11...onetaskgraph-core-v0.2.12) - 2026-08-29

### Fixed

- *(github-projects)* apply every predicate a query carries and declare it ([#65](https://github.com/nickderobertis/onetaskgraph/pull/65))

## [0.2.3](https://github.com/nickderobertis/onetaskgraph/compare/onetaskgraph-core-v0.2.1...onetaskgraph-core-v0.2.3) - 2026-08-27

### Documentation

- present the Rust SDK and the Markdown authoring flow as first-class ([#37](https://github.com/nickderobertis/onetaskgraph/pull/37))

### Fixed

- *(core)* make the crate packageable with its README doctest intact ([#42](https://github.com/nickderobertis/onetaskgraph/pull/42))

## [0.2.2](https://github.com/nickderobertis/onetaskgraph/compare/onetaskgraph-core-v0.2.1...onetaskgraph-core-v0.2.2) - 2026-08-27

### Fixed

- *(core)* make the crate packageable with its README doctest intact ([#42](https://github.com/nickderobertis/onetaskgraph/pull/42))

## [0.2.0](https://github.com/nickderobertis/onetaskgraph/compare/onetaskgraph-core-v0.1.0...onetaskgraph-core-v0.2.0) - 2026-08-26

### Added

- *(copy)* add the copy verb and the plugin write seam across the engine, CLI, and both SDKs ([#29](https://github.com/nickderobertis/onetaskgraph/pull/29))
- *(api)* [**breaking**] carry custom metadata, repositories, and edges that leave the project ([#25](https://github.com/nickderobertis/onetaskgraph/pull/25))

## [0.1.0](https://github.com/nickderobertis/onetaskgraph/releases/tag/onetaskgraph-core-v0.1.0) - 2026-08-25

### Added

- *(release)* automate versioning, publication and the proven end-user install path ([#15](https://github.com/nickderobertis/onetaskgraph/pull/15))
- *(github-projects)* draft the GitHub Projects source against its real v2 GraphQL shapes ([#14](https://github.com/nickderobertis/onetaskgraph/pull/14))
- *(linear)* draft the Linear source against its real GraphQL shapes, without live credentials ([#10](https://github.com/nickderobertis/onetaskgraph/pull/10))
- *(npm-sdk)* generate a typed TypeScript client that drives the real binary ([#8](https://github.com/nickderobertis/onetaskgraph/pull/8))
- *(core)* implement the stdio plugin protocol so an out-of-tree plugin needs no C ABI ([#6](https://github.com/nickderobertis/onetaskgraph/pull/6))
- *(local-md)* read tasks and projects from a folder of Markdown files ([#5](https://github.com/nickderobertis/onetaskgraph/pull/5))
- *(engine)* fan queries out across sources with capability-aware pushdown and a visible plan ([#4](https://github.com/nickderobertis/onetaskgraph/pull/4))
- *(config)* layer file, environment and flag configuration over a named-source registry ([#3](https://github.com/nickderobertis/onetaskgraph/pull/3))
- establish the onetaskgraph workspace, its gate, its CI and its plugin contract ([#1](https://github.com/nickderobertis/onetaskgraph/pull/1))
