# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.3.3](https://github.com/nickderobertis/onetaskgraph/compare/onetaskgraph-local-md-v0.3.2...onetaskgraph-local-md-v0.3.3) - 2026-10-07

### Added

- *(assets)* carry image assets with tasks and documents across copies ([#3392](https://github.com/nickderobertis/onetaskgraph/pull/3392))

## [0.3.2](https://github.com/nickderobertis/onetaskgraph/compare/onetaskgraph-local-md-v0.3.1...onetaskgraph-local-md-v0.3.2) - 2026-10-06

### Added

- *(projects)* render a project's description from a template, with provenance and stored answers ([#3334](https://github.com/nickderobertis/onetaskgraph/pull/3334))

## [0.2.52](https://github.com/nickderobertis/onetaskgraph/compare/onetaskgraph-local-md-v0.2.51...onetaskgraph-local-md-v0.2.52) - 2026-09-29

### Added

- *(github-projects)* answer text, metadata and origin queries with GitHub's own search ([#2983](https://github.com/nickderobertis/onetaskgraph/pull/2983))

## [0.2.50](https://github.com/nickderobertis/onetaskgraph/compare/onetaskgraph-local-md-v0.2.49...onetaskgraph-local-md-v0.2.50) - 2026-09-29

### Added

- *(query)* filter tasks by comment activity with --commented-since ([#2916](https://github.com/nickderobertis/onetaskgraph/pull/2916))

## [0.2.49](https://github.com/nickderobertis/onetaskgraph/compare/onetaskgraph-local-md-v0.2.48...onetaskgraph-local-md-v0.2.49) - 2026-09-28

### Fixed

- *(local-md)* update a task whose metadata holds block-scalar sequences ([#2887](https://github.com/nickderobertis/onetaskgraph/pull/2887))

## [0.2.48](https://github.com/nickderobertis/onetaskgraph/compare/onetaskgraph-local-md-v0.2.47...onetaskgraph-local-md-v0.2.48) - 2026-09-28

### Added

- *(task)* add a targeted task update and classify budget exhaustion ([#2805](https://github.com/nickderobertis/onetaskgraph/pull/2805))

### Fixed

- *(local-md)* keep an item's content byte for byte on write, so a copied rendering still matches its digest ([#2812](https://github.com/nickderobertis/onetaskgraph/pull/2812))

## [0.2.47](https://github.com/nickderobertis/onetaskgraph/compare/onetaskgraph-local-md-v0.2.46...onetaskgraph-local-md-v0.2.47) - 2026-09-27

### Added

- *(templates)* create and regenerate items from templates, keeping answers out of the body ([#2716](https://github.com/nickderobertis/onetaskgraph/pull/2716))

## [0.2.45](https://github.com/nickderobertis/onetaskgraph/compare/onetaskgraph-local-md-v0.2.44...onetaskgraph-local-md-v0.2.45) - 2026-09-26

### Added

- *(task)* add a first-class task priority and a field-setup verb ([#2613](https://github.com/nickderobertis/onetaskgraph/pull/2613))

## [0.2.44](https://github.com/nickderobertis/onetaskgraph/compare/onetaskgraph-local-md-v0.2.43...onetaskgraph-local-md-v0.2.44) - 2026-09-26

### Added

- *(task)* report the backend's short handle as a task's key ([#2585](https://github.com/nickderobertis/onetaskgraph/pull/2585))

### Fixed

- *(local-md)* skip a task file pending deletion on Windows as vanished, not malformed ([#2387](https://github.com/nickderobertis/onetaskgraph/pull/2387))

## [0.2.41](https://github.com/nickderobertis/onetaskgraph/compare/onetaskgraph-local-md-v0.2.40...onetaskgraph-local-md-v0.2.41) - 2026-09-20

### Fixed

- *(local-md)* atomic status writes, listing stable across a replacement, and a project-scoped walk ([#2128](https://github.com/nickderobertis/onetaskgraph/pull/2128))

## [0.2.35](https://github.com/nickderobertis/onetaskgraph/compare/onetaskgraph-local-md-v0.2.34...onetaskgraph-local-md-v0.2.35) - 2026-09-17

### Fixed

- *(local-md)* fail a listing that meets a malformed record ([#1472](https://github.com/nickderobertis/onetaskgraph/pull/1472))

## [0.2.34](https://github.com/nickderobertis/onetaskgraph/compare/onetaskgraph-local-md-v0.2.33...onetaskgraph-local-md-v0.2.34) - 2026-09-16

### Added

- *(metadata)* add task, document and project metadata set verbs ([#1314](https://github.com/nickderobertis/onetaskgraph/pull/1314))

## [0.2.33](https://github.com/nickderobertis/onetaskgraph/compare/onetaskgraph-local-md-v0.2.32...onetaskgraph-local-md-v0.2.33) - 2026-09-16

### Fixed

- *(local-md)* read the canonical in-progress word and resolve a file-relative root ([#1240](https://github.com/nickderobertis/onetaskgraph/pull/1240))

## [0.2.32](https://github.com/nickderobertis/onetaskgraph/compare/onetaskgraph-local-md-v0.2.31...onetaskgraph-local-md-v0.2.32) - 2026-09-15

### Added

- *(status)* add a queued category, a status-only write, and a delivers relation ([#1145](https://github.com/nickderobertis/onetaskgraph/pull/1145))

## [0.2.31](https://github.com/nickderobertis/onetaskgraph/compare/onetaskgraph-local-md-v0.2.30...onetaskgraph-local-md-v0.2.31) - 2026-09-14

### Added

- *(cli)* comment on a task with task comment add, list, edit and delete across every plugin ([#1073](https://github.com/nickderobertis/onetaskgraph/pull/1073))

## [0.2.22](https://github.com/nickderobertis/onetaskgraph/compare/onetaskgraph-local-md-v0.2.21...onetaskgraph-local-md-v0.2.22) - 2026-09-04

### Added

- *(github-projects)* account for what the live tests spend, reduce it, and refuse a run the account cannot afford ([#280](https://github.com/nickderobertis/onetaskgraph/pull/280))

## [0.2.16](https://github.com/nickderobertis/onetaskgraph/compare/onetaskgraph-local-md-v0.2.15...onetaskgraph-local-md-v0.2.16) - 2026-09-01

### Added

- *(local-md)* hold documents in their own folder and report file locations ([#155](https://github.com/nickderobertis/onetaskgraph/pull/155))

## [0.2.14](https://github.com/nickderobertis/onetaskgraph/compare/onetaskgraph-local-md-v0.2.13...onetaskgraph-local-md-v0.2.14) - 2026-09-01

### Added

- *(plugin-api)* give the source contract documents and locations ([#114](https://github.com/nickderobertis/onetaskgraph/pull/114))

## [0.2.13](https://github.com/nickderobertis/onetaskgraph/compare/onetaskgraph-local-md-v0.2.12...onetaskgraph-local-md-v0.2.13) - 2026-08-30

### Fixed

- make project copy atomic, publish the npm package, and provision the pre-push gate ([#87](https://github.com/nickderobertis/onetaskgraph/pull/87))

## [0.2.12](https://github.com/nickderobertis/onetaskgraph/compare/onetaskgraph-local-md-v0.2.11...onetaskgraph-local-md-v0.2.12) - 2026-08-29

### Fixed

- *(github-projects)* apply every predicate a query carries and declare it ([#65](https://github.com/nickderobertis/onetaskgraph/pull/65))

## [0.2.9](https://github.com/nickderobertis/onetaskgraph/compare/onetaskgraph-local-md-v0.2.8...onetaskgraph-local-md-v0.2.9) - 2026-08-28

### Added

- add draft to the status vocabulary and default local-md to backlog ([#54](https://github.com/nickderobertis/onetaskgraph/pull/54))

## [0.2.0](https://github.com/nickderobertis/onetaskgraph/compare/onetaskgraph-local-md-v0.1.0...onetaskgraph-local-md-v0.2.0) - 2026-08-26

### Added

- *(copy)* add the copy verb and the plugin write seam across the engine, CLI, and both SDKs ([#29](https://github.com/nickderobertis/onetaskgraph/pull/29))
- *(api)* [**breaking**] carry custom metadata, repositories, and edges that leave the project ([#25](https://github.com/nickderobertis/onetaskgraph/pull/25))

## [0.1.0](https://github.com/nickderobertis/onetaskgraph/releases/tag/onetaskgraph-local-md-v0.1.0) - 2026-08-25

### Added

- *(release)* automate versioning, publication and the proven end-user install path ([#15](https://github.com/nickderobertis/onetaskgraph/pull/15))
- *(python-sdk)* generate a typed Python client that drives the real binary ([#7](https://github.com/nickderobertis/onetaskgraph/pull/7))
- *(local-md)* read tasks and projects from a folder of Markdown files ([#5](https://github.com/nickderobertis/onetaskgraph/pull/5))
- establish the onetaskgraph workspace, its gate, its CI and its plugin contract ([#1](https://github.com/nickderobertis/onetaskgraph/pull/1))
