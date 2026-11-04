# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [1.0.0](https://github.com/sonirico/naps/compare/v0.2.1...v1.0.0) - 2026-11-04

### Added

- *(deploy)* Helm chart and kind end-to-end test
- *(processor)* TextEncoder and TextDecoder in the JS runtime
- *(ops)* healthz, readyz, Prometheus metrics and graceful drain
- *(pipeline)* JetStream source with ack after publish and JetStream destination
- *(pipeline)* ready signal once a route is subscribed
- *(pipeline)* core NATS relay with processors and queue groups
- *(processor)* WASM processor via Extism
- *(processor)* sandboxed Luau processor
- *(processor)* QuickJS processor for TypeScript and JavaScript
- *(connect)* TLS, creds, nkey and token auth from files
- *(config)* naps.yaml domain with validation

### Fixed

- *(processor)* keep the JS exception type in processing errors
- *(main)* install the rustls crypto provider explicitly

### Other

- animated architecture diagram in the README
- badges, install options and release flow in README
- *(cargo)* keep only sources, examples and docs in the crate package
- *(processor)* assert the import failure cause
- naps 1.0 README and examples
- distroless container image
- iter tests-quality 1: Poll metrics instead of racing counter in http tests
- *(pipeline)* pass a route context instead of long parameter lists
- iter tests-quality 1: Extract stop_test_pipeline helper in jetstream tests
- *(processor)* assert Luau sandbox makes globals read-only
- iter tests-quality 1: Extract subscribe/publish helpers in pipeline_core tests
- iter tests-quality 1: Split test_connect into one test per scenario
- [**breaking**] drop deno and legacy loops, reset toolchain for 1.0
- re-arrange example
