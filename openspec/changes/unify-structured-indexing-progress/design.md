## Architecture Readiness

Architecture readiness: GO

Independent read-only review of the final revision on 2026-09-11 is CLEAN, including a recheck after the source-path and exact overlay-predicate corrections. One atomic result, zero open product decisions, zero implementation-time approvals, zero unresolved external prerequisites and full R1–R5/S1–S15 → T1–T13 → V1–V13 traceability are established. This is architecture readiness, not implementation acceptance; all implementation tasks were unchecked at that architecture handoff. Current implementation evidence is in verification.md; the locked scope can be executed continuously without new product approval.

## Atomic result and ownership

One result: every covered successful MCP response carries the same truthful, bounded indexing contract, discoverable through the matching schemas. Native qualification, projection, budgets and discovery are inseparable parts of that result and ship together. Intermediate tasks are locally testable, not independent releases.

This change owns all implementation and qualification work in tasks.md, including readiness evidence plumbing, remote SQL reader fixtures and any repair needed to satisfy its contract. No external predecessor repair, merge, publication or acceptance is required. No implementation-time product or approval gates remain.

Exact read-only prerequisite: checkout `fa4693c0c0936b74ebb6f686a76f2eb583f5b899` (v0.2.79), inspected 2026-09-11; GitHub Actions run `34578618315` completed successfully for that exact SHA, including Linux Check and Windows. The skipped release job is not required. This is existing-source evidence, not proof of this unimplemented contract. Neither PR119 publication nor `bound-embedding-batches-by-payload` is a prerequisite.

## Locked wire contract

Add `indexing` beside legacy fields; never replace their meanings. Use existing serde/serde_json and one shared MCP type. The object has exactly `schema_version: "1"` and `targets`. Targets are unique, ordered graph, lexical, semantic, reference, maximum four; actual scopes below require one or two. Each target requires all six keys, including explicit nulls:

```json
{"indexing":{"schema_version":"1","targets":[{"kind":"semantic","state":"running","phase":"embedding","progress":{"completed":12,"total":48,"unit":"chunks"},"pass_id":"0123456789abcdef0123456789abcdef:4","reason_code":null}]}}
```

- `kind`: `graph|lexical|semantic|reference`.
- `state`: `waiting|running|ready|disabled|failed|cancelled|superseded|unknown`.
- `phase`: `initializing|parsing|lexical_indexing|embedding|persisting` or null. Publish only an owner-established phase; graph/reference may always use null.
- `progress`: null or `{completed: unsigned integer, total: unsigned integer|null, unit: files|chunks|batches}`. Prefer native chunks for embedding, then native batches; otherwise null. Counters concern this pass/phase, never search hits. Known completed must not exceed total; inconsistent evidence yields null, not clamping. Unknown and zero totals never imply a percentage.
- `pass_id`: null or 32 lowercase hex characters, colon, a positive decimal u64 sequence (maximum 53 ASCII bytes). Hash the existing process-instance identity with existing BLAKE3, and use checked increment; overflow yields unknown/null rather than aliasing. Retain the terminal attempt ID until a new begin. A startup-loaded ready index can have null ID.
- `reason_code`: null or one of `initializing`, `pending_work`, `semantic_disabled`, `coverage_unverified`, `identity_unverified`, `baseline_unavailable`, `overlay_pending`, `stale_generation`, `snapshot_unavailable`, `native_failure`, `cancelled`, `superseded`. No raw exception strings. Ready always has null reason; failed/cancelled/superseded use their corresponding code; disabled uses semantic_disabled. Other mappings below use the matching fixed code; native running uses null.

Progress is non-null only for active running work with a coherent counter sample. Waiting, persisting without native counters, ready and terminal/inactive states use null. Monotonicity applies only within the same pass, phase and unit. No aggregate percentage. Counter equality is never readiness proof.

## Complete response boundary

| Boundary | Required targets / attachment |
| --- | --- |
| Workspace `search.status` | lexical, semantic, every returned lifecycle state |
| Workspace `search_code` | lexical, semantic on baseline warming, normal not-ready, superseded retry, semantic-pending/unavailable lexical fallback, hits and zero hits |
| `find_docs`, `search_docs` through either workspace or reference profile | reference on all non-error not-ready, remote/local/fallback hits and empty responses |
| Reference profile `search.status` | reference only |
| `graph.status` | graph only, including idle/loading/failed/stale states |
| Graph loading response from traversal, missing pool or resolve/superseded retry | graph only, attached at graph dispatch boundary even when the legacy envelope came from metadata::loading |

Graph schema and normal graph data results, list_platform and other tools are out of scope. Do not change metadata::loading globally. Preserve graph revision/stale/reload fields and reference error semantics. Hard RPC errors and transport cancellation remain errors; request cancellation alone does not cancel a shared worker or rewrite its target lifecycle. Reuse one captured snapshot for JSON, text and legacy progress in each response. Independent targets need not represent one global transaction.

## Native progress owner

Replace independent atomic sampling with a short mutex-protected native record in existing `IndexProgress`, using generation-bound pass tokens. Cover all four entry points: index_directory, run_embedding_pass, embed_documents and sync_collection, including the empty pending-work persistence path. Helpers do not prematurely end their caller's pass. Begin/update/phase/finish/reset/drop modify only their own generation. Old callbacks and drops cannot alter a newer attempt. Preserve existing broker `is_active()` keepalive behavior and guard ownership.

Keep native chunk and batch samples together for legacy projection. Capture typed Released/Superseded before conversion to error text. A native failure, cancellation or supersession is retained until a new attempt begins; generic cleanup cannot turn it ready. Computation completion stays running/persisting until the owning commit and live-index installation succeed, including queued reruns. Failure in final persistence is failed.

Producers hold the record lock only for bounded copies/updates. Readers use try_lock; contention/poison produces unknown/snapshot_unavailable with null counters. Never acquire engine locks, do I/O, format text or call other owners while holding this lock. Existing engine correctness locks and worker cancellation/lease decisions stay unchanged.

## Readiness qualification

Maintain small in-memory qualification fields on existing owners; no second scheduler, health monitor or storage authority. Revoke qualifications on root/context/mode changes, dirty work and generation changes at existing mutation boundaries. Reuse root/overlay publication fencing; older completion cannot restore current readiness. Copy evidence from existing work, never count/validate from telemetry.

### Workspace lexical and local semantic

Lexical initializing is waiting/initializing; an owner-established active build is running; a usable current lexical engine is ready independently of semantic work; known native failure is failed. Unknown/unavailable owner evidence is unknown/snapshot_unavailable. Remote lexical readiness requires the current ready baseline and required overlay publication, using the same generation rules below.

Semantic disabled configuration is disabled/semantic_disabled. Native initialization/build/persistence failure is failed/native_failure. Active work and terminal pass outcomes take precedence over cached ready evidence. Required dirty/rerun/unembedded work is waiting/pending_work (or running when an owner is active). Provider configuration or SemanticRuntimeStatus::Ready alone is insufficient.

Local semantic ready requires successful current-generation coverage and native identity qualification plus final persistence/live installation and no pending overlay/root debt. Capture results from existing bootstrap chunk_count and embedding_count_by_collection calls; preserve errors as unknown/coverage_unverified, never unwrap failure to zero. Capture normal pending-document selection and completion outcomes; Applied(Incomplete) or Applied(Superseded), failed/malformed batches and missing coverage cannot qualify ready.

Trust an accepted existing vector sidecar only to its native model/dimension/text-version/generation/checksum/count contract. An unqualified SQLite-BLOB fallback has no proven model provenance: report unknown/identity_unverified until normal existing work qualifies it. Do not force repairs, reembedding or add storage metadata. A genuinely empty scope is ready only after successful completion/publication, not from failed reads or absence of vectors.

Ready describes index readiness, not current query-provider health. A transient query embedding timeout preserves ready index evidence and existing degraded/freshness lexical fallback; it does not globally mark the index failed or add a health probe.

### Remote baseline plus overlay

Use existing PostgreSQL `schema_metadata` publication key `semantic_publication_complete:<snapshot>:<model>:<dimension>` with value `complete`, written/cleared by the existing publisher. Baseline ready or embedding identity alone is insufficient. Extend the existing snapshot_details SELECT with three indexed metadata lookups for model, dimension and this completion marker in the same SQL statement/MVCC snapshot; no added round trip, scan, count or provider call. Parse malformed dimension safely in Rust, not a failing SQL cast.

Carry optional internal evidence in BaselineSnapshotDetails, including snapshot identity/fingerprint from that same details row, through existing probe_status_result into the existing cache. Do not combine it with a separately resolved older snapshot. Require current generation, unexpired existing success TTL (60 seconds), matching reader model/dimension and matching snapshot/fingerprint. Missing marker/details or malformed identity is unknown (coverage_unverified or identity_unverified); absent baseline is waiting/baseline_unavailable. Stale/expired/in-flight cache evidence is unknown/stale_generation even when legacy text retains cached status. Queries only peek at evidence; status may preserve its existing lazy probe behavior, with no additional probe introduced by indexing.

Remote semantic ready additionally requires the current overlay publication (Synced or qualified NoLocalDiffs) and `initialized == true`, `needs_full_rescan == false`, and zero pending_dirty_paths, unembedded_entries and unread_keys according to existing overlay signals. Active overlay is running, pending overlay is waiting/overlay_pending. Known failure/terminal state wins. A publisher-qualified empty baseline can be ready; missing counts cannot establish emptiness. Trust the existing publication marker as its existing authority; do not add serving-table audits or change publication semantics.

### Graph and reference

Capture graph state/revision/stale/reload information coherently under its existing lifecycle owner; cache a bounded record if status currently reads those fields separately. Loading or active reload is running; fresh current published graph with no unread/stale debt is ready; stale graph is waiting/stale_generation; failure is failed; explicit cancellation/supersession retains that state. Never use graph readiness as vector evidence. Legacy loading retry can coexist with a currently ready graph target when the retry concerns another root owner; it must expose the actual graph state rather than derive it from envelope text.

Reference Uninitialized is waiting/initializing, Loading running, Ready ready, Failed failed, explicit lifecycle shutdown cancelled. Both profiles' docs operations use this reference owner. Graph/reference phase, counters and pass ID remain null where the native owner lacks them; do not invent progress instrumentation.

## Compatibility, discovery and budgets

Locked versions at the prerequisite SHA: machine contract `2.2` → `3.0`; shared search hits/not-ready `4` → `5`; search status `1` → `2`; graph schema descriptor `33` → `34`; indexing `1`; list_platform remains `1`. Update constants/enums, ToolDecl metadata, tools/list outputSchema, contract resource and generated BLAKE3 fingerprints/goldens together. No additional root schema_version is added to legacy graph envelopes. Graph outputSchema must require indexing for root `state` or `status == loading`; its alternative data/schema branch must explicitly exclude those conditions so missing indexing cannot pass another branch. Validate actual outputs with already-installed jsonschema, including negative missing-field fixtures.

Legacy not-ready progress remains a projection of the same snapshot: active follows native guard activity; counters and percentage are available only from coherent relevant samples, and percentage only for total > 0. Do not expose stale counters during persistence/wait/terminal states. Existing search result/fallback and legacy lifecycle field meanings remain intact.

For operations already honoring max_output_tokens, reserve the entire mandatory empty/not-ready envelope before selecting hits. Define measured bytes B as UTF-8 bytes of all returned text content plus compact structured JSON bytes (protocol framing excluded), retaining the existing estimate of four bytes/token. Budget compliance is B <= 4 * max_output_tokens. Include indexing and all mandatory legacy/detail/degraded/freshness fields in the measured envelope, using the same captured snapshot. After rendering, remove whole trailing hits and update related counts/notes until the response fits; never truncate JSON or remove indexing. Optional diagnostic notes may be omitted; required fields may not.

If the mandatory no-hit envelope cannot fit, return MCP invalid_params (-32602), fixed message/reason `budget_too_small`, with error data `minimum_output_tokens = ceil(B/4)`. Default 6000 remains unchanged. Existing hard error/cancellation precedence remains. Search status and graph status/schema/resolve retain their existing budget exemptions; graph loading uses a budget only for the existing source-bearing budgeted actions. Document these boundaries. The rejection of previously accepted tiny budgets requires major 3.0 under contract.rs; this is the compatibility consequence of the already-required no-oversize contract, not a new product decision.

Strict consumers qualify the new version before their own deployment; their release is not a prerequisite here. Rollback is the previous qualified binary and matching contract, with no index conversion. Missing indexing in the old contract means unavailable telemetry, never complete.

## Execution, risks and completion proof

Tasks are ordered native owner → qualification → response integration → budget/discovery → end-to-end checks/docs → final checks. Each adjacent fixture is executable before the next slice; the public contract is published only after all integrations are complete. No task asks implementation to choose a policy or seek approval.

| Aspect | Current risk and owned mitigation | Proof |
| --- | --- | --- |
| Correctness/reliability | False ready, late callbacks, partial persistence | T1–T5 and V1–V5 |
| Compatibility | Missing response branches or misleading schema | T6–T10 and V6–V10 |
| Performance/scaling/cost | Polling behind engine lock or adding remote work | try_lock + existing cached evidence; V4/V11 |
| Security | Arbitrary native errors/paths in telemetry | closed enums and bounded IDs; V2/V11 |
| Data/migration | Accidental new storage authority | read existing markers only; V4; no migration |
| Operations/rollback | Unknown legacy coverage mistaken for completion | explicit unknown and documented previous-binary rollback; V12 |
| Infrastructure | Local SQL/runtime checks need controlled services | T4/T11 own ephemeral loopback fixtures; no customer services |

All prerequisites are already positive read-only inputs. This change owns every remaining blocker: coherent samples (T1), precise qualification (T3–T5), branch coverage (T6–T8), bounded envelope (T9), discovery (T10), executable acceptance and docs (T11–T13). Unknown is a truthful outcome for unqualified/contended/stale evidence, but positive ready fixtures are mandatory; returning unknown everywhere cannot pass. The SQL reader fixture proves marker consumption, not full semantic publisher correctness. There is no requirement to repair arbitrary legacy/corrupt stores or deploy a consumer. Consequently all success criteria can hold simultaneously without external waits.

Complexity gate: one native coherent record serves counter correctness; bounded qualification on existing owners serves readiness; one shared wire type serves serialization; three indexed scalar lookups serve existing remote publication proof; final envelope measurement serves the explicit budget requirement. Reuse existing process identity, BLAKE3, serde, jsonschema, overlay fencing, probe cache and tests. No new component/dependency/service is justified or added. Open product decisions: zero.
