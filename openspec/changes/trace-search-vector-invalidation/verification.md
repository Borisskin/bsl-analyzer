## Architecture evidence and implementation evidence boundary

Architecture inventory base: `949e30bb54ab517e5abcc9b551c123d3082f02ef`, 2026-09-09. The first inventory and V1–V9 acceptance oracles below preserve the approved scope. Current implementation evidence and executed commands are recorded separately below; architecture GO alone is not implementation evidence. This change remains unarchived.

## Production inventory (source anchors at the audited SHA)

Paths below are relative to crates. S = bsl-search/src/store.rs; E = bsl-search/src/engine.rs; M = mcp-server/src. Line numbers are navigation aids, function names are the durable anchors.

| Owner / callers | Effects and accounting | Task/check |
|---|---|---|
| S `reindex_file_in_collection_checkpointed`:1651; wrappers `reindex_file*`; E `ingest_fused_file_checkpointed`:1361 ← M graph/build.rs:1072; deferred/FTS bootstrap:1459,1511 | Existing tx upserts file before deleting chunks; count non-NULL children after reservation; one committed event owner | 2.2–2.3 / V1,V3 |
| S `reindex_documents`:1747, `reindex_indexed_documents_in_collection`:1901; E index_directory:1143, document ingest:2242, indexed-documents loop:4677 | Same replacement contract and collection label, no wrapper double count | 2.3 / V3 |
| S `remove_file`:929 ← E removal/reconcile ← M sync.rs:842,930 and bootstrap.rs:1643 | Existing tx FTS deletion then FK cascade; count children before parent deletion | 2.3 / V3 |
| S `replace_reference_collection_if_stale`:1805 ← E:2307 ← M bootstrap.rs:1736 | IMMEDIATE tx, fingerprint no-op distinct from replacement; reference role | 2.3 / V3 |
| S `apply_context_refresh_batch`:1226 ← E `refresh_dirty_contexts_fenced`:3480 ← M embed.rs:907 | Existing cancellable tx; NULL assignment boolean observer counts non-NULL old values, not matched rows | 2.3 / V3 |
| S `apply_workspace_drift_batch`:1283 ← E:2907 ← M sync.rs:692 | Existing batch tx, FTS write before file cascades; removals/tombstones/dirty marks have separate counters | 2.3 / V3 |
| S `apply_workspace_roots_transition`:1409 ← E:2771 ← M embed.rs:408 | Existing tx deletes baseline and overlay rows, then upserts; sequential selectors avoid duplicate counts; live index swap separate | 2.4 / V4 |
| S `clear_workspace_overlay_checkpointed`:3219 ← M bootstrap.rs:1236 | Existing cancellable tx, count baseline and legacy overlay children separately before cascades | 2.4 / V4 |
| S `migrate_structural_schema`:514 → `wipe_all_tables`:618 + `ensure_embedding_generation`:579 | IMMEDIATE tx; inspect old readable schemas and count before DROP. Unknown legacy shape is unavailable, not zero. Artifact deletion before commit is independently observable | 2.1,2.4–2.5 / V2,V4 |
| S `rebuild_root_keyed_tables`:441 | Preserving migration copies embeddings; logical loss zero when commit preserves rows; schema identity transition logged | 2.4 / V4 |
| S `migrate_embed_text_version_checkpointed`:642; `clear_file_hashes`:2433, `clear_file_hashes_without_embeddings`:2441 | Hash metadata mutation only: hashes cleared plus immediate loss zero; subsequent reindex uses its own reason/count | 2.2,2.4 / V1,V4 |
| S `migrate_overlay_embedding_cache_key`:822 inside schema creation | Drops incompatible value cache: cache entries counted separately from chunks, within existing schema tx | 2.4 / V4 |
| S `apply_overlay_publication`:3019; `save_overlay_embedding_cache`:3173 ← E:3986,4047 and workspace_overlay.rs:1624 | Publication tx / per-row autocommit save; replacing a value with a value is not missing-vector loss; report writes, do not count REPLACE as missing embeddings | 2.4,2.6 / V4,V7 |
| S `set_chunk_embeddings`:2323 ← E `run_fenced_embedding_pass`:1501 / `run_embedding_pass`:1606 ← M embed.rs:1091 | Existing batch tx; committed writes/progress, not invalidation; cancellation may follow earlier committed batches | 2.6 / V7 |
| E `load_or_build_index_unpublished`:798, `build_persisted_index`:822, `prepare_built`:852, `install_prepared_built`:872; vector_persist.rs `try_load`:158 / `remove_artifacts`:122 | Sidecar rejection/load/removal/rebuild from SQLite snapshot; zero SQLite loss, generation mismatch explicit; filesystem effects need not roll back with SQL | 2.5 / V4 |
| E live evictions:2939,3245 and replacements:1497,2844 | In-memory state only; never add to persisted SQLite loss | 2.5 / V4 |

Dormant/public primitives, no production callers found in crates at this SHA: S `delete_chunks_for_file`:972, `insert_chunk`:1358, `clear_collection`:2144, `clear_chunk_embedding`:2308, `set_chunk_embedding`:2315, `upsert_overlay_file_with_chunks`:2724, `remove_overlay_file`:2804, `clear_overlay_embedding_cache`:3192, `clear_overlay_state`:3197. Existing transactional overlay upsert can supply exact preimages; other multi-statement/autocommit definitions preserve statement boundaries and expose unavailable preimages where needed. Explicitly list statement outcome rather than claiming method-level atomicity. Standalone library callers still receive these events if invoked. No task deletes or rewrites these APIs just to simplify telemetry.

Raw SQL search across crates is the recheck: DELETE/DROP, embedding=NULL, hash resets, remove_file/rename of database/artifacts, plus all callers of inventory functions. No production whole-search-database filesystem replacement was found; startup identity records replacement observations without claiming an external actor. Non-vector deletions (FTS, manifests, fingerprints, dirty markers, tombstones) are supporting metadata and never counted as vectors. Any newly found active path belongs to the corresponding instrumentation task, under the same locked rules.

Positive reuse evidence: CLI logging.rs and tracing/config.rs own the subscriber; CLI mcp/process_record.rs:164 uses dirs state/data-local paths, UUID and std file locks; broker/security.rs owns the current-user SID/SDDL pattern. Existing daemon rotation in CLI mcp.rs:199 is startup-only and broker/proxy.rs:303 stores logs in runtime space: neither meets the required persistent concurrent byte bound, so the bounded scoped writer is necessary. Installed rusqlite 0.40 source exposes create_scalar_function via its functions feature; preupdate_hook would enable libsqlite3-sys buildtime_bindgen and is excluded. No new external runtime/toolchain dependency is a prerequisite.

## Requirement / scenario → task → verification

| Requirement and all scenarios | Tasks | Acceptance oracle |
|---|---|---|
| Startup provenance: existing index; missing/unreadable/legacy store | 2.1 | V2: seed known files/chunks/overlay/cache; capture pre-migration counts in one snapshot, absent≠unavailable, process IDs differ; fenced callback seam proves snapshot is outside lease; identity mismatch marks unstable |
| Explain reindex decisions: hash lookup fails; decision matrix | 1.1,2.2 | V1: captured subscriber with seeded unchanged/missing/empty/changed hashes and injected hash/read errors; same outcome as uninstrumented control, correct reasons/root/mode and operation propagation |
| Committed invalidation: commit; rollback; cancelled after partial progress; preserving/hash-only migrations; dormant APIs | 2.3–2.4 | V3: mixed NULL/non-NULL rows, FK cascades, repeated selectors, failing statement/checkpoint/commit seams. Assert exact normal active counts and zero rolled-back losses; preserve earlier child commits; observer matches original NULL assignment including generation and competing writer behavior. Unavailable is explicit only in permitted cases |
| Committed invalidation: derived artifact rebuild; migration/cache distinction | 2.4–2.5 | V4: schema/text/root-key fixtures, overlay/cache rows; load/reject/remove/rebuild sidecars from deterministic vectors; assert hashes-only and preserving migration have zero immediate loss, schema reset exact counts, filesystem removal survives SQL rollback as separate event |
| Embedding resume: partial restart; warm restart; terminal pass outcomes | 2.6,3.1 | V7: existing fake/embed-from-vectors fixtures, no model download/API. Seed complete and partial state, restart fresh engine/process, assert complete vectors unchanged and only pending work submitted; failure/refusal after one committed batch retains writes in terminal totals |
| Persistent bounded output: restart/rotation; unwritable journal; concurrent writers/privacy | 1.2 | V5: subprocesses share temporary state root, exceed eight slots, reopen, check nine-file/32-MiB bound and valid JSONL; partial-line/header recovery and differing IDs; Unix modes/Windows DACL/reparse tests; unsafe location disables only journal |
| Persistent bounded output: overflow and filter safety | 1.1–1.2 | V6: queue overflow/blocked sink, gap count, rate-limited fallback without recursion, BSL_LOG broad warn vs explicit target off/debug; canary source/vector/token/URL/raw-error fields absent from both journal and emitted scoped fallback; protocol stdout parses as protocol only |
| Bounded overhead/honest evidence: large operation; failure/crash uncertainty | 1.1–1.2,2.3,3.1–3.2 | V8: 10,000-file synthetic counter stream yields bounded summaries/examples; query/scalar counters show no new corpus scan per item, full writer queue cannot block producer/lease; fixture database and generation equal control under sink failure; two-second shutdown drain limit; explicit unavailable outcome after commit-delivery seam |
| Collection/limitations and handoff | 3.2–3.3 | V9: collect fixture segments in generation order, read reason/count/quality; docs name retention/gaps/power-loss and unknown history. Record actual commands/results for fmt, Clippy, focused/workspace tests and OpenSpec; workflow nonzero-test guards select portable checks on Linux and Windows |

## Final acceptance and owned completion

All listed normal-path V checks must pass; injected failure scenarios pass by preserving indexing and reporting uncertainty, not by requiring a working disk. Test-only seams/fake vectors are owned by this change and must be included where existing fixtures lack the needed failure injection. No live provider, production reindex, daemon installation, remote predecessor change, new owner decision or future approval is an acceptance prerequisite. Platform-specific source and workflow coverage are mandatory; record separately whether Windows runtime validation was actually executed, without implying remote success from Linux results.

Implementation, independent reviews and final completion audit passed. All 16 tasks are complete; all six requirements and 18 scenarios have code/test evidence, with zero mandatory findings.

Architecture results (2026-09-09): initial and revised strict OpenSpec validation passed. Independent read-only review inspected all five final artifacts plus source/dependency anchors and returned GO with zero blockers; final reread also confirmed the queue/pending-record payload bound and try_lock clarification. Reviewer confirmed atomic scope, exact scalar-observer feasibility, ring/ACL implementation ownership and full V1–V9 coverage. Main review completed the task-to-result proof in design.md. Architecture GO does not assert that V1–V9 or Windows runtime tests have run. At that architecture-only stage, no implementation edits or task completion were claimed. The implementation evidence below records the subsequent work.


### Implementation progress

- 1.1: lifecycle.rs typed vocabulary, fixed 8-KiB encoding, captured worker context and bounded summaries. `cargo test -p bsl-search lifecycle:: --lib`: 4 passed, including 10,000-mutation captured-subscriber aggregation and terminal committed totals.

- 2.1 / V2: `cargo test -p bsl-search --lib lifecycle`: 30 passed (2026-09-09). Startup fixtures cover absent/unavailable/legacy/cache/replaced identity and both fenced/unfenced opens before schema reset, exactly one snapshot before loss. `store_identity_normalizes_relative_paths_and_distinguishes_owned_roles` verifies absolute spelling and workspace/reference/baseline labels. Snapshot hook precedes retry loops and constructor callbacks; MCP supplies roots/mode at both bootstrap branches.

- 2.2 / V1: `cargo test -p mcp-server --lib vector_lifecycle`: 7 passed. Real fused writer cases distinguish unchanged, changed, empty hash, missing record, lookup error and read failure; captured mutations share decision parent IDs and preserve the original fallback. Engine context/mode fixture is included in the 30-test bsl-search lifecycle pass.

- 2.3 / V3,V8: `cargo test -p bsl-search --lib lifecycle`: 31 passed; `cargo test -p bsl-search`: 436 passed, 29 existing ignored. `context_observer_matches_original_sql_with_competing_writer_and_rollback` forces SQLITE_BUSY against a live peer write, then compares original NULL and observer SQL for both commit/cancel, four matched updates, generation deltas and two/zero losses. `vector_lifecycle_bulk_removal_bounds_info_and_retains_commits_before_refusal` checks 257 deletions and refusal after 129 committed files, bounded INFO and terminal exact totals. Store lifecycle fixtures cover replacement/cascades/repeated selectors, read failure unavailable, cancellation and reference no-op.
- 2.4 / V4: same passing bsl-search suite covers preserving root-key migration, text/hash-only reset, structural reset baseline/overlay/cache preimages and overlapping root selectors. `hash_only_cache_only_and_structural_reset_are_distinct`, `preserving_root_migration_and_legacy_cache_report_separate_losses`, `schema_reset_counts_cache_entries_even_without_readable_vectors` and `repeated_root_cleanup_counts_each_baseline_and_overlay_vector_once` provide captured-event evidence; engine context/mode fixture proves independent zero-loss mode records.
- Current inventory recheck: active overlay publication also has `workspace_overlay.rs::publish` call at line 1830; cache save is called at line 1624. The nine listed dormant vector APIs remain definition/test-only. `clear_file_hashes` additionally has only an unused public engine wrapper; active partial-resume startup uses `clear_file_hashes_without_embeddings`. Bulk reconcile/removal now owns operation envelopes in engine; store remains the transaction-count owner.

- 2.5 / V4: latest `cargo test -p bsl-search --lib lifecycle --quiet`: 33 passed. Added partial index/sidecar rename outcomes plus generation-read failure and mismatch refusal; successful first rename stays committed, failed second rename does not invent SQL loss. Existing artifact load/reject/removal/rollback fixtures remain green.
- 2.6 / V7: engine fake embedder submits only pending documents; warm pass skips with unchanged generation; later failure/refusal preserves first committed batch. MCP `vector_lifecycle_embedding_orchestration_preserves_context_and_terminal_states` verifies worker context and completed/interrupted/failed outcomes.
- 3.1 / V5–V8: CLI integration `cargo test -p bsl-analyzer --test vector_journal_cli`: 3 passed, including two real workspace stdio startups retaining distinct process generations at broad warn. Journal unit suite: 11 passed; full/disconnected sink matches control database rows/vectors/generation and producer completes while queue remains undrained. Fused MCP fixture verifies later lease takeover preserves prior committed totals. Windows-specific runtime remains unexecuted locally.
- Gate repair: full MCP library run found only the earlier change's fixed-base Git/Cargo.lock audit (998 passed, 1 failed, 1 ignored). Removed that phase-specific audit: it forbids this approved serializer/native-ACL feature and cannot be a permanent runtime invariant. Kept the three source-based lease caller/request invariants, made path comparison and test-module detection portable, and added LF/CRLF helper regression. `cargo test -p mcp-server --lib inventory --quiet`: 4 passed. `cargo clippy --all-targets --all-features -- -D warnings`: passed.

## Current requirement-to-code-to-test evidence (2026-09-09)

Paths below are relative to `crates/`. All fixture tests use temporary directories, fake vectors or a loopback fake embedder; no provider API, model download or production corpus is required.

| Mandatory requirement / scenarios | Enforcing code | Executed fixture evidence |
|---|---|---|
| Startup provenance — existing, absent, unreadable, legacy and identity replacement | `bsl-search/src/lifecycle/startup.rs::observe`, `Store::open/open_existing`, `SearchEngine::open_store_fenced`, both MCP bootstrap roots scopes | Three startup snapshot tests; engine `vector_lifecycle_startup_snapshot_precedes_fence_with_root_mode_metadata` and `vector_lifecycle_each_open_snapshots_once_before_schema_reset`; actual CLI restart test |
| Reindex decisions — lookup failure and complete matrix | `lifecycle::hash_reason/decision/with_reason`, `SearchEngine::observed_file_hash`, `FusedChunkWriter::emit_chunks` | Six `graph/build/vector_lifecycle_tests.rs` tests cover unchanged/missing/cleared/changed/read/lookup errors and takeover; engine context/mode fixture covers those reasons |
| Committed invalidation — commit, cascades/overlap, rollback/cancel, partial prior commits | Store transaction owners in the inventory; `Mutation::finish`, `Batch`, engine bulk removal/reconcile envelopes | `replacement_and_cascade_count_non_null_children_once`, `cancelled_replacement_has_zero_committed_loss_and_keeps_generation`, `failed_cascade_never_claims_committed_loss`, repeated root cleanup; engine bulk removal refusal and fused takeover retain earlier totals |
| Committed invalidation — original locking and context observer | `Store::apply_context_refresh_batch`, connection-local scalar registration | `context_observer_matches_original_sql_with_competing_writer_and_rollback`: real busy-handler handshake while peer holds writer; original/observer matched rows, vectors and generations equal for commit/cancel. Registration/read failures use explicit unavailable without changing writes |
| Committed invalidation — preserving/hash-only migrations and dormant APIs | Structural/root-key/text migrations, cache migration, dormant statement hooks | `hash_only_cache_only_and_structural_reset_are_distinct`, preserving root migration, schema cache counts, dormant unavailable preimage and reference no-op fixtures |
| Derived artifact rebuild, rejection, removal and partial publication | `vector_persist::{try_load,remove_artifacts,PreparedPersist::install}`, engine prepare/install/build and live index observer | Four artifact lifecycle tests plus engine generation-refusal/read-failure fixture; removal survives SQL rollback; all artifact SQL-loss counters zero |
| Embedding resume — partial restart, warm restart and terminal outcomes | `SearchEngine::run_embedding_pass/run_fenced_embedding_pass`, `Store::set_chunk_embeddings`, MCP embedding orchestration guard | Fake embedder partial/warm test; failure/refusal retains committed first batch; MCP orchestration test checks worker context and terminal states |
| Persistent bounded output — restart, ring and privacy | CLI scoped layer/worker/ring; Unix ownership/mode checks; Windows DACL/reparse adapter | 11 CLI logging unit tests include two subprocess writers, 32-MiB/nine-file limit, tail/header repair, Unix unsafe targets, explicit filter/record limits. Three real Unix CLI tests check valid restart, invalid arguments and protocol during sink failure |
| Persistent bounded output — writer failure, overflow, fallback | Journal `try_send`, dropped counter, worker gap and rate-limited diagnostics; bounded guard drain | Full/disconnected sink matches control rows/generation/vectors; worker gap precedes data; blocked disk keeps producer nonblocking; shutdown deadline checked while lock remains held; fallback fails without recursion |
| Bounded overhead and honest evidence — large operation and collection | Lifecycle 128-file/one-second summaries, ten examples, fixed encoding buffer; scalar counts; docs collection/limitations | 10,000-mutation subscriber fixture and real 257-file removal fixture; full queue exercise retains control behavior. Collection performed after two real writers stopped, sorted by generation, parsing complete JSONL lines |

Collection result: `/tmp/bsl-vector-collection-uwxf7caz` retained one segment with three complete records from two process generations; startup states were `absent` then `observed`. The documented generation-order procedure was followed after both writers were terminal. This fixture demonstrates retained diagnostic evidence, not power-loss durability or the historical incident's cause.

Platform boundary: Linux runtime tests executed. Windows native ACL/reparse source and dedicated nonzero-test CI step are present; Windows execution is not claimed. The existing Linux/Windows workflow runs portable V1–V8 filters and rejects zero selected tests. No publication is part of local completion.

## Repository gates

- `cargo fmt --all -- --check`: passed.
- `cargo clippy --all-targets --all-features -- -D warnings`: passed.
- `cargo test --all --no-fail-fast --quiet`: exit 0; 9,786 passed and 66 existing ignored in the final workspace log, zero failures. Full output retained locally at `/tmp/bsl-vector-final-workspace-tests.log`. The subsequent test-only R2/R3 additions were checked in the final full bsl-search suite below.
- `actionlint .github/workflows/ci.yml`: passed.
- `openspec validate trace-search-vector-invalidation --strict --no-interactive`: passed.
- `git diff --check`: passed.

No ignored test was changed or used as passing evidence. The portable lifecycle, sink and CLI acceptance checks selected nonzero tests. Windows runtime execution remains outside the local results above.

## Independent review, pass 1

Read-only `ponytail-review`: no mandatory complexity reductions. Read-only implementation-vs-plan review found one mandatory issue, R1: cumulative unavailable totals were labelled exact after current summary counts reset. SQL owner coverage, observer behavior, startup hooks, scoped sink and ACL source wiring had no further confirmed blockers. R1 was tracked in tasks.md and resolved below.

R1 correction: `unavailable_totals_keep_their_quality_after_flush_and_at_termination` failed on the previous code (`exact` vs `unavailable`), then passed after `Record::encode` included cumulative totals in the quality calculation. `cargo test -p bsl-search --lib lifecycle --quiet`: 34 passed. No SQL, transaction or sink policy changed. Subsequent re-reviews passed.

Independent review, pass 2: read-only ponytail-review and implementation-vs-plan both returned zero mandatory findings. R1 is fixed at the shared serializer and its regression covers progress, following intent and terminal state. Final repository gates passed on this code; Windows runtime remains explicitly unexecuted locally.

Final audit found R2: V3 explicitly requires a commit-failure seam, while prior failure tests rejected statements before commit. Added a deferred foreign-key fixture at the existing transaction owner; this closes missing evidence without changing production behavior.

R2 evidence: `cargo test -p bsl-search --lib failed_commit_keeps_rows_and_reports_unknown_not_committed_loss --quiet`: 1 passed. A deferred foreign key rejects COMMIT after successful destructive statements; the original row/vector/generation survives, guard insertion rolls back, and the journal reports unknown/unavailable with no committed loss. Production code was unchanged.

Independent review, pass 3: R2 fixture verified against the actual commit path; ponytail-review and implementation-vs-plan returned zero mandatory findings, with no further V1–V9 gaps. Production code is unchanged since the R1 serializer correction.

R3: additional final `cargo test -p bsl-search --quiet` failed at `vector_lifecycle_artifact_publish_refuses_changed_generation_without_installing`: zero captured records instead of one (439 passed, one failed, 29 ignored). Assertions remained intact; the root cause and successful repeat are recorded below.

R3 correction: tracing-core 0.1.36 single-dispatch callsite registration can consult an unsubscribed parallel thread and cache `never`. The shared cfg(test) capture wrapper keeps two non-global no-op Dispatch instances alive, disabling this optimization for all bsl-search capture helpers. No production behavior or global subscriber changed. `scoped_capture_survives_first_callsite_on_unsubscribed_thread` runs in an isolated subprocess: without the fix it fails deterministically (zero vs one event, exit 101); with the fix it passes. Original full `cargo test -p bsl-search --quiet`: 441 passed, 29 ignored, zero failures; log `/tmp/bsl-vector-r3-search-tests.log`. This run includes R1, R2 and R3.

## Final complexity gate

| Added element | Current mandatory requirement |
|---|---|
| Typed lifecycle records, operation context and bounded accumulator | Explain decisions, transaction attribution and bounded large-operation output (V1,V3,V8) |
| Startup snapshot and identity observation | Pre-mutation provenance, absent/unavailable/replaced distinction (V2) |
| Connection-local scalar observer and rusqlite functions feature | Exact context-update preimages without changing the original locking/SQL behavior (V3) |
| Scoped tracing layer, bounded std queue, writer and shared ring | Persistent restart evidence, nonblocking failure and concurrent byte bound (V5,V6,V8) |
| Existing Windows API feature wiring and ACL/reparse adapter | Private journal access on Windows (V5) |
| Transaction/artifact/embedding owner hooks | Committed-loss attribution, independent sidecar outcomes and partial resume (V3,V4,V7) |
| Temporary fixtures, capture wrapper and isolated callsite regression | Runnable acceptance including failure injection and reliable parallel event capture (V1–V8) |
| Logging collection docs and existing CI steps | Reproducible collection and nonzero Linux/Windows test selection (V9) |

All added elements have a current requirement above. Existing tracing, std synchronization/filesystem primitives, installed serialization/SQLite dependencies and native Windows APIs are reused. No speculative service, configuration layer or new external runtime prerequisite was added.

Final review and gates after R3: independent ponytail-review and implementation-vs-plan returned zero mandatory findings. `cargo clippy --all-targets --all-features -- -D warnings`, formatting, strict OpenSpec validation, actionlint and whitespace checks passed after the final test-only change. The earlier full workspace run and final full bsl-search run together cover the final code; no production code changed after the workspace run. All 13 original tasks and R1–R3 are checked, V1–V9 have evidence, and the final cursor is removed. Windows runtime is the sole unexecuted platform boundary and is not claimed as a local result. No commit, push, archive or deployment performed.
