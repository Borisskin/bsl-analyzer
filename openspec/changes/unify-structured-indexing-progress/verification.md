# Verification and implementation evidence

Implementation complete after independent review, G1 remediation and final verification. The table below defines mandatory acceptance; the implementation evidence section records the completed checks. Fixture names are locked acceptance identifiers to add beside existing tests; commands must execute at least one test (zero-test success is failure).

## Verified source and prerequisites

Inspected 2026-09-11 at `fa4693c0c0936b74ebb6f686a76f2eb583f5b899`. Existing GitHub Actions run `34578618315` is successful for that SHA, including Linux Check and Windows; release is skipped and irrelevant. No pending PR, consumer release or neighboring change is an input.

| Source | Verified issue / implementation owner |
| --- | --- |
| crates/bsl-search/src/engine.rs | IndexProgress relaxed atomics; four begin_pass producers; helper exits can precede persistence |
| crates/bsl-search/src/lifecycle.rs | Existing process-instance identity; reuse without parsing logs |
| crates/bsl-search/src/vector_persist.rs | Existing native sidecar identity/coverage acceptance contract |
| crates/bsl-search/src/external_baseline.rs and external_baseline/postgres.rs | BaselineSnapshotDetails and snapshot_details; existing semantic_publication_complete metadata authority |
| crates/mcp-server/src/state/{bootstrap,embed,sync,types,overlay_retry}.rs and src/baseline.rs | Qualification, typed outcomes, existing counts/probe cache and overlay debt |
| crates/mcp-server/src/lib.rs | Workspace/reference dispatch, docs in both profiles, graph resolve/superseded fallback |
| crates/mcp-server/src/tools/search/{types,status,hybrid,render,docs}.rs | Response funnels, text/legacy projection, schemas and budgets |
| crates/mcp-server/src/graph/{state,mod,types}.rs and tools/graph.rs | Native graph report coherence and status/loading envelope |
| crates/mcp-server/src/contract.rs | Existing 2.2 policy, ToolDecl metadata, generated schema fingerprints |
| crates/mcp-server/tests/doc_examples.rs | Existing jsonschema dependency and document schema checks |

The remote reader can be verified with an isolated local PostgreSQL 18 schema without pgvector: current migrate_storage tolerates optional vector DDL failure. The fixture uses actual snapshot_details and minimal native metadata writes; it does not claim full publisher end-to-end proof. Local initdb/pg_ctl are available under /usr/lib/postgresql/18/bin. Implementation owns setup, loopback binding, temporary credentials, cleanup and recording results; no customer DB or user-provided service is required.

## Requirement / scenario → task → verification

| Requirement and scenarios | Tasks | Mandatory check and acceptance |
| --- | --- | --- |
| R3 S10/S11 | T1 | V1: bsl-search `indexing_pass_lifecycle` tests all four producers, empty persistence, phase/unit/unknown/zero totals, final commit failure, typed terminal exits, checked ID sequence, late update/drop/reset races and broker active-guard compatibility |
| R1, R3, R5 S10/S11/S15 | T2 | V2: mcp-server `indexing_wire_projection` validates exact fields/enums/nulls/ID bounds, counters and legacy/text agreement, no stale percentage; adversarial error strings excluded |
| R2 S4/S5/S6/S8 | T3 | V3: mcp-server `indexing_local_qualification` covers successful full/empty publication, deferred persistence/rerun, partial/malformed batch, read errors, sidecar versus unqualified BLOB fallback, dirty/generation fencing, disabled/failure and query timeout retaining ready |
| R2, R4 S7/S12 | T4 | V4: bsl-search `indexing_remote_publication_reader` ignored SQL test on isolated local PostgreSQL exercises actual snapshot_details with absent/complete/mismatched/malformed/invalidated marker and qualified empty snapshot; mcp-server `indexing_remote_cache` covers same-row identity, expired TTL, refreshed generation, missing details, pending overlay and no added probe/SQL call count |
| R2/R3 S9/S11 | T5 | V5: mcp-server `indexing_owner_lifecycles` tests fresh/stale/reloading/failed/superseded graph and reference uninitialized/loading/ready/failed/shutdown, coherent revision and null counters |
| R1 S1 | T6 | V6: mcp-server `indexing_workspace_responses` covers every design response row including baseline-warming, superseded retry, pending/unavailable fallback and empty/hit results; JSON-only lexical ready + semantic running assertion |
| R1 S2 | T7 | V7: mcp-server `indexing_reference_responses` covers both profiles, both docs actions, local/remote/fallback hits, empty, not-ready and reference status; scope isolation |
| R1 S3 | T8 | V8: mcp-server `indexing_graph_responses` covers status, traversal/missing-pool/resolve superseded loading and unaffected metadata/data/schema, including actual graph state differing from retry envelope |
| R5 S14 | T9 | V9: mcp-server `indexing_response_budget` measures final UTF-8 text plus compact JSON for exact-fit/one-token-too-small, long multibyte text, zero hits, fallback and graph loading; fixed error data/minimum, unchanged exemptions and hard-error/cancellation precedence |
| R5 S13 | T10 | V10: mcp-server `indexing_discovery_contract` validates real variants with jsonschema through tools/list and contract resource, rejects removed indexing including graph alternative-branch bypass, checks exact versions and golden fingerprints; list_platform remains 1 |
| R1–R5 S1/S4/S5/S6/S8/S12/S15 | T11 | V11: mcp-server `indexing_polling_smoke` uses actual MCP calls on synthetic one-module workspace and controlled loopback embedding stub: observe lexical-ready/semantic-running → ready, delayed persistence, native provider failure, disabled and empty scope; deadline 30 seconds per case, total <= 180 seconds; `indexing_observation_bounds` holds engine/snapshot locks, asserts try-lock unknown path, no added provider/build/scan/probe calls and bounded/redacted JSON |
| R1–R5 | T12 | V12: run existing doc_examples tests plus schema validation for new documented examples; review all enums/null rules, version/budget/exemption/rollback docs against design |
| R1–R5 S1–S15 | T13 | V13: full checks below, nonzero fixture audit, independent implementation review and remediation, strict OpenSpec validation and scope diff |

Each requirement and each scenario has positive/negative evidence above. Allowed unknown states do not replace required successful ready cases. Existing lazy start/probe counts form the baseline for no-extra-work assertions. Contended snapshot tests use deterministic synchronization rather than flaky elapsed-time performance assertions; overall deadlines prevent hangs. Tests leave no customer data or running fixture service.

## Runnable verification plan

Use existing Rust test modules and dependencies; no framework addition. Focused commands:

```sh
cargo test -p bsl-search indexing_pass_lifecycle
cargo test -p bsl-search indexing_remote_publication_reader -- --ignored
cargo test -p mcp-server indexing_
cargo test -p mcp-server --test doc_examples
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
openspec validate unify-structured-indexing-progress --strict --no-interactive
```

The ignored SQL check uses the existing `BSL_TEST_PG_URL` mechanism populated privately for the fixture's temporary database. Do not print credentials or put them in artifacts. Record test counts; filter names listed here must exist and execute. Follow repository docs/contributing/DEVELOPMENT_RULES.md for any additional required gates and document platform-specific checks honestly. Required contract acceptance runs locally on Linux; the existing Windows CI is source prerequisite evidence, not a claim that this new implementation was tested on Windows. Release/consumer qualification is outside this handoff.

Final implementation completion requires all V1–V13 passed, mandatory fixtures nonzero, independent review free of blocking findings, documentation aligned, all implementation tasks completed with evidence, and strict validation passed. Repairs within this scope are owned by T13; no future approval or predecessor publication is needed. Record actual commands/results and residual nonblocking platform/release limitations here at implementation time.

## Architecture review record

- Initial strict OpenSpec validation passed; the audit found incomplete producer/response coverage, unsafe ready inference, unspecified remote proof, loose budget/discovery rules and deferred architectural choices.
- These are resolved in the locked design and T1–T13/V1–V13. Independent read-only native and contract source audits informed the fixes; final independent read-only revision review is CLEAN (2026-09-11), reconfirmed after the final navigation, overlay-predicate and Clippy-command corrections.
- Completion proof: all 13 tasks have owned bounded outcomes and checks; all 15 scenarios are traced; positive ready fixtures prevent an unknown-everywhere shortcut; no external publication or approval is required.
- Final strict validation passed on 2026-09-11. Scope verification: exactly five target OpenSpec artifacts; five neighboring bound-embedding-batches-by-payload files retain their original SHA-256 hashes; tracked files unchanged; all 13 implementation tasks unchecked; cursor/service lines removed.
- No Rust changes, new-contract tests, implementation completion, deployment or consumer qualification are claimed by this architecture review.

## Implementation evidence — 2026-09-11

The source base remains `fa4693c0`; the implementation is an uncommitted diff on `feat/unify-structured-indexing-progress`. The historical architecture record above describes the handoff, not the current task state. Original implementation checks below are green; completed independent review and final verification are recorded below.

| Check / scenarios | Enforcing code and executable evidence | Result |
| --- | --- | --- |
| V1 / S10–S11 | `bsl-search/src/progress.rs`: generation token, checked sequence, try-lock, typed finish/drop. `engine.rs`: index_directory, run_embedding_pass, outer index_documents/replace_reference and sync ownership. `engine/lifecycle_tests.rs`: actual directory/doc/sync producers. `state/embed.rs`: publication precedes claim release; rerun retains ownership. | Native `indexing_pass_lifecycle`: 8 passed, including actual overflow branch, stale callback/drop/reset, empty owned persistence, store/persistence failures and per-file batch totals. MCP publication/keepalive tests pass, including the real broker integration in the workspace run. |
| V2 / S10–S11/S15 | `mcp-server/src/indexing.rs`: closed typed vocabulary, required nullable fields, bounded ID, one captured sample for text/legacy/JSON, no raw error fields. | `indexing_wire_projection`: 2 passed; unknown/zero/inconsistent totals, batch fallback, inactive counters and missing nullable key rejected by schema. |
| V3 / S4–S6/S8 | `engine.rs`, `store.rs`, `workspace_overlay.rs`: count outcomes, accepted sidecar versus BLOB fallback, cached context/coverage/epoch and overlay debt. `state/bootstrap.rs`, `state/embed.rs`, `state/mod.rs`: final fenced publication and qualified target selection. | Native `indexing_local_qualification`: 5 passed; MCP state projection/publication and actual search fixtures pass. Real loopback query timeout (12 seconds) returns lexical/degraded/partial while both targets remain ready and exactly one query request is made. |
| V4 / S7/S12 | `external_baseline/postgres.rs::snapshot_details` uses existing SELECT with metadata joins. `baseline.rs::indexing_publication` reads only current cached same-row evidence. `state/mod.rs` combines it with initialized clean overlay, exact published manifest identity and warmup outcome (G1). | Actual isolated PostgreSQL 18 ignored reader fixture: 1 passed; server stopped. `indexing_remote_cache` and `indexing_remote_qualification_combines_publication_and_overlay` pass missing/malformed/mismatched/expired/qualified-empty evidence, dirty/uninitialized overlay, and zero added actor/probe work. |
| V5 / S9/S11 | `graph/state.rs::status_report_with_indexing` samples one owner/revision; pre-opened pool availability preserves legacy status without I/O. `state/bootstrap.rs::ReferenceSearchState::indexing_snapshot` reads the reference lifecycle. | Graph owner truth table and actual graph boundary tests pass. Reference lifecycle, shutdown/contention/redaction and actual poisoned-engine publication failure pass; no false ready after failed publication. |
| V6 / S1 | `lib.rs::workspace_search` finalizes every non-error branch, including early baseline warming. `state/indexing_tests.rs` exercises the real dispatcher. | Normal not-ready/warming, pending/failed/disabled lexical fallback, semantic/lexical hits and empty results, status and native superseded target pass. Defensive retry renderer/finalizer is tested separately (see reachability note below). |
| V7 / S2 | `lib.rs::workspace_search` docs branch and `reference_search` use the reference owner. | Both profiles × both docs actions × local/remote/fallback × populated/empty are exercised by actual dispatch. Actor request bitmask proves remote/fallback paths. Not-ready/status and reference-only scope pass. Actual bodies validate against tools/list schemas and reject removed indexing. |
| V8 / S3 | `lib.rs::graph` attaches only status/loading, including resolve retry; normal data and schema remain outside target envelopes. | `indexing_graph_responses` covers actual status, all source-bearing loading actions, exhausted pool with graph still ready, data/schema/resolve exemptions, tiny-budget errors and negative schema validation. Defensive metadata-loading retry renderer is checked with a genuinely ready graph. |
| V9 / S14 | `tools/search/budget.rs::finalize_indexed_response` measures UTF-8 text plus compact structured JSON after attachment, preserves mandatory metadata, trims whole trailing hits, returns fixed -32602 minimum error. | `indexing_response_budget` passes exact fit/minimum-minus-one, long multibyte, empty/degraded and loading. Dispatcher tests cover list_platform/status/graph exemptions and tiny requests. Existing cancellation matrix passes; cancelled calls cannot wait behind reference publication. |
| V10 / S13 | `contract.rs`, `tools/search/types.rs`, `tools/graph.rs`, tool attributes in `lib.rs`. | `indexing_discovery_contract`: 1 passed; `contract::tests`: 14 passed, including unchanged golden/fingerprint check. Machine 3.0/search 5/status 2/graph 34/indexing 1/list_platform 1. Graph alternate schema branch cannot bypass required indexing. |
| V11 / S1/S4–S6/S8/S12/S15 | `indexing_runtime_tests.rs`: exact-test subprocess isolation, joined loopback provider, real MCP calls, bounded watchdogs. | Focused runtime 2/2 passed including 12-second timeout; full parallel MCP 1077 passed/0 failed/1 ignored in 70.14 seconds. Lexical-ready/semantic-running→ready, held publication/persisting, malformed provider, empty, disabled, held locks, total provider count, fixed vocabulary and redaction are proven. Fixture watchdogs sum to ≤180 seconds. |
| V12 | `docs/mcp/README.md`, `docs/mcp/TOOLS_AND_EXTENSION.md`, `tests/doc_examples.rs`. | doc_examples: 1 passed; graph examples are now validated too. Scope, enums/nulls, terminal rules, versions, budget/errors/exemptions and rollback match the design. |
| V13 | Repository gates and source diff. | `cargo test --workspace` exit 0 (279 successful harness summaries, 9972 reported passed, 67 ignored; includes subprocess summaries). Workspace all-target/all-feature Clippy with `-D warnings` exit 0. Strict OpenSpec validation passed. Final formatting and scope checks are repeated after review. |

### Reachability and limits

- `tools/search/call.rs` deliberately cannot emit `CallOutcome::Superseded`; the search retry closure is defensive. Ordinary resident reads hold the writer mutex, so normal `resolve_names` also cannot manufacture Salsa `PendingWrite`; existing session tests cover that outcome directly. The change preserves and wires both defensive closures. Their renderer/finalizer tests plus source wiring establish the response contract without introducing a production fault-injection API or claiming a reachable native trigger that does not exist.
- PostgreSQL qualification tests the actual reader of the existing completion marker, not a new full pgvector publisher audit. All fixture state is local and disposable; no customer service or provider was used.
- No new Windows run, release, downstream consumer deployment or real customer reindex is claimed. Linux contract qualification is the mandatory gate in this handoff; these platform/release items are outside its exit criteria.
- Five original files of neighboring `bound-embedding-batches-by-payload` retain their initial SHA-256 hashes. No staged changes, archive, commit or push.

### Complexity trace

Every added production element has a current requirement: native coherent record/tokens and checked allocator (R3); cached qualification/context/overlay counts on existing owners (R2/R4); same-statement remote metadata evidence and cache projection (R2/R4); one shared indexing wire/render type (R1/R3/R4); final envelope budgeting (R5); updated existing discovery schemas (R5). Test-only fixtures cover their listed scenarios. No new dependency, scheduler, persistent progress history, health monitor or service was added.

## Independent implementation review

- Native lifecycle/Ponytail read-only audit: clean for the reviewed generation fencing, terminal retention, outer publication/rerun and counter semantics.
- Independent response/Ponytail read-only audit (T2/T6–T10/T12): clean for wire/null projection, both search dispatchers, graph boundaries, final budgets, schema branches, versions/fingerprints and docs. Tests were run by the implementation owner, not claimed as independently rerun.
- Readiness contract audit found mandatory G1 (R2/S7): a new cached baseline could qualify an overlay from a different manifest because overlay identity was not compared. G1 was repaired: overlay identity is captured at existing clean/full/planned publication, and cached baseline proof requires the exact tuple. The A→B test failed before the fix; transitions and matching requalification now pass. The native full-refresh/plan/manifest-change/publication fixture proves an old plan keeps its own identity. MCP indexing suite: 22 passed; native new publication fixture: 1 passed. Independent implementation/Ponytail re-review of the final G1 patch is CLEAN: no mandatory findings; capture/comparison stays on existing owners and telemetry adds no I/O.

## Final verification — 2026-09-11

| Dimension | Final result |
| --- | --- |
| Completeness | 14/14 tasks (T1–T13 + G1), all 5 requirements and 15 scenarios mapped to code and executable checks above. |
| Correctness | Positive and negative qualification, response/schema/budget, native lifecycle and observation tests passed; G1 regression reproduced before repair and passed afterward. |
| Coherence | Locked architecture followed; independent native/response/readiness and Ponytail reviews clean after G1 remediation. No new dependency, service, storage migration or publication authority. |

- Final `cargo test --workspace`: exit 0; 279 successful harness summaries, 9975 reported passed and 67 ignored (totals include child-process summaries). All mandatory indexing groups executed, including runtime polling/observation and G1 native publication. The separately run isolated PostgreSQL reader fixture passed 1/1; its SQL implementation was unchanged afterward.
- Final `cargo clippy --workspace --all-targets --all-features -- -D warnings`: exit 0.
- Final `cargo fmt --all -- --check`, `git diff --check` and strict OpenSpec validation: PASS.
- Requirement/scenario coverage: R1/S1–S3 response matrix; R2/S4–S9 local/remote/owner qualification; R3/S10–S11 coherent generation and terminal lifecycle; R4/S12 bounded observation; R5/S13–S15 discovery, budgets and text/legacy agreement all have the Code/Test evidence listed above. No mandatory evidence is deferred.
- CRITICAL: 0. Mandatory warnings/gaps: 0. No unverified mandatory risk. The platform/release limits listed above remain outside scope.
- Scope audit: the five original neighboring change files retain their initial hashes; staging is empty. No archive, commit, push, release or external deployment was performed. Goal cursor removed only after successful final gates.
