# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.2.18](https://github.com/nickderobertis/onetaskgraph/compare/onetaskgraph-plugin-api-v0.2.17...onetaskgraph-plugin-api-v0.2.18) - 2026-09-01

### Fixed

- *(github-projects)* read GitHub's secondary rate limiter as one, and stop the copy outrunning it ([#173](https://github.com/nickderobertis/onetaskgraph/pull/173))

## [0.2.14](https://github.com/nickderobertis/onetaskgraph/compare/onetaskgraph-plugin-api-v0.2.13...onetaskgraph-plugin-api-v0.2.14) - 2026-09-01

### Added

- *(plugin-api)* give the source contract documents and locations ([#114](https://github.com/nickderobertis/onetaskgraph/pull/114))

## [0.2.13](https://github.com/nickderobertis/onetaskgraph/compare/onetaskgraph-plugin-api-v0.2.12...onetaskgraph-plugin-api-v0.2.13) - 2026-08-30

### Fixed

- make project copy atomic, publish the npm package, and provision the pre-push gate ([#87](https://github.com/nickderobertis/onetaskgraph/pull/87))

## [0.2.9](https://github.com/nickderobertis/onetaskgraph/compare/onetaskgraph-plugin-api-v0.2.8...onetaskgraph-plugin-api-v0.2.9) - 2026-08-28

### Added

- *(github-projects)* store many projects on one board as issues and their sub-issues ([#56](https://github.com/nickderobertis/onetaskgraph/pull/56))
- add draft to the status vocabulary and default local-md to backlog ([#54](https://github.com/nickderobertis/onetaskgraph/pull/54))

## [0.3.0](https://github.com/nickderobertis/onetaskgraph/compare/onetaskgraph-plugin-api-v0.2.8...onetaskgraph-plugin-api-v0.3.0) - 2026-08-28

### Added

- *(github-projects)* store many projects on one board as issues and their sub-issues ([#56](https://github.com/nickderobertis/onetaskgraph/pull/56))
- add draft to the status vocabulary and default local-md to backlog ([#54](https://github.com/nickderobertis/onetaskgraph/pull/54))

## [0.2.0](https://github.com/nickderobertis/onetaskgraph/compare/onetaskgraph-plugin-api-v0.1.0...onetaskgraph-plugin-api-v0.2.0) - 2026-08-26

### Added

- *(copy)* add the copy verb and the plugin write seam across the engine, CLI, and both SDKs ([#29](https://github.com/nickderobertis/onetaskgraph/pull/29))
- *(api)* [**breaking**] carry custom metadata, repositories, and edges that leave the project ([#25](https://github.com/nickderobertis/onetaskgraph/pull/25))

## [0.1.0](https://github.com/nickderobertis/onetaskgraph/releases/tag/onetaskgraph-plugin-api-v0.1.0) - 2026-08-25

### Added

- *(release)* automate versioning, publication and the proven end-user install path ([#15](https://github.com/nickderobertis/onetaskgraph/pull/15))
- establish the onetaskgraph workspace, its gate, its CI and its plugin contract ([#1](https://github.com/nickderobertis/onetaskgraph/pull/1))
