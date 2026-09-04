# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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
