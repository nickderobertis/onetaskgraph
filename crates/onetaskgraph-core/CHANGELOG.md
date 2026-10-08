# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.3.9](https://github.com/nickderobertis/onetaskgraph/compare/onetaskgraph-core-v0.3.8...onetaskgraph-core-v0.3.9) - 2026-10-08

### Added

- *(github-projects)* upload image assets as GitHub user attachments ([#3560](https://github.com/nickderobertis/onetaskgraph/pull/3560))

## [0.3.8](https://github.com/nickderobertis/onetaskgraph/compare/onetaskgraph-core-v0.3.7...onetaskgraph-core-v0.3.8) - 2026-10-08

### Added

- *(linear)* upload image assets through Linear's file upload ([#3561](https://github.com/nickderobertis/onetaskgraph/pull/3561))

## [0.3.7](https://github.com/nickderobertis/onetaskgraph/compare/onetaskgraph-core-v0.3.6...onetaskgraph-core-v0.3.7) - 2026-10-08

### Fixed

- name no real organization's configuration in onetaskgraph's examples, fixtures and tests ([#3551](https://github.com/nickderobertis/onetaskgraph/pull/3551))

## [0.3.6](https://github.com/nickderobertis/onetaskgraph/compare/onetaskgraph-core-v0.3.5...onetaskgraph-core-v0.3.6) - 2026-10-07

### Fixed

- *(copy)* carry a reference's #fragment through a rewrite ([#3518](https://github.com/nickderobertis/onetaskgraph/pull/3518))

## [0.3.3](https://github.com/nickderobertis/onetaskgraph/compare/onetaskgraph-core-v0.3.2...onetaskgraph-core-v0.3.3) - 2026-10-07

### Added

- *(assets)* carry image assets with tasks and documents across copies ([#3392](https://github.com/nickderobertis/onetaskgraph/pull/3392))

## [0.3.2](https://github.com/nickderobertis/onetaskgraph/compare/onetaskgraph-core-v0.3.1...onetaskgraph-core-v0.3.2) - 2026-10-06

### Added

- *(projects)* render a project's description from a template, with provenance and stored answers ([#3334](https://github.com/nickderobertis/onetaskgraph/pull/3334))

## [0.3.1](https://github.com/nickderobertis/onetaskgraph/compare/onetaskgraph-core-v0.3.0...onetaskgraph-core-v0.3.1) - 2026-10-06

### Added

- *(engine)* let a long-lived Engine end a command so sources drop what a person can change ([#3307](https://github.com/nickderobertis/onetaskgraph/pull/3307))

## [0.3.0](https://github.com/nickderobertis/onetaskgraph/compare/onetaskgraph-core-v0.2.58...onetaskgraph-core-v0.3.0) - 2026-10-06

### Added

- *(status)* [**breaking**] scope status_mapping by item kind, refuse unmapped statuses, and drop Linear's by-type fallback ([#3262](https://github.com/nickderobertis/onetaskgraph/pull/3262))

## [0.2.58](https://github.com/nickderobertis/onetaskgraph/compare/onetaskgraph-core-v0.2.57...onetaskgraph-core-v0.2.58) - 2026-10-03

### Performance

- *(github-projects)* search the board for project and document text ([#3231](https://github.com/nickderobertis/onetaskgraph/pull/3231))

## [0.2.56](https://github.com/nickderobertis/onetaskgraph/compare/onetaskgraph-core-v0.2.55...onetaskgraph-core-v0.2.56) - 2026-10-02

### Added

- *(linear)* bring the Linear source to parity with GitHub Projects ([#3160](https://github.com/nickderobertis/onetaskgraph/pull/3160))

## [0.2.55](https://github.com/nickderobertis/onetaskgraph/compare/onetaskgraph-core-v0.2.54...onetaskgraph-core-v0.2.55) - 2026-10-02

### Added

- *(store)* read items in batches and copy without repeating a lookup ([#3105](https://github.com/nickderobertis/onetaskgraph/pull/3105))

## [0.2.53](https://github.com/nickderobertis/onetaskgraph/compare/onetaskgraph-core-v0.2.52...onetaskgraph-core-v0.2.53) - 2026-10-01

### Fixed

- *(copy)* keep a rewritten rendered document's body digest true of the copy ([#3021](https://github.com/nickderobertis/onetaskgraph/pull/3021))

## [0.2.52](https://github.com/nickderobertis/onetaskgraph/compare/onetaskgraph-core-v0.2.51...onetaskgraph-core-v0.2.52) - 2026-09-29

### Added

- *(github-projects)* answer text, metadata and origin queries with GitHub's own search ([#2983](https://github.com/nickderobertis/onetaskgraph/pull/2983))
- *(copy)* record where an item was copied to and follow that link on every later copy ([#2967](https://github.com/nickderobertis/onetaskgraph/pull/2967))

## [0.2.50](https://github.com/nickderobertis/onetaskgraph/compare/onetaskgraph-core-v0.2.49...onetaskgraph-core-v0.2.50) - 2026-09-29

### Added

- *(query)* filter tasks by comment activity with --commented-since ([#2916](https://github.com/nickderobertis/onetaskgraph/pull/2916))

### Fixed

- *(copy)* give a copied project's tasks ids scoped to the destination project ([#2929](https://github.com/nickderobertis/onetaskgraph/pull/2929))

## [0.2.49](https://github.com/nickderobertis/onetaskgraph/compare/onetaskgraph-core-v0.2.48...onetaskgraph-core-v0.2.49) - 2026-09-28

### Fixed

- *(local-md)* update a task whose metadata holds block-scalar sequences ([#2887](https://github.com/nickderobertis/onetaskgraph/pull/2887))

## [0.2.48](https://github.com/nickderobertis/onetaskgraph/compare/onetaskgraph-core-v0.2.47...onetaskgraph-core-v0.2.48) - 2026-09-28

### Added

- *(task)* add a targeted task update and classify budget exhaustion ([#2805](https://github.com/nickderobertis/onetaskgraph/pull/2805))

## [0.2.47](https://github.com/nickderobertis/onetaskgraph/compare/onetaskgraph-core-v0.2.46...onetaskgraph-core-v0.2.47) - 2026-09-27

### Added

- *(templates)* create and regenerate items from templates, keeping answers out of the body ([#2716](https://github.com/nickderobertis/onetaskgraph/pull/2716))

## [0.2.46](https://github.com/nickderobertis/onetaskgraph/compare/onetaskgraph-core-v0.2.45...onetaskgraph-core-v0.2.46) - 2026-09-27

### Added

- *(templates)* render minijinja templates with declared variables and answers ([#2654](https://github.com/nickderobertis/onetaskgraph/pull/2654))
- *(core)* expose a failure's class, kind, source and retry-after through typed accessors ([#2645](https://github.com/nickderobertis/onetaskgraph/pull/2645))

## [0.2.45](https://github.com/nickderobertis/onetaskgraph/compare/onetaskgraph-core-v0.2.44...onetaskgraph-core-v0.2.45) - 2026-09-26

### Added

- *(task)* add a first-class task priority and a field-setup verb ([#2613](https://github.com/nickderobertis/onetaskgraph/pull/2613))

## [0.2.44](https://github.com/nickderobertis/onetaskgraph/compare/onetaskgraph-core-v0.2.43...onetaskgraph-core-v0.2.44) - 2026-09-26

### Added

- *(task)* report the backend's short handle as a task's key ([#2585](https://github.com/nickderobertis/onetaskgraph/pull/2585))

### Documentation

- *(readme)* lead with hash-gated captures of the real CLI ([#2561](https://github.com/nickderobertis/onetaskgraph/pull/2561))

## [0.2.40](https://github.com/nickderobertis/onetaskgraph/compare/onetaskgraph-core-v0.2.39...onetaskgraph-core-v0.2.40) - 2026-09-19

### Fixed

- *(build)* make the network destinations optional core features and pin sibling crates exactly ([#2113](https://github.com/nickderobertis/onetaskgraph/pull/2113))

## [0.2.39](https://github.com/nickderobertis/onetaskgraph/compare/onetaskgraph-core-v0.2.38...onetaskgraph-core-v0.2.39) - 2026-09-18

### Fixed

- *(subprocess)* hand a hosted plugin the directory of the configuration document that named it ([#2023](https://github.com/nickderobertis/onetaskgraph/pull/2023))

## [0.2.37](https://github.com/nickderobertis/onetaskgraph/compare/onetaskgraph-core-v0.2.36...onetaskgraph-core-v0.2.37) - 2026-09-18

### Fixed

- *(github-projects)* write issue state and Status together ([#1791](https://github.com/nickderobertis/onetaskgraph/pull/1791))

## [0.2.36](https://github.com/nickderobertis/onetaskgraph/compare/onetaskgraph-core-v0.2.35...onetaskgraph-core-v0.2.36) - 2026-09-17

### Added

- *(github-projects)* add missing board status options safely ([#1610](https://github.com/nickderobertis/onetaskgraph/pull/1610))

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
