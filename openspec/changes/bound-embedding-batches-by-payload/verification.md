## Status

Architecture readiness: GO. D1 was accepted and final independent re-review completed without mandatory findings on 2026-09-11. Implementation, independent review/remediation and local final verification completed on 2026-09-11. The subsequent PR #148 Windows CI failure is being corrected under the user's 2026-09-12 instruction; its current evidence is recorded below.

## Source-backed baseline before implementation

Exact source `fa4693c0c0936b74ebb6f686a76f2eb583f5b899` on `feat/bound-embedding-batches-by-payload`, inspected 2026-09-11. GitHub Actions run `34578618315` rechecked completed/success for exactly that SHA. This is a positive read-only input, not evidence for the planned payload behavior. No future predecessor merge, deployment or consumer acceptance is required.

| Owner | Verified source / implication |
| --- | --- |
| `crates/bsl-search/src/embedder.rs` | Baseline request serializer used send_json; ten batch attempts/120s versus one interactive/12s; response preview leaks and byte-slices UTF-8; sorted count does not prove index permutation. |
| `crates/bsl-search/src/publish.rs` and `ports.rs` | Existing execution policy and generic generator; reuse bounded worker channels and semantic-key mapping. |
| `crates/bsl-search/src/engine.rs` | Nine-family inventory below; parallel errors can be reduced to lifecycle-only failure while returning Applied; first typed error can return before complete sidecar publication. |
| `crates/bsl-search/src/workspace_overlay.rs` | Second overlay embedding path with missing-index mapping and existing cached embeddings. |
| `crates/mcp-server/src/state/{bootstrap,embed,types,mod}.rs` | Existing semantic/overlay/reference owners and native outcome consumption; extend these instead of another observer. |
| `crates/bsl-analyzer/src/bin/cli/search_baseline/postgres.rs` | CLI embedding and execution config resolution; MCP resolution is bootstrap::embedding_config. |
| `crates/mcp-server/src/tools/search/{semantic,docs,status,types,hybrid,render}.rs` and `src/lib.rs` | Raw embedding errors can reach fallback/error/status; both profiles route reference docs; hit budgets have a soft minimum, status has no token input. |
| `crates/mcp-server/src/contract.rs` | Baseline machine 2.2/search 4/status 1; change target 2.3/5/2 failure contract. |
| `Cargo.lock`, pinned ureq 3.3.0 source | Existing response read limit 10 MiB; typed timeout/status/read-limit errors permit safe classification without raw strings. |

The earlier downstream snapshot had 41,488 chunks and 13,056 stored embeddings. It showed successful requests and oversized candidate payloads, but no retained exact provider error. It does not establish one cause for every missing vector and is not a required customer-data fixture.

## Complete native caller inventory

| Family | Source entry | Required evidence |
| --- | --- | --- |
| File indexing | `engine.rs::index_directory` → embed_batch | Packed file inputs and complete-file publication/mapping. |
| Fenced pending | `engine.rs::run_fenced_embedding_pass` | Existing checks before request and fenced writes after each packed request. |
| Parallel pending | `engine.rs::run_embedding_pass` | Existing bounded workers/queues, retained earlier commits, failed terminal outcome. |
| Reference documents | `engine.rs::embed_documents` / replace_reference_collection_if_stale | Ordered parallel batches, whole-corpus publication and lexical-only failure outcome. |
| Off-lock overlay | `engine.rs::embed_missing_overlay_chunks` | Interactive calls, checks around network and per-batch cache publication. |
| Collection sync | `engine.rs::sync_indexed_documents_in_collection_with_embeddings` | Cached/missing vectors and cursor/range alignment; no partial file success. |
| Overlay vector builder | `workspace_overlay.rs::build_overlay_vectors` | Missing-index mapping and embedding-cache reuse. |
| Shared baseline publisher | `publish.rs::SharedEmbeddingPublisher::publish` | Trait planner override, unchanged worker caps, keyed storage and failed marker qualification. |
| Query/direct transport | `engine.rs` query methods, MCP search/wait.rs; Embedder public methods | Singleton guard, one interactive attempt and safe local multi-input guard. |

## Requirement / scenario → task → verification

Fixtures below are names to implement beside the existing native/MCP tests. Every command must execute a nonzero relevant test set; a zero-test exit is not evidence. Implementation owns its loopback services, synchronization, temporary stores and cleanup.

| Requirements / scenarios | Task | Mandatory check |
| --- | --- | --- |
| R1 S1/S2 | T1 | V1 `payload_configuration`: selected D1 default/override/invalid cases in both production constructors and direct library config; disabled semantics unchanged; no network on rejection. |
| R2 S3–S6, R4 S10 | T2 | V2 `payload_transport`: actual captured bodies for ASCII/Cyrillic/escapes, model/dim/provider overhead, exact/one-byte-over limits, empty input, oversized singleton/raw multi-input, shuffled/invalid indices and vector shape. Verify one-body retries, no HTTP for local refusal and typed independent response-limit failure. |
| R2 S3/S6, R3 S7/S8 | T3 | V3 `payload_pending_publication`: sequential fenced and parallel pending paths; physical-body/count caps; earlier committed rows after later 413; missing rows pending; no complete sidecar/Ready; Released/Superseded/TransientRefusal before final publication wins. |
| R2 S3/S6, R3 S7/S8 | T4 | V4 `payload_file_collection`: file indexing and collection sync with cached gaps/remainder; compare original IDs/text to stored vectors; a failed file is not partially accepted. |
| R2 S3/S6, R3 S9 | T5 | V5 `payload_reference_publication`: packed parallel reference inputs retain order; embedding failure returns lexical-only outcome/FTS stamp and failure; later successful call retries and clears failure. |
| R2 S3/S6, R3 S7/S8 | T6 | V6 `payload_overlay`: both off-lock and cache-builder paths, actual bodies and missing-index reuse; cancellation between requests, current cache commits/debt and no stale publication. |
| R2 S3/S6, R3 S7/S8 | T7 | V7 `payload_shared_publish`: production Embedder trait override and existing fake implementation; actual keyed vectors, count/concurrency bounds, earlier stored batches after failure and no successful publication result. Use the existing local FakeEmbeddingStore in the new failure test to prove the publisher returns Err, then verify exact CLI control flow: BaselinePublisher::publish(...)? precedes and prevents populate_serving_semantic_with_progress on failure. FakeEmbeddingStore.publish_calls counts snapshot publication, not semantic markers; do not claim it proves a remote marker write. No new remote SQL qualification. |
| R3 S7–S9, R4 S10/S11 | T8 | V8 `payload_lifecycle`: native first failure reaches workspace/overlay/reference owner; source owner/cancel precedence, no false Ready, retry/new-attempt reset, query failure not persisted, lexical reference remains ready. |
| R4 S10/S11, R5 S12/S13 | T9 | V9 `payload_mcp_contract`: actual workspace status/code fallback hits+empty, both reference docs routes/status and search_docs errors; adversarial raw bodies/UTF-8/messages never leak; schemas accept allowed optional objects and reject unknown codes/keys/numbers; versions/fingerprints, complete budget and minimum-floor behavior. |
| R1–R5 S1–S13 | T10 | V10 existing `doc_examples` plus new failure-example schema/error-data checks; review limits, ceilings versus response limits, retry/cost, direct raw API, scope and rollback against design. |
| R2–R4 S3/S6–S11 | T11 | V11 `payload_mcp_smoke`: actual MCP build against joined loopback server that rejects oversize bodies; enough differently sized module chunks to force byte packing below the count cap; observe success, exact stored-vector mapping, known later failure and successful next attempt. Assert query failure is local and status observation adds no provider call. |
| R1–R5 S1–S13 | T12 | V12 full repository gates, every fixture family nonzero, independent final implementation/Ponytail review, scope check and strict OpenSpec validation. |

For V2/V11, test deterministic 413 through the batch path without ten sleeps; exercise a transient failure followed by success to prove retries do not replay successful ranges. Test response-size classification on the one-attempt interactive path against the unchanged native read bound. Bounded service/watchdog setup: at most 30 seconds per synthetic MCP case and 180 seconds total, excluding compilation. Use synchronization/counters instead of timing thresholds for cancellation and no-extra-work evidence. No customer service, model, provider secret or custom framework is necessary.

Existing related tests provide reusable fixtures: native lifecycle and reference replacement tests; MCP state/embed fence/retry/panic hooks; search renderer `budget_covers_the_text_and_the_structure_together`, hybrid `a_long_degradation_note_is_charged_to_the_budget_that_carries_it` and `the_whole_response_stays_within_the_budget_it_was_given`; doc_examples and contract snapshots. Do not treat those unchanged tests alone as proof of new byte packing or failure fields.

## Planned commands and acceptance

```sh
cargo test -p bsl-search payload_
cargo test -p mcp-server payload_
cargo test -p bsl-analyzer payload_configuration
cargo test -p mcp-server --test doc_examples
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
openspec validate bound-embedding-batches-by-payload --strict --no-interactive
```

The mandatory implementation qualification is local Linux synthetic/native/MCP acceptance. New Windows CI, full remote publisher/provider qualification, consumer deployment and customer reindex are outside scope, not blockers deferred to another team. The source's pre-existing CI success is not new implementation acceptance. Runtime test processes/services must be joined/stopped, and no unrelated files or data changed.

## Architecture audit record

- Initial strict validation passed. Source audits found unresolved byte config, hidden-split checkpoint growth, incomplete caller inventory, lost native failures, raw response leakage, loose mapping validation and unspecified wire/budget behavior.
- Independent native and MCP read-only audits confirmed the common caller-visible planner, safe typed outcomes on existing owners and compatible additive response contract. Main source review confirmed a plain first-error return before complete publication preserves committed data without a new partial-result type.
- Existing reference lexical-only success and hit-response soft minimum are preserved. No dependency on the separate structured-indexing implementation is introduced.
- Independent artifact re-review found no technical blockers beyond D1 after correcting V7: reuse the existing FakeEmbeddingStore in a new failure test; snapshot counters do not prove semantic-marker writes. Requirement/scenario coverage is 5 requirements / 13 scenarios mapped to T1–T12 and V1–V12.
- D1 was explicitly accepted by the user on 2026-09-11: 1 MiB default, positive environment override, safe invalid-configuration failure for zero/malformed/overflow. Final independent re-review returned GO with no mandatory findings. At that architecture handoff all implementation tasks were unchecked; implementation evidence appears below.

## Joint completion proof

T1 supplies the accepted finite policy to T2. T2 produces exact contiguous bounded ranges and safe failures; T3–T7 integrate every caller family without replacing existing ownership or publication controls. T8 retains those failures at existing lifecycle boundaries; T9 exposes them under the closed compatible schemas and current budget minimum. T10 documents that same behavior, T11 exercises success/failure/recovery through the actual local MCP path, and T12 verifies the combined change. Thus every task reaches the single bounded-and-truthful embedding outcome; expected failures are successful negative tests, not unresolved acceptance blockers. All R1–R5 / S1–S13 have owned V1–V12 evidence routes above. No deployment, future predecessor, product decision or external approval is required. This paragraph records the architecture completion argument; actual executed checks are listed below.

At the architecture handoff, strict OpenSpec validation passed for the five planning artifacts, all 12 implementation tasks were unchecked, and the service cursor was removed. That historical record does not describe current implementation progress.

## Implementation evidence

- T1 / R1 S1–S2: EmbedderConfig resolves DEFAULT_MAX_REQUEST_BYTES and request_bytes_from_env; native semantic constructors validate before Store open; MCP embedding_config and CLI embedder_config propagate Result<Option<_>>. CLI resolves settings before adapter/publication. `cargo test -p bsl-search payload_configuration` PASS (1); `cargo test -p mcp-server -p bsl-analyzer payload_configuration` PASS (2 relevant tests, including nonzero CLI child invocation). Explicit bad/zero/overflow settings are safe errors; disabled semantics, existing count/concurrency and valid overrides are covered. Literal migrations use existing defaults.

- T2 / R2 S3–S6, R4 S10: Embedder::batch_ranges and prepare_request/send_request use the same serializer, caller ranges and checked exact body guard; EmbeddingGenerator has a count-only default; typed safe errors include response-read limit and response permutation/shape validation. `cargo test -p bsl-search payload_` PASS (6: T1 plus 5 transport tests); captured HTTP bodies, 413 no retry, local no-network refusals, one-body transient retry, successful-range non-replay, UTF-8/escapes/optional envelope, shuffled/invalid indices, response 10 MiB bound and actual timeout all covered with joined local fixture.

- T3 / R2 S3/S6, R3 S7/S8: both pending passes consume the common preplanned ranges and expose actual batch counts. Failed parallel work drains/joins workers, preserves first error and returns before sidecar completion; sequential failure checks final owner/cancel precedence. `cargo test -p bsl-search payload_pending_publication` PASS (2 tests, both paths, retained typed 413 + exact stored IDs/vectors + retry recovery; all Released/Superseded/TransientRefusal and cancellation variants win after a failed request). No complete sidecar on failure.

- T4 / R2 S3/S6, R3 S7/S8: file indexing preplans every file before scheduling; collection sync plans missing ranges per file, maps range offsets to missing_indices and counts actual requests. File indexing now retains and returns first failure after workers join, before complete live/sidecar publication. `cargo test -p bsl-search payload_file_collection` PASS (2 tests, both entry points): exact stored input/vector mapping, reversed collection input, cached gaps, batch totals, and later 413 retaining the previous atomic file version.

- T5 / R2 S3/S6, R3 S9: embed_documents plans contiguous byte/count ranges, joins bounded workers and restores range order; ReferenceCollectionReplaceOutcome now carries optional safe embedding_failure independently of lexical success. `cargo test -p bsl-search payload_reference_publication` PASS (1): a later 413 produces usable FTS corpus/stamp + typed failure, the next call retries all missing vectors, validates exact stored mapping, and successful/unchanged outcomes contain no stale failure.

- T6 / R2 S3/S6, R3 S7/S8: off-lock overlay and cache vector builder use common ranges. Off-lock failure checks cancellation/fence before returning and retains prior durable cache batches; cache builder preserves successful keyed vectors. All four full/dirty raw/manifest refresh routes settle every key/debt before surfacing a typed embedding failure. `cargo test -p bsl-search payload_overlay` PASS (3): saved cache and exact mapping, missing-index gaps and resume, between-request cancellation, all refresh routes reaching later keys without swallowing failure.

### T7 executed evidence

`cargo test -p bsl-search payload_shared_publish` passed all 3 tests. The production Embedder trait override honors exact byte ranges and keyed cache gaps; the existing generic fake retains count batching. A synchronized two-worker fixture verifies the existing concurrency bound, and a later typed failure preserves earlier keyed stores. The publisher returns Err, and source control flow in CLI `search_baseline/publish.rs` propagates `BaselinePublisher::publish(...)?` before `populate_serving_semantic_with_progress`; the fixture's snapshot publication count is not represented as remote semantic-marker evidence.

- T8 / R3 S8–S9, R4 S11: typed failures flow through existing workspace, overlay and reference owners. `cargo test -p mcp-server payload_lifecycle --lib` PASS (4): actual reference lexical fallback plus retained safe cause, invalid enabled config, main/overlay owner separation and reset at admitted attempt, fence/old-worker protection. `cargo test -p mcp-server tools::search::status::tests --lib --no-fail-fast` PASS (16 including current-failure matrix): status optional field is independent of lexical ready/busy/loading and uses overlay-before-main selection in both profiles without a new provider probe.

- T9 partial executed evidence: `cargo test -p mcp-server payload_ --lib` PASS (9 including two complete-envelope/minimum-budget checks, native/schema parity and actual safe builder/status/error outputs). `cargo test -p mcp-server contract::tests --lib` PASS (13); `cargo test -p mcp-server --test contract` PASS (7 actual MCP discovery checks). Machine contract 2.3, hits/not-ready schema 5 and status 2; only both search fingerprints changed. Actual application handler/build acceptance is recorded with T11 below when complete.

- T10 / R1–R5: docs/mcp/{README,SETUP,CONTRACT,TOOLS_AND_EXTENSION}.md document the accepted byte limit, exact serialization, common caller ranges/direct single-request contract, unchanged per-request retry/response bounds, safe closed diagnostics/profile scope and complete-envelope soft minimum, more physical requests/cost and rollback. `cargo test -p mcp-server --test doc_examples` PASS (1 test validates all marked successful examples and separately validates the explicit search RPC error with the published closed failure schema). All 15 JSON blocks parse, including 5 new failure examples.

- T9 completed / R4 S10–S11, R5 S12–S13: actual `workspace_search` and `reference_search` handlers exercise lexical hits, empty and not-ready results, known build failure RPC errors, query-local RPC/fallback errors and status. Workspace and reference runtimes are separate: workspace status does not borrow reference failure; reference status preserves it. `cargo test -p mcp-server payload_mcp --lib --no-fail-fast` PASS (5 tests, 0.78 s), including exact failure-envelope budget, below-floor minimum and closed schema/error checks. Raw synthetic provider text and endpoint strings never reach the responses.

- T11 / R2–R4 S3/S6–S11: the owned joined loopback endpoint rejects bodies over 600 bytes and supplies real vectors for 12 differently sized code modules under count cap 8/concurrency 1. The real `SharedState::spawn_embed_pass` plus MCP handlers prove more physical requests than count batching alone, exact original row-ID/vector mapping, partial 413 failure with retained committed rows and no false semantic Ready, and recovery sending only still-pending rows. Actual workspace build/query failures cover hit/empty fallback; reference native recovery plus both profile handlers cover lexical availability, known-build RPC errors and query-local errors. Status and known-failure projections add no provider request. Both synthetic cases have owned temporary state, joined endpoint threads and internal Tokio deadlines. External process watchdog verification below enforces the hard runtime bound even across synchronous native calls. The combined MCP acceptance run above took 0.78 s excluding compilation.

- Pre-review native regression: `cargo test -p bsl-search --no-fail-fast` PASS (461 passed, 29 existing ignored). The all-target build also checked the search_demo config migration. Two existing overlay assertions now expect the explicit typed transport error after settlement; all their original tail, mark, unread-key and prior-entry checks remain. `python3 scripts/test-ci-test-output.py` PASS (4 wrappers, success/empty/failure output).

## Executed requirement and scenario coverage

The focused runs above and the repository gates below exercise these code/test pairs; no planned check is counted as executed evidence.

| Requirement / scenario | Implementation | Executed test evidence |
| --- | --- | --- |
| R1 S1 configuration precedence | native `EmbedderConfig::{default,request_bytes_from_env}`; MCP `embedding_config`; CLI `embedder_config` | native/MCP/CLI `payload_configuration` (3 tests across crates) |
| R1 S2 invalid configuration | config validation before semantic engine/storage opening, direct transport and empty/cached publisher/standalone entry paths | same configuration tests, both `payload_configuration_*_rejects_zero_for_empty_or_cached_work` regressions, `payload_lifecycle_reference_config_error_reaches_the_runtime_owner` |
| R2 S3 packed work sets | `Embedder::batch_ranges`; all nine inventoried caller families use ranges or the single-query guard | all native `payload_` caller families; `payload_mcp_smoke_build_failure_recovery_and_query_locality` |
| R2 S4 exact serialization boundary | `serialize_request`, `batch_ranges`, `prepare_request`, `send_request` | `payload_transport_serialized_ranges_and_exact_boundary`, `payload_transport_singleton_empty_and_optional_envelope` (captured HTTP bytes, UTF-8/escapes/model/dimensions/provider) |
| R2 S5 oversized singleton/direct request | `prepare_request` and planner singleton refusal, before retries | same transport boundary tests: no request, no truncation; local input/request classifications |
| R2 S6 vector alignment | exact response-index permutation/shape validation; original range offsets and keyed commits in engine/overlay/publisher | `payload_transport_alignment_and_safe_errors`, pending/file/reference/overlay/shared-publisher mapping tests and real MCP stored row-ID/vector comparison |
| R3 S7 later request failure | first typed error retained after worker drain/join; no complete publication; earlier durable commits preserved | pending publication, failed-file atomicity, overlay partial-cache and shared-publish failure tests; lifecycle and MCP partial 413/recovery |
| R3 S8 cancellation/fencing | existing owner/cancellation checks before requests and before committing/publishing; admitted-attempt lifecycle updates | `payload_pending_publication_owner_and_cancel_win_after_failed_request`, `payload_overlay_offlock_preserves_paid_cache_and_checks_between_requests`, MCP lifecycle owner tests; existing cancellation/fence regressions in the workspace suite |
| R3 S9 reference lexical fallback | `ReferenceCollectionReplaceOutcome::embedding_failure`; reference runtime stores typed cause separately from lexical Ready | native `payload_reference_publication_preserves_lexical_failure_then_retries_in_order`, MCP reference lifecycle and both-profile handler matrix |
| R4 S10 safe classified failure | closed native failure enum/object and stage-aware ureq classification; safe response/status/error projection | five transport tests, closed schema/native parity, actual status/handler outputs with synthetic raw provider UTF-8 text excluded |
| R4 S11 successful retry/new attempt | existing workspace/overlay/reference owners clear only their own admitted/new or successful attempt; query failure remains local | four lifecycle tests, one-body retry/non-replay transport test and actual MCP build/recovery/query-locality tests |
| R5 S12 discovery/profile coverage | search output schemas 5/2, contract 2.3, both profile routes and search fingerprints; list_platform/graph unchanged | 13 contract unit + 7 real MCP contract tests, schema parity, actual two-profile docs/status/error matrix, doc_examples |
| R5 S13 complete failure envelope budget | `failure_hits_response` sizes text plus compact structured JSON before retaining any hit; mandatory minimum retained with exhausted flag | `payload_mcp_contract_failure_budget_counts_the_complete_escaped_envelope`, `payload_mcp_contract_empty_failure_preserves_the_soft_minimum` |

## Mandatory gap correction

G1 was found by the first full workspace run: moving reference snapshot resolution before semantic configuration validation changed two existing FTS-only refusal paths. `search_docs` now keeps the original unconfigured-engine refusal before dynamic baseline resolution; the known-embedding-failure branch resolves terminal baseline errors before exposing its safe cause. The same resolver is reused without an extra probe. `cargo test -p mcp-server tools::search:: --lib --no-fail-fast` PASS (85), including both unchanged regressions and cancellation/response/budget tests. Full gates after this correction are recorded below.

## Pre-review repository gates

After G1 correction, `cargo fmt --all -- --check` and `cargo clippy --workspace --all-targets --all-features -- -D warnings` PASS. `cargo test -p mcp-server payload_mcp --lib --no-fail-fast` PASS (5, 0.75 s). `cargo test --workspace` PASS (9963 tests passed, zero failed, 66 existing ignored across 271 Cargo targets including doc-tests). This closes G1's full regression evidence; the original two semantic-validation tests were retained unchanged.

The two actual MCP fixture tests were then executed individually with `timeout --kill-after=2s 30s <current-mcp-test-executable> <exact-test-name> --exact --nocapture`, using the existing binary from the successful workspace build. Both executed exactly one passing test, combined elapsed runtime 0.41 s excluding compilation. These external watchdogs enforce the per-case limit across synchronous code too; the two-case maximum is below the 180-second suite ceiling. No provider/customer endpoint was used.

## Independent implementation reviews

Independent read-only ponytail-review completed: zero mandatory complexity findings across native transport/callers, MCP lifecycle/wire and owned fixtures. One optional consolidation of identical producer/worker join handling could remove 19 lines; it adds no missing behavior and was left unchanged. No dependency, executor, observer, persistence layer or configuration outside the locked requirement scope was introduced.

The added byte setting serves R1; the shared planner, exact guard and response validation serve R2; first-error propagation through existing workers/publication owners serves R3; the closed failure value and existing-owner lifecycle projection serve R4; schema mirrors, failure-aware envelope rendering and contract examples serve R5. The new local fixtures are the required V1–V11 acceptance evidence, not production services.

First independent implementation-versus-plan review: native/CLI found G2 (R1 S2); MCP/lifecycle/wire found zero mandatory issues and one optional status wording inconsistency (G3). Native review otherwise confirmed all nine caller families and R2–R4; MCP review confirmed R1/R3–R5, including request cancellation isolation from shared build owners. Both reviews read actual code and tests without mutations.

G2 correction: the generic publisher invokes its existing planner for empty input before any identity/store or empty/cached success path; the native override validates the configuration. Standalone overlay validates its supplied config before Store open/planning. Two direct-library regressions failed before these guards, then passed for empty and fully cached inputs, with positive valid-config controls and zero HTTP. `cargo test -p bsl-search payload_configuration_ --lib --no-fail-fast` PASS (3); `cargo test -p bsl-search payload_ --lib --no-fail-fast` PASS (19).

G3 correction changes only the embedding-failed overlay sentence to describe the existing lexical fallback. Structured diagnostics and search behavior are unchanged. `cargo test -p mcp-server tools::search::status::tests --lib --no-fail-fast` PASS (16). Repeat reviews and final gates will verify these corrections before completion.

## Repeat independent reviews after corrections

Repeat read-only ponytail-review returned zero mandatory findings. Repeat native/CLI review confirmed G2 closed and R1–R4 preserved, including actual empty/cached negative and valid positive controls without HTTP. Repeat MCP/lifecycle/wire review confirmed G3 closed and no mandatory or optional issues in its scope; R1/R3–R5, cancellation isolation, owner/error semantics, budgets and versions are unchanged. No additional gap remains. The optional join-loop shortening from complexity review is left unchanged.

## Final verification

Completeness: all 12 original tasks and G1–G5 are checked; all 5 requirements and 13 scenarios have the executed Code/Test mappings above. Correctness: independent reviews and targeted regressions found no unresolved requirement divergence. Coherence: the implementation follows the locked planner, existing execution/runtime owners, exact version and budget policies; no new dependency or persistent infrastructure. The final repository gates passed on the unchanged reviewed source; their corrected totals and bounded runtime evidence are recorded below.

G4: final clippy rejected `err().expect()` in the new G2 standalone regression. A direct `let Err(error) = result else { panic!(...) }` preserves the assertion without adding a Debug implementation solely for tests. No production behavior changed. Configuration regressions PASS (3); fmt PASS; full workspace/all-targets/all-features clippy with `-D warnings` PASS (4.75 s). Repeat reviews cover this final test-only correction.

After G4, independent ponytail-review again returned GO with zero mandatory findings; native implementation-versus-plan review confirmed the preserved negative/positive controls and zero mandatory findings. Source manifest comparison confirmed production/MCP source unchanged since their clean full reviews; only the reviewed regression changed. Final source manifest: 30 changed/new Rust files, SHA-256 `031d096080aae52a5836739d923a3496a9e8d7c99051a1ed4db574c94149f7e0`.

G5: the first report collector incorrectly summed child-test summaries together with their containing Cargo target. The corrected one-off collector takes each target’s final unfiltered summary, rejects incomplete/failed targets and has a synthetic nested-summary regression. The earlier pre-review run is 9963 passed (not the initially reported 9971); the final run is **9965 passed, zero failed, 66 existing ignored across 271 Cargo targets (238 nonempty)**. Cargo itself exited successfully in both runs. No production or test source changed for this correction.

Final bounded MCP acceptance: both exact current-binary cases PASS under external 30-second watchdogs, combined elapsed 0.41 s excluding compilation, below the 180-second suite limit. Every required payload fixture family executed nonzero tests. Source SHA-256 still matches the reviewed final manifest. Strict OpenSpec validation PASS. Independent ponytail and implementation/evidence reviews confirmed the corrected report totals, unchanged source manifest and zero mandatory findings.

## Final assessment

| Dimension | Verified result |
| --- | --- |
| Completeness | 17/17 tasks: 12 original plus G1–G5; 5/5 requirements and 13/13 scenarios mapped to current Code/Test evidence. |
| Correctness | Native/CLI and MCP independent reviews are clean after corrections; all required positive/negative fixture families ran nonzero tests. |
| Coherence | Locked D1, linear shared planner, existing worker/publication/lifecycle owners, compatible schemas and minimum budget policy preserved. |

Final checks: fmt PASS; workspace/all-targets/all-features clippy with `-D warnings` PASS; workspace tests **9965 passed / 0 failed / 66 existing ignored**, 271 Cargo targets (238 nonempty); two externally bounded MCP cases PASS in 0.41 s excluding compilation; contract/discovery/doc examples covered by the successful workspace run; strict OpenSpec validation PASS. Tests ran on the final unchanged Rust source manifest shown above.

CRITICAL/mandatory findings: zero. Unverified mandatory risks: zero. Optional suggestion: consolidate identical producer/worker join handling (up to 19 fewer lines); this is cosmetic and does not affect the result. Qualification is the locked local Linux native/MCP scope; Windows/provider/remote SQL qualification and downstream deployment are outside it. Smaller byte ceilings can increase request count/time/cost as documented. At the implementation handoff, no archive, installation, release, commit or push had been performed.

## Publication handoff — 2026-09-12

The user requested a PR, authorizing one scoped commit and publication of `feat/bound-embedding-batches-by-payload`. The reviewed 30-file Rust manifest is unchanged, so the final implementation checks above apply to the published code. Live `upstream/develop` remains `7133432d`; the current parent `fa4693c0` is the open draft PR #142, which itself includes the earlier PR #119 changes. Publish this change as a draft against `itrous/bsl-analyzer:develop`, explicitly identifying #142 as its landing dependency and linking the payload-only diff from `fa4693c0`. No archive or release is part of this publication.

## PR #148 CI repair — 2026-09-12

Run `34686984149` on `047744180e7df064b482d5035b3e93e772598a82` passed Linux Check but failed Windows lifecycle tests. The shared native `PayloadServer` accepted from a nonblocking listener, then attempted blocking-style reads without resetting the accepted stream mode. Windows reported `WouldBlock` (10035) at `payload_tests.rs:44`; the server thread panicked and the reference recovery test later failed `recovered.written`. All native payload callers share this fixture. The separate MCP endpoint uses a blocking listener and does not have this mode mismatch.

G6 explicitly sets the accepted native stream to blocking mode before applying the existing two-second read/write timeouts. Listener polling, shutdown, the watchdog, assertions and production transport/retry/publication behavior remain unchanged. The user authorized scoped fixes and repeated commit/push until CI succeeds. Local correction checks PASS: exact reference recovery (1), all native payload tests (19), Windows-job lifecycle selection (44), fmt and `cargo clippy -p bsl-search --all-targets --all-features -- -D warnings`. Independent read-only ponytail and implementation-versus-plan reviews returned GO with zero mandatory findings. New remote CI is still pending; these Linux results are not represented as Windows acceptance.
