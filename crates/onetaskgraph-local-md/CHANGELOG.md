# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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
