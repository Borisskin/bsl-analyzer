## 0. Baseline and traceability

- [x] 0.1 Fetch `upstream/develop`; require exact v0.2.77 integration base `edc78e22f3efbfe51ffd8e6dfd05b457976195ca`, confirm PR #54 merge `75b8a978` is an ancestor, record dirty state, and capture the production caller inventory before editing.
- [x] 0.2 Map every requirement scenario to one exact task and planned test/inventory identifier in `verification.md`; reject duplicate or unmapped rows.
- [x] 0.3 Run the focused lease, graph, bootstrap, sync, embed, and overlay baseline plus formatting and strict Clippy; record pre-existing failures separately.

## 1. Outcome and profile contract

- [x] 1.1 Classify platform lock contention, `UNCLAIMED`, temporary missing record, malformed record, and managed open/read/lock errors at the lease boundary with deterministic seams.
- [x] 1.2 Introduce exhaustive `LeaseOperationOutcome<T, E>` and `LeaseOperationError<E>`; preserve callback versus lease I/O cause without later ownership probes.
- [x] 1.3 Add `publish_short`; perform fallible restamp before one atomic visibility commit, prove restamp failure executes the commit zero times, permit commit error only before visibility, perform no later lease I/O, and keep raw acquisition private.
- [x] 1.4 Add `publish_checkpointed`; latch the first confirmed terminal cause, roll back on terminal/restamp error, and test same-token liveness refresh plus foreign-token control.
- [x] 1.5 Split `bsl_search::FenceOutcome` and every host adapter into distinct `Superseded` and `Released` variants with exhaustive matches.
- [x] 1.6 Remove production use of `with_ownership_outcome` and `with_ownership_checkpointed`; add the exact source-inventory gate.

## 2. Caller migration and bounded work

- [x] 2.1 Migrate heartbeat restamp to one short publication attempt per normal tick, without an inner retry loop.
- [x] 2.2 Keep temporary graph construction in off-fence `GRAPH_BUILD_BATCH = 500` batches; migrate prepared rename, snapshot identity install, and descriptor-pool install to short publication with all open/build work outside.
- [x] 2.3 Migrate fused per-file graph ingest to checkpointed publication at `WORKSPACE_APPLY_BATCH_ROWS = 64` chunks and prove complete rollback when terminal control arrives at chunk 65.
- [x] 2.4 Migrate drift, full-rescan, and directory apply to prepared 64-row publications while preserving cursor and remaining debt.
- [x] 2.5 Add checkpointed 64-row helpers for baseline manifest save/clear and external-baseline fingerprint clear; preserve all-or-nothing visibility.
- [x] 2.6 Migrate roots/context and external-baseline state transitions to checkpointed publication and prove atomic rollback.
- [x] 2.7 Migrate overlay Phase C publication to `publish_checkpointed` at the existing 64-row boundary while preserving its prepared bundle and retry state.
- [x] 2.8 Migrate structural schema/open and FTS rebuild transactions to checkpointed publication at the existing 64-row boundary with rollback tests.
- [x] 2.9 Migrate single-file ingest transactions to checkpointed publication at the existing 64-row boundary with owner-specific rollback tests.
- [x] 2.10 Migrate embedding preflight, prepared vector publication, and sidecar swap without holding a fence across network work.
- [x] 2.11 Re-audit every production callback for file walks, network calls, descriptor/database open, workspace-sized loops, and process-local mutex nesting.

## 3. Workflow-owned retry

- [x] 3.1 Reuse or minimally relocate one data-only retry-window contract for startup, change-hub/drift, overlay/embedding, and graph owners; lock a 600-second default, coalescing/re-arm/error semantics, checked monotonic arithmetic, deterministic clock seam, and an exhaustive owner-table test without adding a scheduler or knob.
- [x] 3.2 Bound startup publication to 600 seconds with 2-second cadence; return initialization error on exhaustion and stop immediately on operation/terminal outcomes.
- [x] 3.3 Bound change-hub enable and prepared drift admission to 600 seconds; on enable exhaustion keep the existing sink dormant with degraded/failed state so the next genuinely new hub batch can start exactly one fresh budget, and preserve the specified cursor/rescan-debt exits.
- [x] 3.4 Include overlay and embedding preflight/publication in their existing single retry obligation; retain the compatibility environment override and prepared paid batch.
- [x] 3.5 Replace unbounded graph `withheld_build` with a trigger-driven 600-second obligation sourced only from the original transient result.
- [x] 3.6 Keep admitted-operation `RescanDebt` separate from transient deadlines as one indefinitely coalesced current-disk recovery slot with existing capped backoff.

## 4. Request and recovery invariants

- [x] 4.1 Keep `resolve_names`, `graph`, and `symbol_info` descriptor-pool-only; add independent held-lock/empty-pool prompt-return tests.
- [x] 4.2 Remove fenced prefetch from lexical, semantic, and hybrid requests; read resident state, coalesce into the existing search sink, and add independent held-lock tests.
- [x] 4.3 Make metadata, diagnostics, and graph status render cached process-local state without `owns_caches()`/`may_build()` refresh; add held-lock tests.
- [x] 4.4 Preserve typed background-snapshot outcomes: transient retains batch/deadline; superseded/released exit; changed/missing/open error advances cursor and creates one debt.
- [x] 4.5 Preserve drift cursor advancement, topology nudges, one rescan debt, remove-then-recreate convergence, and existing capped backoff after injected Store failure.
- [x] 4.6 Preserve zero network calls after failed embedding preflight, one call per prepared paid batch across transient publication, and `Failed` on deadline.
- [x] 4.7 Preserve graph failure provenance and reject later-probe reclassification for transient, operation, superseded, and released outcomes.

## 5. Verification and delivery gates

- [x] 5.1 Run every exact scenario filter listed in `verification.md`; each must report at least one executed test.
- [x] 5.2 Run `cargo fmt --all -- --check`, strict Clippy for both crates, both crate suites with `--no-fail-fast`, and `git diff --check`.
- [x] 5.3 Wire the exact portable lock/release/path-identity/request filters into Linux `Check` and Windows `MCP transports + secure broker`, and validate the workflow with `actionlint`.
- [x] 5.4 Audit this change against the recorded base for new dependencies, runtime threads/schedulers, persistent formats, lease records, configuration knobs, and MCP wire changes. This is a one-time PR diff review, not a permanent restriction on later changes.
- [x] 5.5 Fill actual evidence in `verification.md` and run `openspec validate enforce-workspace-lease-operation-profiles --strict --no-interactive`.

## 6. Maintainer review closure

- [x] 6.1 Close the five blocking review findings: keep broker accept cached-only, require non-zero CI filters, preserve retryable startup transactions, re-arm graph recovery only on fresh work, and remove alias tests.
- [x] 6.2 Strengthen the held-lock and caller-inventory gates without restoring request-time lease work.

## 7. Inventory portability and prepared-build retention

- [x] 7.1 Remove the historical Git-baseline audit from permanent unit tests; keep source inventories working without Git history and with native Windows paths and CRLF checkouts.
- [x] 7.2 Add all remaining inventory tests to both Linux and Windows CI with non-zero exact filters.
- [x] 7.3 Retry only the prepared index swap on transient refusal, preserving the existing deadline and pending rerun; prove no rebuild or repeated paid call and no lost fresh-work signal.
- [x] 7.4 Run focused regressions, crate suites, strict workspace Clippy, formatting, workflow lint, and OpenSpec validation; record evidence.
- [x] 7.5 Isolate temporary graph databases for overlapping backend generations in one process; prove refused/failed builds preserve another build's file and rerun the superseded-daemon lifecycle integration.
- [x] 7.6 Make cold-graph name-dictionary fixtures hold a real lease lock before startup, release it for ready-graph controls, and cover the transport suite on Windows too.

## 8. Debt lifecycle: one graph debt state and one background stop

- [x] 8.1 Make declaring the roots idempotent in the hub: compare against the declaration the hub thread accepted, send no re-arm and ask for no reconcile when it is unchanged, announce a gap in the cover once, and heal it in the hub itself; read "never polled" as overdue rather than fresh; make `shutdown` a production method that stops the hub thread and the blind poll alike.
- [x] 8.2 Replace the graph's seven flags with one debt state, one pure decision and one executor: change, forced reload, failed build (a loader that could not be spawned included), owed marks, an unsound publication with a probe of its own, and a pending topology/roots refresh; gate publication against mark consumption; hold marks on a lease that cannot be confirmed.
- [x] 8.3 Give every background owner one stop: registered wakers released by `OwnerStop::stop` itself, a lock-free stop read for predicates that run under another owner's lock, stop-aware sleeps and acquires, `WorkspaceSearchApply::Stopping` as its own outcome, `AdmittedEngine::lock` test-only, and an explicit shutdown order whose outcome does not depend on the order of its middle steps.
- [x] 8.4 Report the search overlay's debt honestly: expose `initialized` as its own fact beside the counts, and answer `partial` with `index_building` while the overlay has never been built.
- [x] 8.5 Key the wait/acquire inventory by `(file, enclosing fn, token)`, classify every key, and prove the gate fails on a sleep injected into the retry driver; record the scenarios in `verification.md` and their exact filters in both CI `Drift contract` steps.
- [x] 8.6 Run the full gates — formatting, strict Clippy, `git diff --check`, both crate suites, the contract snapshot, every exact filter and the non-existent-filter probe — and record the real counts and logs.

## 9. Graph schedule v3: maturity, not openness — and one admission point for the stop

- [x] 9.1 Give every kind of graph debt one schedule and one maturity answer (`Ripeness`: ripe now, due at a moment, a standing watch at a declared cap, exhausted with the external work that revives it, or queued behind a debt that is itself owned); build the decision and the alarm on that one account rather than on separate rules about what to mute.
- [x] 9.2 Make the decision pick the ripest work instead of the merely open debt: a forced build is demanded by the work chosen, inherited only by a retry that is going to read disk anyway, and marks start through their own grace and budget.
- [x] 9.3 Let an exhausted retry hold nothing, so the debts queued behind it come forward instead of starving; keep a delivered change ahead of a marks grace, because that grace exists to leave room for exactly such a change.
- [x] 9.4 Put the stop inside the decision and make the claim the one linear admission point: `Facts.stopping`, `ReloadClaim::Stopping` read under the lock that grants the slot, and a first-build claim that refuses the same way.
- [x] 9.5 Replace the tautological ownership check with an independent oracle: the model keeps its own ledger, reads the standing rather than re-running the decision, proves each named revival by applying it, and adds a bounded settling walk that catches both a hot loop and a debt nothing will ever run.
- [x] 9.6 Fix the verified findings of the same round: a watcher that never started abandons its advisory; the retry driver reads the stop before sleeping on every branch that let the lock go; the first drain records without deciding; the recovery probe opens a descriptor of its own; a stop exit of the embed pass says how it ended; a point batch may not settle in the other baseline mode; a status answer does not publish an ownership verdict nothing has checked.
- [x] 9.7 Run the gates and the mutation controls: eleven controls, each reddening a check named in advance, with the production text restored byte-for-byte afterwards.

<!-- GOAL_CURSOR -->
Tasks 0-7 are complete with final local verification recorded on 2026-09-09.
The user authorized publication of the follow-up and live CI verification. Maintainer acceptance
and issue closure remain separate delivery actions.
