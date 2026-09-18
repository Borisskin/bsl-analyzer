## Scenario Traceability

Every spec scenario has exactly one row. Identifiers are planned until implementation; an exact test filter that executes zero tests fails the gate.

| # | Requirement / scenario | Task | Exact planned evidence | Platform |
|---:|---|---:|---|---|
| 1 | Lease operations expose explicit profiles / Prepared value is published atomically | 1.3 | `workspace_lease::tests::publish_short_restamp_failure_skips_commit_and_success_commits_once` | Linux, Windows |
| 2 | Lease operations expose explicit profiles / Atomic mutation requires multiple checkpoints | 1.4 | `workspace_lease::tests::checkpointed_atomic_publish_rolls_back_at_boundary` | Linux, Windows |
| 3 | Lease operations expose explicit profiles / Graph data request reads a resident snapshot | 4.1 | `tools::graph::tests::graph_data_requests_are_pool_only_under_held_lease`; `graph_supersession_contract::resolve_names_misses_immediately_when_preopened_handles_are_busy`; `graph_supersession_contract::graph_handler_misses_immediately_when_preopened_handles_are_busy`; `graph_supersession_contract::symbol_info_misses_immediately_when_preopened_handles_are_busy`; `graph_supersession_contract::references_misses_immediately_when_preopened_handles_are_busy` | Linux, Windows |
| 4 | Lease operations expose explicit profiles / Search request reads resident state | 4.2 | `tools::search::tests::all_search_modes_are_resident_only_under_held_lease` | Linux, Windows |
| 5 | Lease operations expose explicit profiles / Status request reads cached state | 4.3 | `tests::all_status_requests_are_cached_under_held_lease` | Linux, Windows |
| 6 | Fenced outcomes preserve their origin / Operation fails after admission | 1.2 | `workspace_lease::tests::callback_error_preserves_operation_origin` | Linux |
| 7 | Fenced outcomes preserve their origin / Lease lock is temporarily unavailable | 1.1 | `workspace_lease::tests::contention_unclaimed_and_missing_are_transient` | Linux, Windows |
| 8 | Fenced outcomes preserve their origin / Managed lease I/O fails | 1.1 | `workspace_lease::tests::managed_lease_io_error_is_not_transient` | Linux, Windows |
| 9 | Fenced outcomes preserve their origin / A live foreign token is observed | 1.4 | `workspace_lease::tests::checkpoint_observes_live_foreign_token_and_rolls_back` | Linux, Windows |
| 10 | Fenced outcomes preserve their origin / Shutdown prevents admission | 1.4 | `workspace_lease::tests::pre_admission_release_skips_callback` | Linux, Windows |
| 11 | Fenced outcomes preserve their origin / Release interrupts checkpointed work | 1.4 | `workspace_lease::tests::release_during_checkpointed_callback_rolls_back_as_terminal` | Linux, Windows |
| 12 | Fenced outcomes preserve their origin / Supersession precedes release | 1.4 | `workspace_lease::tests::first_terminal_cause_survives_release` | Linux, Windows |
| 13 | Lock holding and shutdown latency are bounded / Long refresh exceeds the stale interval | 1.4 | `workspace_lease::tests::checkpointed_operation_outlives_stale_after_without_self_supersession` | Linux |
| 14 | Lock holding and shutdown latency are bounded / Release arrives during a bounded batch | 1.3 | `workspace_lease::tests::release_waits_for_only_the_admitted_batch_and_refuses_the_next` | Linux, Windows |
| 15 | Lock holding and shutdown latency are bounded / Release arrives during an indivisible transaction | 1.4 | `workspace_lease::tests::a_throttled_checkpoint_still_reports_a_release` | Linux, Windows |
| 16 | Lock holding and shutdown latency are bounded / Manifest and fingerprint transitions span many rows | 2.5 | `engine::tests::manifest_and_fingerprint_transitions_checkpoint_and_rollback` | Linux, Windows |
| 17 | Every retrying obligation has a fixed deadline / Startup lock remains contended | 3.2 | `state::bootstrap::tests::startup_lease_retry_stops_at_600_seconds` | Linux |
| 18 | Every retrying obligation has a fixed deadline / Fresh work coalesces during retry | 3.1 | `state::retry_window::tests::all_retry_owners_preserve_deadline_and_streak_when_coalescing` | Linux |
| 19 | Every retrying obligation has a fixed deadline / Work arrives after deadline exhaustion | 3.1 | `state::retry_window::tests::eligible_background_owners_rearm_only_on_fresh_external_work` | Linux |
| 20 | Every retrying obligation has a fixed deadline / Permanent operation error occurs | 3.1 | `state::retry_window::tests::all_retry_owners_stop_on_operation_error` | Linux |
| 21 | Every retrying obligation has a fixed deadline / Heartbeat is transiently refused | 2.1 | `workspace_lease::tests::heartbeat_refusal_waits_for_normal_tick` | Linux, Windows |
| 22 | Drift and snapshot failures preserve progress and recovery debt / A second change follows a durable failure | 4.5 | `state::sync::tests::durable_drift_error_advances_cursor_and_coalesces_debt` | Linux, Windows |
| 23 | Drift and snapshot failures preserve progress and recovery debt / Topology marking fails | 4.5 | `state::sync::tests::durable_drift_error_advances_cursor_and_coalesces_debt` (combined end-to-end scenario) | Linux |
| 24 | Drift and snapshot failures preserve progress and recovery debt / A removed path is recreated before recovery | 4.5 | `state::sync::tests::durable_drift_error_advances_cursor_and_coalesces_debt` (combined end-to-end scenario) | Linux, Windows |
| 25 | Drift and snapshot failures preserve progress and recovery debt / Background snapshot preparation has a classified outcome | 4.4 | `graph::snapshot::tests::background_snapshot_preserves_all_typed_outcomes` | Linux, Windows |
| 26 | Local contention does not repeat paid embedding work / Ownership is refused before network work | 4.6 | `state::embed::tests::failed_typed_preflight_makes_zero_network_calls` | Linux |
| 27 | Local contention does not repeat paid embedding work / Publication is refused after vectors are returned | 4.6 | `state::embed::tests::prepared_vectors_survive_transient_publish_without_second_call` | Linux |
| 28 | Local contention does not repeat paid embedding work / Embedding deadline is exhausted | 4.6 | `state::embed::tests::publication_deadline_moves_runtime_to_failed` | Linux |
| 29 | Graph build retry follows the original failure / Ownership returns after transient refusal | 4.7 | `graph::build::tests::original_transient_arms_withheld_build_until_trigger` | Linux |
| 30 | Graph build retry follows the original failure / Build operation fails while ownership probe is negative | 4.7 | `graph::build::tests::operation_error_waits_for_fresh_work_without_losing_its_origin` | Linux |
| 31 | The complete caller set is regression-protected / Removed generic API remains referenced | 1.6 | `inventory::no_generic_or_unclassified_production_lease_callers` | Linux |
| 32 | The complete caller set is regression-protected / Portable exact filters run in CI | 5.3 | `workspace_lease::tests::contention_unclaimed_and_missing_are_transient`; `workspace_lease::tests::a_throttled_checkpoint_still_reports_a_release`; `graph::snapshot::tests::path_identity_detects_equal_size_and_time_replacement`; `tools::graph::tests::graph_data_requests_are_pool_only_under_held_lease`; `tools::search::tests::all_search_modes_are_resident_only_under_held_lease`; `tests::all_status_requests_are_cached_under_held_lease` | Linux `Check`, Windows `MCP transports + secure broker` |

## Supplemental Inventory Gates

| Gate | Task | Exact planned evidence |
|---|---:|---|
| Fence callers remain exact and request paths remain lease-free | 2.11 | `inventory::fence_callers_are_exactly_classified`; `inventory::request_paths_do_not_call_lease_or_mutation_helpers` |
| Source inventory works with platform checkout conventions | 7.1 | `inventory::production_prefix_handles_checkout_line_endings`; all inventory filters in Linux and Windows CI |
| Prepared index survives swap contention and retains fresh work | 7.3 | `state::embed::tests::prepared_index_survives_transient_swap_without_rebuild` |
| Runtime and compatibility surface is unchanged in this PR | 5.4 | One-time review of the PR diff against its merge base; not a permanent unit test |

## Required Commands

- Run each Rust identifier above with an exact or fully qualified filter and confirm a non-zero test count.
- Run all `inventory::*` identifiers above and fail on any match outside the documented allow-list. These tests only read checked-out Rust sources: Git history, Git executables, and a fixed `Cargo.lock` are not prerequisites. Review this PR's dependency/runtime/schema/configuration/wire diff separately against its merge base; future intentional changes are not forbidden by this PR's historical scope.
- In Linux `Check` and Windows `MCP transports + secure broker`, run each of the six portable identifiers in row 32 as `cargo test -p mcp-server <identifier> -- --exact` and require a non-zero test count.
- `cargo fmt --all -- --check`
- `cargo clippy -p bsl-search -p mcp-server --all-targets --all-features -- -D warnings`
- `cargo test -p bsl-search --no-fail-fast`
- `cargo test -p mcp-server --no-fail-fast`
- `actionlint .github/workflows/ci.yml`
- `git diff --check`
- `openspec validate enforce-workspace-lease-operation-profiles --strict --no-interactive`

## Post-handoff Delivery Checks

After a later explicit publication request, match the PR head SHA to the locally verified commit and observe the Linux/Windows jobs at that SHA. Maintainer acceptance and issue closure are repository delivery decisions, not evidence that the local implementation itself is complete.

## Current Status

PR head `949e30bb` was published and passed Linux and Windows CI on 2026-09-04.
The 2026-09-08 follow-up for inventory portability and prepared-index retention is implemented
locally; nine focused regressions pass on Rust 1.98. Full MCP verification is not green because
of inotify exhaustion and a failing forced-retry references test. Publication and native Windows
CI for this follow-up remain pending.

## Evidence Collected

- Baseline: initial implementation started from `f8bf4da5831840070aa19477be68e74d78014fa6`; on 2026-09-04 the branch was rebased onto v0.2.77 integration base `edc78e22f3efbfe51ffd8e6dfd05b457976195ca`; `git merge-base --is-ancestor 75b8a978 HEAD` passed.
- Caller inventory: seven production files referenced the legacy ownership APIs and ten files referenced `LeaseOutcome` or `FenceOutcome` before implementation.
- Traceability: an exact requirement/scenario-to-matrix comparison passed with 32 scenarios, 32 unique rows, no duplicate scenario, and every row linked to an existing task.
- Pre-implementation strict validation: `openspec validate enforce-workspace-lease-operation-profiles --strict --no-interactive` passed.
- Pre-implementation code baseline: `workspace_lease` 24 tests, graph snapshot 19, graph build 85, bootstrap 35, sync 52, embed 25, and overlay retry 16 all passed; `cargo fmt --all -- --check` and strict Clippy for `bsl-search` plus `mcp-server` passed with no pre-existing failure.
- Lease core: the final `workspace_lease` suite passed, including exact five-way classification, callback/lease error provenance, pre-commit restamp with zero commit calls on failure, checkpoint rollback, first-terminal precedence, same-token liveness, real foreign-token takeover, and release latency filters. The existing cross-fence restamp throttle regression also passed after separating forced short restamp from checkpointed throttling.
- Post-review lease correction: checkpointed work now yields and reacquires the OS lock at every
  cooperative boundary, revalidates the real lease record, and retains the reacquired guard through
  the next batch/commit. The deterministic real-record takeover test passed and the final lease
  module suite passed.
- Outcome adapters: `bsl-search` has no `FenceOutcome::Terminal`; its full suite passed with 403 tests and 29 ignored. `mcp-server --all-features` compiled both production and all test targets after exhaustive host migration to distinct `Superseded` and `Released` variants.
- Legacy surface and heartbeat: the exact `inventory::no_generic_or_unclassified_production_lease_callers` filter executed one passing test and `rg` found no legacy ownership API or `LeaseOutcome` reference under `mcp-server/src`; the exact heartbeat refusal filter passed and proves one transient miss is retried only by the next explicit tick.
- Graph profiles: snapshot, build, and state module suites passed (19, 85, and 28 tests). Exact typed snapshot/path identity and original transient/operation provenance filters each executed one passing test; temporary graph work is off-fence, prepared installs use short publication, and fused ingest uses checkpointed 64-row boundaries.
- Search/store publication: the exact prepared-drift rollback, manifest/fingerprint rollback,
  root migration, FTS rebuild, and fused-file rollback tests passed. Drift advances each cursor
  only after an `Applied` 64-row slice; startup manifest save/clear and external-baseline mode
  changes now use checkpointed transactions.
- Retry ownership: the three exact owner-table tests passed for startup, change-hub, drift,
  overlay/embedding, and graph owners. Startup uses the locked 600-second/2-second policy;
  drift retains prepared work within one deadline and converts durable failure into one
  independently backed-off current-disk rescan debt. A dormant change-hub sink remains visibly in
  `Watcher mode: polling`; operation failure records one local rescan debt, and only a fresh hub
  batch creates the next enable budget.
- Request paths: the exact graph pool-only, all-search resident-only, and all-status cached-only
  tests each executed once and passed. The four busy descriptor-pool tests also passed; status
  uses cached lease/standalone extension/external-object state and search only coalesces a
  background refresh.
- v0.2.77 cancellation integration: six search cancellation gates and both workspace/reference
  handler-matrix tests passed. `search_call` and `CancellationToken` remain on the request path,
  while source inventory and the held-lease regression confirm that cancellation adds no lease
  access, synchronous resident prefetch, or refresh wait.
- Embedding: all three paid-work exact tests passed, proving zero network calls after failed
  preflight, one paid call across transient publication refusal, and `Failed` on deadline.
- Exact matrix: every listed scenario, handler, and supplemental-inventory identifier executed at
  least one test and passed after the final integration changes.
- Full gates: `bsl-search` passed 405 tests with 29 ignored; the final `mcp-server` rerun passed
  992 library tests with 1 ignored plus every integration target. Strict Clippy, rustfmt check, `git diff --check`,
  `actionlint`, all three inventory gates, and strict non-interactive OpenSpec validation passed.
- Full local CI follow-up: workspace-wide `RUSTFLAGS='-D warnings' cargo clippy
  --all-targets --all-features` and `RUSTFLAGS='-D warnings' cargo test --all --no-fail-fast`
  both exited successfully. All four `partitioned-baseline-scale` workflow commands then passed
  in release mode, executing five 1.6M-entry tests in total. Windows runner behavior remains a
  live-CI gate because it cannot be reproduced on this Linux host.
- Full-suite repair: the first MCP run exposed a broker takeover timing race after request-time
  ownership refresh was removed. The broker now refreshes ownership on its existing background
  tick even while a session is active; the broker suite then passed 8/8 and the complete MCP
  suite passed on rerun.
- Independent reviews: Ponytail reported no blocker and only optional deduplication/deletion ideas.
  The implementation-vs-plan review's checkpoint takeover blocker was fixed with real lock yield,
  reacquisition, record validation, and rollback; its degraded-state observation and real-handler
  evidence gaps were closed with the existing polling status, rescan debt, and four exact busy-pool
  handler tests.
- Maintainer review: all five blocking findings were closed. Broker accept reads only the cached
  terminal bit; CI exact filters reject zero executed tests; startup retries rerun the transaction;
  graph operation/changed failures wait for genuinely fresh work; alias tests were deleted. The
  held-lock status/graph tests and exact fence/request-path inventories close MID-1, MID-2, and MID-4.

## 2026-09-08 Scoped Follow-up Evidence

- Inventory no longer invokes Git or compares a lockfile to a historical SHA. All four tests
  also passed in a source-only fixture with no `.git`, CRLF Rust files, and a deliberately changed
  `Cargo.lock`. Native `Path` comparison replaces platform-dependent string comparison.
- The prepared-index regression failed on the original loop: an apply event appeared between
  two swap attempts. It passes after the fix, with and without a pending rerun; the real lease
  lock causes refusal, only the swap is retried, and the paid embedding endpoint is called once.
- Nine non-zero exact filters passed on Rust 1.98: all four inventory tests, prepared-index
  retention, paid-vector retention, publication deadline, supersession, and the mid-flight rerun.
  Linux and Windows CI now include the inventory and prepared-index filters.
- `bsl-search` passed 405 tests with 29 ignored. The full MCP run was not accepted: its library
  had 822 passed / 171 failed / 1 ignored, and integration targets had 154 passed / 1 failed.
  A library-only rerun with four test threads still had 145 failures; reducing concurrency did
  not remove the environment problem.
- Temporary diagnostic output identified notify `MaxFilesWatch`. The host had 1,048,405 visible
  watch references against a per-user limit of 1,048,576. The diagnostic changes were removed;
  system limits and other processes were not changed. The poisoned-lock failures in the broad
  run do not replace the separate, passing focused regression results.
- The `references` integration target's `a_name_declared_after_the_last_scan_is_found_on_a_forced_retry`
  test also failed in isolation: a newly added method returned `not_found` with graph `not_ready`. Its cause has not
  been established by this follow-up; no references or watcher production code was changed.
- Rust 1.98 is selected for both Cargo and its child tools by explicit toolchain paths; selecting
  Cargo alone picked up a different `rustc` from this host's PATH. Clean workspace
  `RUSTFLAGS='-D warnings' cargo clippy --all-targets --all-features` passed with Clippy 0.1.98,
  using separate target and temporary directories on the workspace disk. Rustfmt 1.98,
  workflow actionlint, strict OpenSpec validation, and `git diff --check` passed.

## 2026-09-09 Final Follow-up Verification

- The host inotify shortage was corrected in the active VS Code profile outside this repository.
  The forced-retry references test passed unchanged both alone and in the complete suite.
- The first post-inotify suite exposed a separate, reproducible graph build collision: overlapping
  backend generations within one process used the same PID-only temporary SQLite path. Tracing
  captured `disk I/O error` and `attempt to write a readonly database` during their concurrent
  reloads. Temporary tracing instrumentation was removed after diagnosis.
- Full and incremental graph builds now share a PID-plus-build-counter path allocator. The
  superseded-build regression failed deterministically with the old PID-only path, then passed
  with the fix; it also proves refusal and fused failure leave another build's file untouched.
  No dependency, worker, persistent cache schema, lease record, configuration, or wire change
  was introduced.
- The final Rust 1.98 `cargo test -p bsl-search -p mcp-server --all-features --no-fail-fast`
  passed: `bsl-search` 405 passed / 29 ignored, MCP library 993 passed / 1 ignored, and all
  integration targets passed, including `superseded_daemon_lifecycle` and the references retry.
- Strict workspace Clippy (`RUSTFLAGS='-D warnings' cargo clippy --all-targets --all-features`)
  passed after the graph fix. Formatting, workflow lint, strict OpenSpec validation, and diff
  checks passed. The earlier non-zero exact-filter and source-only/CRLF inventory evidence
  remains applicable; the final library suite includes the inventory and prepared-index tests.

## 2026-09-09 CI Fixture Follow-up

- CI on `c123df62` passed Windows and the Linux formatting, Clippy, and exact-filter gates.
  The full Linux suite exposed `names_resolve_before_the_graph_exists`: its fixture assumed
  the background graph would still be unavailable after the independent resident became ready,
  but the graph provider had already reached `answered`.
- The four cold-graph name-dictionary scenarios now hold the real cache lease lock before
  backend construction. Dictionary answers remain available while graph publication is
  withheld; the two positive controls release the lock and still require `answered` afterward.
  All six `name_dictionary` transport tests passed with the deterministic fixture. Production
  behavior is unchanged. Windows CI explicitly runs this suite to verify its lock behavior too.

## 2026-09-11 Maintainer Rebase

The user approved rebasing and updating PR #119 onto current `upstream/develop`.
The original head `03fcb358f591ffdc8b791550de245f5966b508de` is retained at
`backup/pr119-before-rebase-20260911`; the new base is
`7133432d276d7fb1f5e21168e0cc59ff7f8b861e` (v0.2.79).
Work takes place in a separate worktree, preserving PR #142 and unrelated local changes.

The name-dictionary conflict keeps upstream provider-identity assertions and the
held-lease fixture, including both publication-after-release controls. Four embedding
tests use upstream's existing `env_lock()` helper. Three diagnostics helpers have the
same Unix gate as their only caller. The selective baseline LSP fixture publishes
complete snapshots atomically, avoiding the intermediate empty-file error epoch already
reproduced and fixed during PR #142 verification. Production scope is unchanged.
Validation of this rebased tree follows; previous dated results are historical evidence.
