## Why

Embedding work is grouped by document count, while transports can reject the serialized byte size. A downstream build configured for 256 documents and a 1 MiB request limit produced incomplete embeddings; inspected text/context batches exceeded that ceiling. Exact provider errors were not retained, so this does not prove that all missing vectors shared one cause. Current native paths can also lose a batch failure before MCP reports the outcome.

## What Changes

Deliver one atomic outcome: every native embedding work set is sent as count- and byte-bounded requests, and work that cannot complete retains a safe machine-readable failure instead of being advertised as complete.

- Use the actual request serializer for a shared linear batch planner and final transport guard, including UTF-8, JSON escaping, model, dimensions and provider routing.
- Preserve existing caller-owned concurrency, cancellation/checkpoints, input/vector mapping, publication fences and per-file/reference atomicity. Never truncate or silently drop an oversized input.
- Carry typed embedding failure through existing native and MCP owners; keep lexical fallback available, retain already committed vectors and clear diagnostics on the appropriate successful/new attempt.
- Update the affected search/status schemas, discovery fingerprints, budget accounting and documentation together.

## Capabilities

### New Capabilities

- `bounded-embedding-payloads`: Exact embedding request budgets and safe failure outcomes.

### Modified Capabilities

None; no canonical capability spec for this surface exists in this checkout.

## Scope and prerequisites

This change owns native configuration/planning/transport and all nine caller families, lost-error repair, existing MCP lifecycle/response integration, deterministic local HTTP/Store fixtures and verification. Exact read-only source input: `fa4693c0c0936b74ebb6f686a76f2eb583f5b899`, with completed successful CI run `34578618315` rechecked on 2026-09-11. No future predecessor repair, merge, requalification or publication is an implementation prerequisite. The separate structured-indexing change is not required or modified.

No new dependency, endpoint, executor, retry scheduler, observer, durable log, schema migration, document identity change, customer rebuild, installation or release. Implementation progress and evidence for this scope are tracked in tasks.md and verification.md. No archive, commit or push.

Architecture readiness is tracked in design.md. D1 is accepted: default 1,048,576 bytes, positive EMBEDDING_MAX_REQUEST_BYTES override, and safe configuration error for zero, malformed or overflowing values; all other handoff decisions are derived from current source and the requirements below.
