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
| Source inventory works with platform checkout conventions | 7.1 | `inventory::production_source_handles_checkout_line_endings`; `inventory::production_source_keeps_the_request_handlers_of_lib_rs`; `inventory::production_source_drops_the_test_modules_of_real_sources`; `inventory::the_request_path_gate_fails_on_an_injected_lease_call`; all inventory filters in Linux and Windows CI |
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

PR head `949e30bb` was published and passed Linux and Windows CI on 2026-09-04; the
2026-09-09 follow-up head `03fcb358` passed both jobs as well. The inotify exhaustion and the
forced-retry references failure recorded for the 2026-09-08 working copy were resolved before
that head and no longer describe this change.

The state after the 2026-09-10 and 2026-09-11 integration rounds is recorded in the sections of
those names below: the inventory gate now reads the production half of every source rather than a textual
prefix, the checkpointed adapter and the startup transactions are re-runnable, the workspace
overlay and the graph have a background owner in local mode, and `graph status` no longer
reports a graph with a catch-up owed as fresh. Windows remains verified by CI only.

The 2026-09-12 drift contract section below replaces the search sink as the owner of the graph
trigger and of the overlay's dirty marks; its evidence is local (macOS) only and no CI has run
on it yet.

## 2026-09-10 Integration Review

Reviewed as a cherry-pick onto `origin/develop` `2f1737a4`, with two independent gates: an Opus
read of the whole delta and a full onecpi implementation round. Both are recorded outside this
repository, in the task's review artefacts.

Confirmed and repaired in this working copy:

- the source inventory read a textual prefix, which in `lib.rs` ended at the `#[cfg(test)] mod`
  DECLARATION on line 13 — twelve of four thousand lines. It now removes each `#[cfg(test)]`
  element and keeps the rest, with anchors on the real sources and a positive control that
  injects a request-time lease call and requires the gate to fail;
- the checkpointed fenced adapter consumed its operation on the first attempt, so the host's
  retry met `checkpointed fenced apply operation invoked more than once` instead of the rolled
  back transaction. The adapter is re-runnable and the startup transactions no longer mutate
  process state before their first checkpoint;
- in `SqliteLocal` the workspace overlay had no owner for its dirty marks and the graph had no
  drift trigger for a `.bsl`-only edit. The search sink owns both now, in bounded units, with
  the same retry budget as the drift it follows, and `graph status` reports a graph with a
  catch-up owed as stale.

Local verification is macOS only. Windows and Linux remain covered by CI at the published head;
`actionlint` and `openspec validate --strict` are not installed on the review host and are
recorded as not run rather than as passing.

## 2026-09-11 Second Integration Round

A second full onecpi implementation round over the same delta found five blocking issues in the
repairs above and thirteen smaller ones. What that round changed:

- the overlay drain held the engine lock for a whole workspace refresh and wrote
  `overlay_fingerprint_cache` rows outside the fence. It is bounded to 64 paths per call and runs
  inside `publish_short`, so a superseded generation cannot touch those rows;
- a request-side coalesce still moved the driver's fresh epoch, which cleared the backoff of the
  pass already running. It only wakes the driver now;
- `graph status` reported a snapshot from a replaced revision as fresh;
- the change-hub sink measured "fresh work" against a generation it had never waited on, so a
  batch delivered INSIDE an exhausted admission budget opened the next one. The dormant wait
  synchronises with the hub's generation first, and only later work opens a budget;
- the standalone advisory was moved off the request path onto the search sink, which does not
  exist for every backend — where it does not, the advisory froze at boot. The slot now says
  whether anyone is watching it, and a read derives the advisory itself when nobody is. That
  derivation takes no lease, no shared-path open and no interprocess wait, which is what a
  status request is barred from;
- the graph was the one retry owner with a deadline and no delay: every lifecycle call restarted
  a full rebuild. It now holds an earliest-next instant from the same backoff schedule as the
  other owners, and a lifecycle call that starts nothing spends neither the budget nor the drift
  signal that `graph status` reports;
- a panic in the graph builder left its temp database behind for the life of the cache directory,
  because each build now takes a unique name. The temp file is released on every exit;
- the broker's idle poll clamp, lowered from 15s to 1s, is restored: no test needs it and it
  multiplied every backend's wakeup rate.

Local verification is macOS only, on the same terms as the section above.
`crates/bsl-analyzer/tests/partitioned_diagnostics_baseline_lsp.rs` fails intermittently on this
host (once in three runs in isolation) with a diagnostics publication observed before the
baseline is applied; every crate that test exercises is byte-identical to `2f1737a4`, so it is
recorded as a pre-existing flake and not as a result of this change.

## 2026-09-12 Drift Contract

The change hub is the one transport for workspace facts; the search consumer (cursor `S`) and a
graph watcher (cursor `G`, started with the graph and independent of search) read it on their
own. Context marks are consumed only by a graph publication that observed their fact (the hub
sequence read before its pre-scan); marks no publication observed owe a forced project reload.
A hub whose watch cannot be set up polls (one reconcile, then stat walks with a bounded rolling
content check). The overlay's dirty marks have an owner of their own (`OverlayBacklog`), which
prepares each batch off every lock and publishes it in one short transaction under a 100 ms
busy timeout. Every hold of the search engine goes through a FIFO admission queue. `graph` and
`search_code` answers carry `freshness.drift_watch`; the graph reads stale while nobody watches.
MCP `contract_version` 2.3 (minor): `search_code` schema `5`, `graph` schema `34`; the reference
profile's `search` schema and answers are byte-identical (fingerprint unchanged in the
contract snapshot), and the reason vocabulary gained no code.

| Scenario | Evidence | Platform |
|---|---|---|
| Healthy quiet start costs no reconcile, mark or reload | `state::bootstrap::tests::a_quiet_healthy_boot_asks_for_no_reconcile_mark_or_reload` | Linux, Windows |
| Watch setup failure: one reconcile, then polls | `change_hub::tests::a_hub_without_a_watch_polls_after_one_reconcile` | Linux, Windows |
| Marks consumed only by an observing publication, every order | `graph::state::tests::marks_are_consumed_by_an_observing_publication_in_every_order` | Linux, Windows |
| Late search publication after a config edit | `graph::watcher::tests::marks_placed_after_the_graph_caught_up_on_its_own_are_consumed_at_once` | Linux, Windows |
| Graph follows edits with search init failed / hung | `state::bootstrap::tests::the_graph_follows_edits_after_search_init_failed`; `graph::watcher::tests::a_stalled_search_cursor_costs_the_hub_its_capacity_at_most_and_the_graph_nothing` | Linux, Windows |
| No watch: both consumers follow edits through the poll | `state::bootstrap::tests::a_workspace_without_a_watch_follows_edits_through_the_poll` | Linux, Windows |
| Same-stat edit under polling | `change_hub::tests::a_same_stat_edit_is_found_by_the_content_check_once` | Linux, Windows |
| Owed marks force a reload of an unchanged graph | `graph::watcher::tests::owed_marks_force_a_rebuild_of_an_unchanged_graph` | Linux, Windows |
| Supersession: owners leave, cursors released, nothing applied | `state::bootstrap::tests::a_superseded_daemon_applies_nothing_more_and_releases_its_cursors` | Linux, Windows |
| Shutdown stops every owner within a second | `state::background_lifetime_tests::shutdown_stops_every_owner_within_a_second` | Linux, Windows |
| Backlog drains without pauses; busy store refuses; point and full plans coexist | `state::overlay_backlog::tests::a_large_backlog_drains_without_pauses_between_batches`; `state::overlay_backlog::tests::a_held_writer_lock_refuses_the_commit_and_a_stop_is_answered_during_the_backoff`; `state::overlay_backlog::tests::point_batches_and_full_plans_do_not_roll_each_other_back` | Linux, Windows |
| Engine admission is FIFO with one raw waiter | `tools::search::acquire::tests::acquisitions_are_served_in_ticket_order_with_one_raw_waiter`; `inventory::the_search_engine_is_held_only_through_its_admission` | Linux, Windows |
| Point path shape (no whole-baseline load, no read under the lock) | `point_refresh::structure::the_point_path_keeps_its_shape`; `point_refresh::structure::the_point_path_gate_reads_a_crlf_checkout`; `point_refresh::tests::a_bounded_publication_refuses_whole_baseline_loads_and_phase_c_needs_none` | Linux, Windows |
| Watch and freshness are reported | `state::bootstrap::tests::the_search_and_the_graph_say_who_watches_them`; `graph::state::tests::freshness_rests_on_the_drift_watch`; `state::sync::tests::a_consumer_blocked_on_its_first_fence_is_not_yet_watching`; `state::consumer_phase_tests::a_consumer_whose_thread_never_ran_is_abandoned` | Linux, Windows |
| A failed reload stays owed, behind its backoff, and is retried with no further event | `graph::build::tests::a_failed_reload_is_owed_behind_its_backoff`; `graph::watcher::tests::a_failed_reload_is_retried_without_another_event` | Linux, Windows |
| Owed marks stop rebuilding when their budget is spent; a reconcile owes a project reload | `graph::state::tests::owed_marks_stop_rebuilding_when_their_budget_is_spent`; `graph::watcher::tests::a_reconcile_owes_a_project_reload_in_every_live_state` | Linux, Windows |
| The poll's content check keeps its budget and misses no edit before a first read | `change_hub::tests::the_content_check_keeps_its_budget_for_a_large_file`; `change_hub::tests::a_same_stat_edit_before_the_first_read_is_found` | Linux, Windows |
| A retry that cannot start keeps a failed build owed; a refused first build is retried by its owner; fresh marks that revive a spent obligation wake the watcher | `graph::build::tests::a_failed_reload_whose_retry_is_held_stays_owed`; `graph::build::tests::a_refused_first_build_is_owed_a_retry`; `graph::state::tests::marks_that_revive_a_spent_obligation_wake_the_watcher` | Linux, Windows |
| Marks alone start no embedding pass; a starting index answers partial; a local removal hides nothing | `engine::retry_signal_ownership::marks_alone_demand_no_embedding_pass`; `tools::search::render::tests::an_index_whose_watch_is_starting_is_not_complete`; `engine::tests::a_local_drift_removal_hides_nothing` | Linux, Windows |
| Backlog activity ends with its work; a revived owner starts its schedules over; shutdown does not wait out an engine hold | `state::overlay_backlog::tests::a_drained_backlog_stops_counting_as_work_at_once`; `state::overlay_backlog::tests::a_revived_owner_starts_its_busy_schedule_over`; `state::background_lifetime_tests::shutdown_does_not_wait_out_an_engine_hold` | Linux, Windows |
| The backlog owner answers a stop between keys, retries a busy commit soon, and its wakes reach the embedding driver | `state::overlay_backlog::tests::a_stop_during_preparation_is_answered_between_keys`; `point_refresh::tests::a_cancelled_preparation_reads_no_further_key`; `state::overlay_backlog::tests::a_busy_commit_is_offered_again_soon`; `state::overlay_retry::tests::a_wake_between_the_check_and_the_sleep_is_not_lost` | Linux, Windows |
| Published answers pass their profile's schema; surface snapshot | `tools::search::render::tests::answers_pass_the_schema_their_profile_publishes`; `contract::tests::mcp_surface_snapshot` | Linux, Windows |

CI runs every row above as an exact filter that must execute at least one test (`Drift
contract` step in Linux `Check` and Windows `MCP transports + secure broker`). Not verified
anywhere, and not claimed: real Linux inotify exhaustion, a real Windows
`ReadDirectoryChangesW` overflow and contended `LockFileEx` — no step exhausts an OS limit;
the CI steps above run the in-process simulations of those states (watch setup refused,
blind roots, a held lease lock) on each platform, which is not the same evidence. Not run on
this host: the Postgres + pgvector serving path (pgvector is not installed here, so the
point/full-plan coexistence was proven on the local store with a mock embedder); `actionlint`
and `openspec validate --strict` (not installed — recorded as not run).

## 2026-09-12 Debt Lifecycle

The graph carries one debt state instead of seven flags: a delivered change, a forced project
reload, a failed build (a loader that could not even be spawned included), marks nobody has
consumed, a publication that could not vouch for itself, and a pending topology/roots refresh.
One pure decision reads that state, one executor acts on it, and the watcher asks it when to
wake. Recovery from an unreadable subtree has an owner — the watcher's probe, from 60 seconds
doubling to 8 minutes — and a request only wakes it, never reads disk. Declaring the same roots
again costs the hub nothing: no message, no re-arm, no reconcile, in polling and in watching
alike, and a gap in the cover is announced once and healed by the hub itself.

Background owners share one stop. `OwnerStop::stop` raises the flag and releases every wait
registered with it — the hub's `closing`, the backlog's signal, the retry driver's condvar —
and the stop is read without taking its own lock, because those predicates run under someone
else's. Leaving has an outcome of its own (`WorkspaceSearchApply::Stopping`), which the
compiler made every owner handle; the engine's uncancellable `lock()` is `#[cfg(test)]` and the
one uncancellable hold left is `take_for_shutdown`, used by the reference profile's shutdown.
A retry pause is not work: both the embed pass and the overlay driver lower their liveness
signal for it and raise it again for the attempt that follows. The shutdown order is explicit
and its outcome does not depend on the order of its three middle steps.

MCP is unchanged by this work: `contract_version` 2.3, `search_code` schema `5`, `graph` schema
`34`, no new reason code. One visible behaviour change: while the workspace overlay has never
been built, `search_code` answers `partial` with `index_building` — in a remote-overlay profile
with no embedder, for the life of the daemon.

| Scenario | Evidence | Platform |
|---|---|---|
| Re-declaring the same roots costs the fallback poll, a blind root and a second owner nothing | `change_hub::tests::a_repeated_declaration_costs_the_fallback_poll_nothing`; `change_hub::tests::a_repeated_declaration_costs_a_blind_root_nothing`; `change_hub::tests::two_owners_declaring_one_set_declare_it_once` | Linux, Windows |
| A declaration naming a missing root is reported once | `change_hub::tests::a_declaration_naming_a_missing_root_is_reported_once` | Linux, Windows |
| A blind root whose poll cannot start reads overdue rather than fresh | `change_hub::tests::a_blind_root_whose_poll_cannot_start_reads_overdue` | Linux, Windows |
| Every short order of events leaves every debt owned (11⁴ sequences, INV-OWN / INV-STALE / INV-LOOP) | `graph::debt::tests::every_short_order_of_events_keeps_every_debt_owned` | Linux, Windows |
| Two owners recording one reconcile owe one build; a quiet graph starts none of its own | `graph::debt::tests::two_owners_recording_one_reconcile_owe_one_build`; `graph::debt::tests::a_quiet_graph_starts_no_build_of_its_own` | Linux, Windows |
| An unsound publication is probed, and only a healing rebuilds | `graph::debt::tests::an_unsound_publication_is_probed_and_only_a_healing_rebuilds`; `graph::state::tests::a_healed_subtree_is_found_by_the_probe_and_rebuilt`; `graph::state::tests::a_module_that_becomes_readable_again_is_rebuilt`; `graph::state::tests::a_chronically_unreadable_subtree_is_probed_but_never_rebuilt` | Linux, Windows |
| A request spends neither the probe's budget nor a debt | `graph::state::tests::a_request_spends_neither_the_budget_nor_a_debt` | Linux, Windows |
| A loader that cannot be spawned is owed a retry | `graph::state::tests::a_loader_that_cannot_start_is_owed_a_retry` | Linux, Windows |
| Marks are not consumed against a publication being installed, and survive a lease that cannot be confirmed | `graph::state::tests::a_publication_cannot_be_installed_while_marks_are_being_consumed`; `graph::state::tests::marks_survive_a_lease_that_cannot_be_confirmed` | Linux, Windows |
| Leaving is `Stopping`, not a refusal and not a poisoned lock — with the engine held, and in the drift preparation | `state::tests::an_owner_told_to_leave_answers_stopping_even_with_the_engine_held`; `state::sync::tests::a_preparation_refused_by_a_stop_is_stopping_not_a_poisoned_lock` | Linux, Windows |
| A paused driver leaves in every order of the shutdown's three middle steps | `state::overlay_retry::tests::a_paused_driver_leaves_in_every_shutdown_order` | Linux, Windows |
| One refusal moves the publication backoff one step | `state::retry_window::tests::one_refusal_moves_the_schedule_one_step` | Linux, Windows |
| A pass in its retry pause is not live work, with the claim still held | `state::embed::tests::an_embed_pass_in_its_retry_pause_is_not_live_work`; `state::overlay_retry::tests::a_driver_in_its_retry_pause_is_not_a_running_pass` | Linux, Windows |
| The daemon's shutdown stops a polling hub, and the poll of a blind root | `state::background_lifetime_tests::shutdown_stops_a_polling_hub`; `change_hub::tests::shutdown_stops_the_fallback_poll`; `state::background_lifetime_tests::shutdown_stops_the_poll_of_a_blind_root` | Linux, Windows (blind root: Linux) |
| A stop releases an owner already asleep in a hub wait | `state::background_lifetime_tests::a_stop_releases_an_owner_asleep_in_a_hub_wait` | Linux, Windows |
| A shutdown wakes the diagnostics sweeper out of its tick | `diagnostics_state::lifecycle::tests::a_shutdown_wakes_the_idle_sweeper_out_of_its_tick` | Linux, Windows |
| Only an initialized overlay can vouch for its own debt | `tools::search::lexical::tests::only_an_initialized_overlay_can_vouch_for_its_own_debt`; `engine::tests::an_uninitialized_overlay_owes_what_its_counts_cannot_say` | Linux, Windows |
| Every production wait and engine hold is classified, and the gate fails on an injected sleep | `inventory::every_production_wait_is_classified`; `inventory::the_wait_inventory_fails_on_an_injected_sleep`; `inventory::the_test_only_modules_are_the_ones_the_parent_gates`; `inventory::no_request_path_holds_an_owner_wait` | Linux, Windows |

CI runs every row above as an exact filter that must execute at least one test, in the same
`Drift contract` step. The blind-root row is Linux only: its watch refusal is arranged through
unix permissions. Not verified anywhere, and not claimed: the exit bound inside a running
SQLite operation, which the store's busy timeout leaves at up to 30 seconds by design — the
≤ 1 second bound above is claimed only for waits the stop releases.

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

## 2026-09-12 Graph schedule v3

An open debt and a ripe one are not the same thing, and the graph's schedule had been treating
them as one. Every kind of debt now answers a single maturity question — ripe now, due at a
moment, a standing watch at a declared cap, exhausted with the external work that would revive
it, or queued behind a debt that is ITSELF owned — and the decision picks the ripest work while
the alarm is a projection of the same answer. A retry whose budget is spent holds nothing, so
the debts behind it come forward rather than starving. The stop is read inside the decision and
again at the claim, which is the one linear admission point: the claim walks the tree before it
takes the slot, and a stop landing inside that walk is answered there.

| сценарий | что доказывается | фильтр |
|---|---|---|
| SC-V1 | всякий порядок событий длины 4 держит договор, и из каждого состояния граф оседает по собственным будильникам без единой новой внешней работы | `graph::debt::tests::every_four_deep_order_keeps_the_contract`, `graph::debt::tests::every_short_order_keeps_the_contract_and_settles` |
| SC-V2 | марки внутри своей отсрочки — открытый долг, а не созревший: сборка не стартует, а будильник назван | `graph::debt::tests::marks_in_their_grace_start_no_build` |
| SC-V3 | израсходованное окно марок перестаёт запускать сборки и не оставляет будильника, которым нельзя воспользоваться | `graph::debt::tests::marks_whose_budget_is_spent_start_no_more_builds` |
| SC-V4 | остановленный повтор не держит слот: марки за ним выходят вперёд вместо голодания | `graph::debt::tests::marks_behind_a_spent_failure_are_not_starved`, `graph::debt::tests::a_spent_retry_holds_nothing_and_the_standing_says_so` |
| SC-V5 | живой повтор в паузе держит слот, и долг за ним не называет своего устаревшего момента; прошедший момент — это `Now`, а не `At` | `graph::debt::tests::marks_left_behind_a_live_retry_name_no_stale_alarm` |
| SC-V6 | публикация, не ручающаяся за себя, оседает в наблюдение с объявленным пределом темпа, а не в вечный повтор | `graph::debt::tests::an_unsound_publication_settles_into_a_watch` |
| SC-V7 | остановленный граф не начинает ничего ни одним из путей — запись, запрос, будильник — и не ставит будильника, а долги остаются | `graph::debt::tests::a_stopped_graph_decides_no_work_however_it_is_driven`, `graph::state::tests::a_stopped_graph_starts_nothing_however_it_is_driven` |
| SC-V8 | стоп, пришедший во время обхода внутри захвата, отказывает на захвате и не забирает слот по дороге | `graph::state::tests::a_stop_during_the_claims_own_walk_is_refused_at_the_claim` |
| SC-V9 | первый обход наблюдателя записывает долг, но не забирает claim у слитной загрузочной сборки (с отрицательным контролем: решающий обход его забирает) | `graph::watcher::tests::the_first_drain_records_an_idle_graphs_debt_without_taking_its_build`, `graph::watcher::the_first_observation_decides_nothing::the_first_drain_is_the_quiet_one` |
| SC-V10 | advisory, который некому вести, сам об этом говорит | `graph::watcher::tests::an_advisory_nobody_keeps_current_says_so` |
| SC-V11 | ни одна ветка цикла драйвера не засыпает, отпустив замок и не прочитав стоп | `state::overlay_retry::every_wait_reads_the_stop::no_branch_of_the_loop_sleeps_without_reading_the_stop`, `state::overlay_retry::tests::a_stop_while_the_driver_is_asking_the_engine_is_not_slept_off` |
| SC-V12 | проба восстановления лечит при исчерпанном пуле запросов, потому что открывает дескриптор сама | `graph::state::tests::a_module_that_becomes_readable_again_is_rebuilt` |
| SC-V13 | всякий выход прохода эмбеддинга говорит, чем он кончился; стоп — терминальный исход | `state::embed::embed_exit_status::every_exit_of_the_pass_says_how_it_ended` |
| SC-V14 | точечная партия, подготовленная в одном режиме baseline, не применяется в другом — в обе стороны | `point_refresh::tests::a_baseline_mode_change_discards_a_prepared_point_batch` (крейт `bsl-search`) |
| SC-V15 | кэшированный вердикт владения до первой удавшейся проверки не выдаётся за ответ | `workspace_lease::tests::a_cached_ownership_verdict_before_any_check_succeeded_is_not_an_answer` |

Платформы: все пятнадцать сценариев переносимы и стоят в обоих шагах CI «Drift contract».
SC-V12 расширяет уже существовавший `#[cfg(unix)]`-тест и остаётся в Linux-группе.

Мутационные контроли (11, каждый краснит названную заранее проверку, продакшн-текст восстановлен
побайтно): наследование forced у ветки немедленного старта → INV-QUIET; блокировка марок мёртвым
отказом → INV-OWN; прошедший момент как `At` → INV-FORWARD; решение без стопа → INV-STOP; захват
без стопа → SC-V8; ветка простоя без стопа → SC-V11; проба через пул запросов → SC-V12; решающий
первый обход и решающий `drain_quietly` → SC-V9; молчащий выход по стопу → SC-V13; публикация без
сверки режима → SC-V14.

Границы доказательства заявлены прямо: перебор исчерпывающий на префиксах длины 4 из алфавита в 14
событий плюс ограниченная доигровка (до 64 ходов) из каждого достигнутого состояния; тот же договор
пройден прототипом на глубинах 5 и 6. Это не доказательство общей живости произвольного
бесконечного расписания. `Watching` намеренно допускает вечное наблюдение: проба восстановления
при 8-минутном пределе стоит один stat-обход за интервал — объявленное ожидание внешнего события,
а не потеря прогресса.

### Дополнение после круга 1 ревью реализации (graph schedule v3)

Круг `20260912-153311-524b42`: 24 линзы, **0 упавших**, все флаги целостности false, предмет
заморожен. 20 находок (7 high), все проверены по коду: 19 подтверждено, 1 опровергнута, 18
исправлены, 1 названа ограничением. Четыре из семи блокеров — регрессии этой же эпохи.

| сценарий | что доказывается | фильтр |
|---|---|---|
| SC-V16 | слот, который это поколение захватило, никогда не остаётся `Running` без потока: отказ спавна возвращает слот, а не глушит расписание до конца поколения | `graph::state::tests::a_claimed_slot_is_never_abandoned` |
| SC-V17 | слитная холодная сборка — такая же точка допуска: после стопа она не допускается и слот не берёт | `graph::state::tests::a_stopped_graph_admits_no_fused_cold_build` |
| SC-V18 | загрузочный вызов не перезапускает упавший граф мимо расписания, а свежая работа его оживляет | `graph::state::tests::the_boots_own_call_respects_a_spent_retry_budget` |
| SC-V19 | наблюдатель ТОЛЬКО записывает: ни одной записи через форму, решающую там, где записывает | `graph::watcher::the_watcher_records_and_does_not_decide::the_watcher_never_calls_a_recording_that_decides` |
| SC-V20 | и его будильник тоже не берёт первую сборку простаивающего графа — с положительным контролем на неограниченном исполнителе | `graph::watcher::tests::the_watcher_records_an_idle_graphs_debt_without_taking_its_build` |

Исправлено без отдельного нового фильтра (покрыто существующими прогонами и структурными
гейтами): проба отличает «не смог посмотреть» от «смотрел, нечего лечить» и не тратит на первое
удвоение паузы; пауза публикации внутри прохода опускает и `IndexProgress`, а не только зеркало
flight; `hub_armed` сбрасывается на провалившейся пересборке вместе с `force_scan`; отказ ограды
по стопу пишет `Stopped`, а не «ownership superseded»; отказ ограды считается там, где случается
(иначе пауза была `retry_delay(0)` = ноль); долг хука не имеет своего момента; проба стоит за
уже открытым forced-долгом; генерация хаба читается до первого наблюдения; именованный повтор
дыры платит окном только за состоявшуюся попытку; проход эмбеддинга входит в `OwnerLive`;
инкрементальная сборка несёт тот же `TempBuildFile`, что и полная; все сканирующие исходник
гейты нормализуют переводы строк (иначе CRLF-checkout валил бы их в Windows-шаге CI).

**Оракул был независим от `decide`, но не от `stale()`.** Долг хука в `stale()` намеренно не
входит, и доигровка выходила по нему — поэтому вечно переармируемый будильник хука она не видела.
Теперь доигровка читает `open()`, и дефект краснел на ней сразу же, до правки.

Названное и НЕ исправленное: `BlindPoll::retarget` делает два полных обхода дерева под
`state`-мьютексом прямо в потоке хаба, вопреки заявлению самого модуля. Предсуществующее, вне узла
этой эпохи; вынос обходов из-под замка в потоке событий — отдельная работа со своим риском.

## 2026-09-14 Budget provenance

Предмет: происхождение бюджета допуска фоновой работы графа. Договор — B1–B12 плана
budget-provenance; 13 находок предыдущего круга разобраны все, две из них опровергнуты и их
опровержения сохранены в реализации.

Что заменено. Бюджет больше не выводится из «наблюдённого состояния». Каждое допущение имеет
именованных спонсоров и неизменный мандат (`BuildTicket`: identity, kind, mode, scan cutoff,
forced fact), выдаваемый под тем же замком, что и слот, и забираемый ровно одним строителем.
Один внешний факт покупает одну первую попытку: фронтир потраченных кредитов двигается **на
допуске**, и ни отказ, ни успех его не возвращают. Восстановление сообщает измеренный вектор
способностей, а не булев «healed», и помнит измеренные уровни через отказы и повторные
несостоятельные публикации.

| сценарий | что доказывается | фильтр | платформы |
|---|---|---|---|
| BP-1 | факт, доставленный ПОСЛЕ принятого claim, не покрыт доказательством этой сборки: барьер на живом пути claim→pre-scan→публикация, отдельно для Change и ProjectForced | `graph::state::tests::post_claim_fact_survives_publication` | Linux, Windows |
| BP-2 | кредит, погашенный доказательством публикации, не финансирует более поздний отказ | `graph::debt::tests::answered_credit_cannot_finance_a_later_failure` | Linux, Windows |
| BP-3 | сравнение гасит ровно то, что покрыл его cutoff; факт выше остаётся долгом и платит за следующий допуск | `graph::debt::tests::comparison_answer_retires_only_covered_credits` | Linux, Windows |
| BP-4 | удержанная работа платит за свой следующий claim ровно один раз и не оживляет собственный отказ | `graph::debt::tests::pending_work_pays_for_its_next_claim_only_once` | Linux, Windows |
| BP-5 | марки с истёкшим собственным бюджетом не превращают живой retry в forced-перечитывание проекта | `graph::debt::tests::expired_marks_do_not_force_a_live_retry` | Linux, Windows |
| BP-6 | новые марки оживляют исчерпанный счёт, повтор прежней расстановки — нет | `graph::debt::tests::a_new_fact_revives_spent_marks_without_reusing_old_credit` | Linux, Windows |
| BP-7 | повторное измерение того же уровня восстановления не открывает эпоху | `graph::debt::tests::repeated_recovery_level_never_reopens_an_epoch` | Linux, Windows |
| BP-8 | несостоятельная публикация не стирает измеренный witness и не возвращает backoff на пол: 120→240→480 | `graph::debt::tests::unsound_recovery_keeps_its_witness_and_backoff` | Linux, Windows |
| BP-9 | проба стоит за forced-долгом, у которого есть ЖИВОЙ владелец (в том числе транзитивно), и не стоит за исчерпанием | `graph::debt::tests::a_live_forced_owner_suppresses_redundant_probes` | Linux, Windows |
| BP-10 | один ход предлагает каждую ревизию хука один раз | `graph::state::tests::one_comparison_turn_flushes_each_hook_revision_once` | Linux, Windows |
| BP-11 | факт, пришедший во время сравнения, имеет исполнителя или защёлку продолжения, а не тридцатисекундный сон | `graph::state::tests::a_new_fact_during_comparison_runs_before_idle_wait` | Linux, Windows |
| BP-12 | первое наблюдение наблюдателя — уровень, а не доставка: бюджет не оживает; положительный контроль на настоящей доставке | `graph::watcher::tests::initial_observation_is_not_a_new_failure_credit` | Linux, Windows |
| BP-13 | закрытый scan-курсор не воскресает записью после drain и не подписывается заново | `graph::snapshot::tests::closing_a_scan_cursor_prevents_resubscription_and_stale_writeback` | Linux, Windows |
| BP-14 | терминальная аренда сохраняет своё объяснение в статусе бэклога, без новых reason codes | `state::overlay_backlog::tests::terminal_lease_exit_preserves_its_reason` | Linux, Windows |
| BP-15 | принятая владельцем однократная стоимость начального хвоста ограничена одним сообщением, и обнаружение same-stat правки этим не оплачено | `change_hub::tests::an_untouched_file_is_reported_at_most_once_and_a_same_stat_edit_is_still_found` | Linux, Windows |

Все пятнадцать добавлены точными фильтрами в **оба** шага `Drift contract`. Инвентарь после
добавления: Drift 132 Linux / 125 Windows; группа `Portable workspace lease profiles` — прежние
14 фильтров на каждой платформе, не изменена. Ни один прежний фильтр не исчез.

Ограничения, названные прямо. CI не исполнялся: ни Linux-, ни Windows-джоб; обе группы
воспроизведены локально на macOS, и это не является исполнением Windows. Платформенные
зависимости (pgvector, native Linux/Windows, inotify, `LockFileEx`) не проверены. actionlint и
openspec недоступны, файл не валидирован ими.

## 2026-09-14 Budget provenance — corrections

Независимый replay кандидата `998b7169` нашёл реальные нарушения уже принятого договора. Ниже —
сценарии, закрывающие их; исходные severity не понижены, опровержения прежних findings сохранены.

| сценарий | что доказывается | фильтр | платформы |
|---|---|---|---|
| BPC-1 | измеренное лечение — самостоятельное основание: оживляет исчерпанный бюджет без единого события в хабе | `graph::debt::tests::a_measured_healing_revives_a_spent_budget_without_a_new_fact` | Linux, Windows |
| BPC-2 | и не возвращает заработанный шаг паузы на пол | `graph::debt::tests::a_measured_healing_keeps_the_pace_the_probing_earned` | Linux, Windows |
| BPC-3 | повтор уже отвеченной доставки не покупает сборку и не оживляет исчерпанные марки | `graph::debt::tests::a_delivery_already_answered_buys_nothing_when_it_repeats` | Linux, Windows |
| BPC-4 | Operation закрывает счёт, который заплатил за этот допуск | `graph::debt::tests::marks_operation_closes_its_sponsor` | Linux, Windows |
| BPC-5 | исход сборки, оплаченной только марками, не создаёт первичный бюджет | `graph::debt::tests::expired_marks_outcome_does_not_mint_primary` | Linux, Windows |
| BPC-6 | присоединение марок платит на claim, в grace или нет | `graph::debt::tests::marks_attached_in_grace_pay_on_claim` | Linux, Windows |
| BPC-7 | допуск перепроверяет спонсора после длинного обхода, а не только в момент решения | `graph::debt::tests::a_claim_after_a_long_walk_revalidates_its_sponsor` | Linux, Windows |
| BPC-8 | отказавший hook получает конечную паузу и не раскручивает исполнителя | `graph::state::tests::a_refusing_hook_is_paced_and_does_not_spin_the_executor` | Linux, Windows |
| BPC-9 | наблюдённый уровень не финансирует последующий отказ | `graph::state::tests::an_observed_level_is_not_authority_for_a_later_failure` | Linux, Windows |
| BPC-10 | Initial-допуск сохраняет режим, зафиксированный решением по маркам | `graph::state::tests::an_initial_claim_keeps_the_marks_sponsored_mode` | Linux, Windows |
| BPC-11 | три перестановки закрытия курсора под настоящими барьерами, в трёх потоках | `graph::snapshot::tests::a_closing_cursor_races_its_own_drain_and_a_second_comparison` | Linux, Windows |

Обязательный H1 (`graph::state::tests::post_claim_fact_survives_publication`) расширен до полного
требования ADDENDUM: поздний вклад варьируется между Change, ProjectForced и марками; проверяются
identity/mode/cutoff/спонсоры ticket и то, что удержанная работа финансирует следующий допуск
ровно один раз и не финансирует его отказ.

Инвентарь после добавления: Drift **143** Linux / **136** Windows; `Portable workspace lease
profiles` — прежние 14 на каждой платформе, не изменены. Ни один прежний фильтр не исчез.

Ограничения те же и названы прямо: CI не исполнялся ни на Linux, ни на Windows; локальное
воспроизведение обеих групп на macOS исполнением Windows не является. pgvector, native
Linux/Windows, inotify, `LockFileEx` не проверены; actionlint и openspec недоступны.
