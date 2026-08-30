# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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
