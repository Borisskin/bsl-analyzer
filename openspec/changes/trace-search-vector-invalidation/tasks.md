## 1. Event and persistence foundation

- [x] 1.1 Add the bounded lifecycle event vocabulary, operation propagation and summary accumulator in bsl-search, reusing tracing; check V1 identity, reason fields, 128-file/one-second summaries and ten-example limit with a captured subscriber.
- [x] 1.2 Add the scoped CLI subscriber and private shared journal ring in CLI logging/tracing setup, using the design's paths, limits and fallback; check V5–V6 with temporary state roots, two processes, permissions, rotation, interrupted records, queue overflow and blocked writer.

## 2. Attribution at existing owners

- [x] 2.1 Capture startup provenance before both fenced/unfenced store-open mutations in store/engine/bootstrap; check V2 existing, absent, old-schema, unavailable and replaced-identity fixtures, outside the constructor lease callback.
- [x] 2.2 Propagate reindex reasons through fused graph ingestion and engine/bootstrap/sync decisions, preserving hash/read fallback behavior; check V1 decision matrix including unchanged, missing, cleared, changed, lookup failure, read failure and root/mode transitions.
- [x] 2.3.1 Verify active file/document/drift transaction attribution and dormant primitive exceptions; recheck the SQL/caller inventory and V3 exact non-NULL preimages, cascades, overlap, cancellation and earlier committed batches.
- [x] 2.3.2 Verify the context scalar observer against original NULL SQL under real competing writers and commit/rollback; preserve matched rows, generations and exact non-NULL loss counts (V3).
- [x] 2.3.3 Verify bounded bulk-removal envelopes and terminal committed totals across later fence refusal in engine reconcile/remove owners (V3,V8).
- [x] 2.4 Instrument root/mode transitions, preserving/root-key/schema/embed-text migrations and overlay value-cache effects in store/engine/bootstrap; check V4 baseline/overlay/cache totals, immediate hash-only zero loss, preserving migration zero loss and schema-reset preimages.
- [x] 2.5 Trace persisted artifact load/reject/remove/rebuild and live index replacement in engine/vector_persist with explicit zero SQLite loss; check V4 rejection reason and artifact removal followed by SQLite rollback remain separate outcomes.
- [x] 2.6 Add embedding pass start/resume and terminal totals at engine pass owners and MCP orchestration, carrying context across workers; check V7 completed, interrupted, failed, skipped and partial-commit passes with the existing fake embedder.

## 3. End-to-end evidence and handoff

- [x] 3.1 Add a small isolated restart/attribution fixture spanning bsl-search and MCP startup; check V7–V8 warm unchanged restart, partial resume, content/context change and sink failure against control database rows, generations and lease behavior. No live provider or production corpus.
- [x] 3.2 Update docs/contributing/LOGGING.md with exact paths, collection, filtering, retention, redaction and uncertainty; fill this change's verification.md with requirement-level actual results and source anchors. Check V9 by following collection on the fixture and clearly separating executed platform tests from unexecuted ones.
- [x] 3.3 Run focused tests in bsl-search, mcp-server and CLI logging, then repository formatting, Clippy and workspace tests; wire the portable V1–V8 regressions into existing Linux/Windows workflow with nonzero-test checks. Record commands/results, run strict OpenSpec validation and whitespace checks. A remote publish is not required for this local handoff.




Each task owns its adjacent runnable checks; V1–V9 and current results are defined in verification.md. All original implementation tasks are checked after source/test verification. No task requires a new product choice or approval. Deployment, archive, commit and push are outside this change's completion criteria.

## Разрывы ревью

- [x] R1 Preserve unavailable count quality in cumulative totals after a summary flush, including the following intent and terminal event. Verify with a captured 128-mutation batch before/after the fix.


- [x] R2 Add the mandatory V3 commit-failure oracle using a deferred foreign-key constraint; prove no committed loss is emitted and original rows/generation survive. No production behavior change.

- [x] R3 Eliminate intermittent missing lifecycle capture under parallel bsl-search tests; retain event assertions and verify the original full suite.
