## Why

Repeated embedding generation in `tn-bp30-sdd` may discard work previously completed. Read-only inspection on 2026-09-05 found 17,981 of 18,357 indexed file records updated that day, including sampled source files with July modification times. Pending vectors were actively being generated. Neither the previous complete vector count nor the triggering invalidation was captured, so the root cause remains unproven. Existing logs cannot reliably distinguish a legitimate content/context change, migration, cache replacement, read failure, and restart resume.

## What Changes

- Add structured tracing of index startup identity, reindex decisions, destructive vector mutations, migrations, and embedding completion/resume.
- Correlate intent and committed outcome with daemon and operation identities; report actual existing vectors removed or invalidated, not merely affected chunks.
- Aggregate normal-level events with bounded file examples; reserve per-file details for DEBUG.
- Persist a scoped diagnostic journal with bounded rotation across daemon restarts and machine reboots.
- Document collection and interpretation, and verify that instrumentation preserves indexing and lease behavior.

## Capabilities

### New Capabilities

- `search-vector-invalidation-observability`: Trace the origin and committed effects of vector invalidation across process lifetimes.

### Modified Capabilities

None.

## Impact

Primary code: `crates/bsl-search/src/{store,engine}.rs`, `crates/mcp-server/src/state/{bootstrap,embed,sync}.rs`, `crates/mcp-server/src/graph/build.rs`, and the existing tracing/broker log setup. The source-backed writer inventory and requirement-level verification mapping are in verification.md. CLI logging/tracing owns the private persistent sink; vector_persist owns artifact reason reporting.

No search algorithm, embedding model, public MCP contract, database schema, or invalidation policy change is intended. No production restart, installation, full corpus reindex, commit, or push is authorized by this planning change.

The atomic result is a local diagnostic attribution trail plus deterministic fixture proof. Exact storage, event, counting, failure and handoff decisions are locked in design.md; implementation does not wait for further inventory/design approval or PR #119 publication. No application schema migration is introduced.
