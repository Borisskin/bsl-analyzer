## Why

MCP consumers cannot observe indexing consistently without parsing text. Lexical results can already be served while embeddings are still building; a ready graph, configured provider or successful lexical fallback does not prove semantic coverage.

## What Changes

Deliver one machine-readable indexing contract on existing workspace code/status, reference docs/status (including docs requested through the workspace profile), and graph status/loading responses. Include successful empty results. Report independent target readiness and coherent optional native counters; preserve existing search, fallback, lease and cancellation semantics.

Own the native progress records, readiness qualification, response projection, budget enforcement, discovery/version changes, documentation and verification needed for that single result. Reuse existing local coverage checks and remote publication metadata; polling adds no provider calls, scans or new background work. Unknown evidence remains unknown.

The required strict budget behavior rejects previously accepted undersized requests, so the machine contract advances from `2.2` to `3.0`, rather than an additive minor version. Existing fields retain their meaning. See design.md for exact versions and the error contract.

## Capabilities

### New Capabilities

- `structured-indexing-progress`: One bounded, versioned lifecycle/progress contract across the covered MCP responses.

### Modified Capabilities

None. There are no canonical capability specs for these surfaces in this checkout.

## Impact

Implementation touches existing bsl-search progress/readiness and PostgreSQL baseline detail readers; MCP lifecycle, response, budget and discovery owners; adjacent tests and MCP docs. The implementation and synthetic local verification are owned by this change; results are recorded in verification.md. No predecessor publication or further approval is required.

No new endpoint, notification stream, sampler, queue, dependency, database migration, model, customer reindex, downstream adapter change, installation, release, archive, commit or push. No persisted progress history or overall percentage. Implementation status is tracked in tasks.md; review and final verification remain mandatory.
