## Architecture Readiness

Architecture readiness: GO

One atomic outcome and implementation ownership are defined below. Product decision D1 was accepted by the user on 2026-09-11. Final independent re-review on 2026-09-11 returned GO with no mandatory findings; the joint completion proof is recorded in verification.md. There are zero open product decisions, implementation-time approvals or unresolved external prerequisites. Implementation progress and executed evidence are tracked in tasks.md and verification.md.

## Atomic outcome, inputs and exclusions

One result: native embedding work uses bounded HTTP requests and exposes a truthful safe failure when completion is impossible. Planner integration, preservation of committed data, failure propagation and discoverable diagnostics must ship together; none is a separate user-facing feature.

Exact read-only input: checkout `fa4693c0c0936b74ebb6f686a76f2eb583f5b899` (v0.2.79), CI run `34578618315` completed/success for that SHA, rechecked 2026-09-11. Source already contains ownership fences, bounded caller worker pools, overlay publications and the baseline publisher. Future merges of PR #142/#119 or the separate structured-indexing change are not prerequisites. Implementation owns every repair and local fixture needed below. Current code, not downstream incident logs, is the authority for behavior.

No index/schema migration, document/key/model change, new TOML section, provider probing, dependency, worker pool, public endpoint, persistent failure history, notification stream, customer service or downstream adapter change. Rollback is the preceding binary/configuration; existing data remains readable. It also restores that binary's count-only behavior, so it is not a payload-limit remedy.

## Locked decisions and configuration

Keep existing document-count, concurrency and progress settings at their existing owners. `EmbeddingExecutionPolicy` currently defaults to 32/10/20 and clamps zero counts/concurrency to one; this change does not revise those policies. MCP uses its existing CPU-derived concurrency default; CLI retains its existing default. There is no existing global HTTP request-count budget to preserve.

Add one effective request-body byte limit to `EmbedderConfig`. Production MCP and CLI resolve the same `EMBEDDING_MAX_REQUEST_BYTES`; direct library callers supply the effective value. Do not infer limits from a model/provider name or add a second count setting. Resolve embedding settings only when embeddings are enabled; an invalid byte setting must not silently turn enabled semantics into FTS-only. The limit is for serialized body bytes, excluding headers and protocol framing; response-size limits remain independent.

### Product decision D1 — ACCEPTED

Accepted by the user on 2026-09-11 (answer: «да»): default 1,048,576 bytes; a positive `EMBEDDING_MAX_REQUEST_BYTES` overrides it consistently; zero, malformed and overflowing values fail with `embedding_invalid_config`, with no raw value in the diagnostic. No unlimited sentinel or new TOML field. A direct invalid library setting must fail before network activity too.

This default changes installations that omit the variable. The downstream 1 MiB limit is a specific integration requirement, not a universal provider fact. The user accepted this behavior change, including explicit configuration failures for invalid values. D1 is locked; no further product approval is required.

## Common planner and transport boundary

Reuse the existing `EmbeddingRequest` serializer in `embedder.rs`. One `batch_ranges(texts, max_items)` planner returns contiguous input ranges; concrete owners use the current execution count cap. Extend the existing `EmbeddingGenerator` port with a count-only default for non-HTTP/test implementations and the byte-aware override for `Embedder`, so `SharedEmbeddingPublisher` uses the same planner without a provider-specific executor.

Measure the empty-input envelope and serialized JSON string sizes, including commas, with checked arithmetic. Greedily pack in order while both ceilings hold; exact equality is accepted. A constant number of serialization passes is allowed: the planner counts encoded strings and the sender serializes each selected complete request. Do not reserialize every growing prefix or materialize every complete HTTP body in the caller's work queue. Costs are linear in input bytes; existing count-bounded queues/workers remain the memory/concurrency owners.

At send time serialize the selected complete `EmbeddingRequest` once, check the actual byte length, then send those exact bytes with JSON content type. Do not check one representation and call `send_json` on another. Optional dimensions, escaped/non-ASCII model/provider values and provider `only`/`allow_fallbacks` overhead are included.

A singleton that cannot fit fails with `embedding_input_too_large`; no source truncation, content splitting, skipped-success or network retry. Empty valid work returns no vectors and sends no request. Plan before scheduling requests for that work set, so an oversize detected there does not leave partially scheduled work. Already committed work from earlier calls remains intact.

The low-level public `embed_batch` and `embed_batch_interactive` remain single-request APIs. An unplanned oversized multi-input call fails locally with `embedding_request_too_large`; the owning indexing/publishing callers must use the common planner to process a whole work set. This explicit guard also covers callers outside the repository without hiding extra network calls inside their cancellation/timeout boundary. `embed` is a singleton with the same guard.

Validate the response indices as exactly the permutation 0..request-input-count before ordering vectors; reject duplicate, missing or out-of-range indices and incompatible vector shapes with `embedding_invalid_response`. Sorting plus response count alone is insufficient. Range offsets, original chunk IDs, `missing_indices` and semantic keys remain the mapping authority; batch boundaries never change identities.

## Caller ownership, retries and publication

All caller families in verification.md must replace count-only partitioning with planner ranges. They retain their existing execution/fence mechanisms. Progress batch totals use actual packed range counts, not `div_ceil(count_cap)`; completed chunks refer to the same original inputs.

Retries apply to one prepared bounded request, not to the entire set of already successful requests. Preserve the batch path's maximum ten attempts, 120-second per-request timeout and existing backoff for other failures; the interactive path makes one attempt with its existing 12-second timeout. Local config/size failures are rejected before retry/network work. HTTP 413 is a known request-limit refusal and is not retried unchanged. Do not sleep after a final attempt. Packing can increase physical requests, elapsed build time and cost; do not claim unchanged total request count or a nonexistent global deadline. Existing outer operation deadlines, cancellation checks and publication retry budgets remain enforced between the new physical requests.

Caller-visible planning preserves per-request cancellation/fence checkpoints rather than turning one interactive call into N hidden 12-second calls. No new scheduler or cancellation policy. A request cancellation does not cancel a shared build; native Released/Superseded/TransientRefusal retains its existing precedence over an embedding failure.

For parallel pending work, retain the first typed failure while draining existing results and applying permitted successful batches through their original fences. After joins and the final ownership/cancellation check, a stopped outcome wins; otherwise return that failure before complete sidecar publication/live installation. Existing committed SQLite rows remain; missing rows remain retryable. No new partial-index result type is needed: a failed pass may keep its previous live index, and the next successful pass installs complete current data.

Preserve file/collection atomicity: do not write a partially vectorized file as fully vectorized. Preserve whole-reference replacement: `ReferenceCollectionReplaceOutcome` can carry an optional typed embedding failure alongside its existing written/fingerprint fields. A successfully committed lexical-only reference corpus keeps its existing FTS stamp and remains usable by `find_docs`; it is retried under the existing next-call/boot policy. Do not turn this into a failed lexical loader or a semantic success.

Shared baseline publication keeps existing successful commits and completion-marker authority. A failed generation must not receive a semantic-complete marker. Overlay paths retain already cached successful vectors and pending debt according to their existing publication/fence rules. No unrelated serving-store audit or automatic data repair is added.

## Typed safe failure contract

Use a closed native classification, not parsing of Display/HTTP response strings. Embedding diagnostics exposed in MCP or formatted from these typed failures contain no raw response previews, endpoint/credential/source text or arbitrary exception text. Remove the unsafe response preview slice in the embedding parser. Unknown/legacy embedding errors map to a fixed generic code. Existing non-embedding baseline/ownership error taxonomy is unchanged.

| Code | Established source |
| --- | --- |
| `embedding_invalid_config` | Rejected byte-limit setting under the selected D1 policy |
| `embedding_input_too_large` | Actual singleton request cannot fit |
| `embedding_request_too_large` | Unplanned multi-input request guard or HTTP 413 |
| `embedding_response_too_large` | Response-read size-limit error, distinguished by read stage |
| `embedding_timeout` | Typed transport timeout |
| `embedding_transport_error` | Known connection, I/O, DNS, TLS or protocol failure |
| `embedding_provider_error` | Other failing HTTP status |
| `embedding_invalid_response` | Invalid JSON, response indices/count or vector shape |
| `embedding_failed` | Other embedding failure without a safe specific classification |

`ureq` is pinned to 3.3.0 in this source. Its existing `read_to_string` response bound is 10 MiB; retain it. Request packing does not prevent response-limit errors. Classify typed `BodyExceedsLimit` at the response-read stage as response-too-large, not from its generic error name alone; do not increase the response bound.

The optional wire object is named `semantic_failure`. It has required `code` from the table, and optional unsigned `request_bytes` and `max_request_bytes` only when a local exact-size refusal establishes both values. It has no other fields, arbitrary strings or serialized error source. Omit the object when there is no known current embedding failure; absence is not a readiness proof. No overall indexing/graph target is added.

Use existing semantic runtime, overlay warmup and reference semantic runtime as owners. A build failure is retained until that owner's next attempt begins or successfully completes; an individual successful transport retry does not retain its earlier transient failure. A query failure is response-local and must not overwrite a qualified build's runtime state. Select a known overlay failure before the main workspace build failure when both owners have current failures; do not clear a different owner's state. Read-only projection introduces no provider call, Store scan, SQL probe or new observer.

| Response boundary | Behavior |
| --- | --- |
| Workspace `search.status` | Include known workspace/overlay semantic failure independently of lexical ready/busy/loading status. |
| Workspace `search_code` | Include the current semantic build/query failure on lexical fallback, including zero hits; preserve existing degraded/freshness behavior with safe text. |
| Reference `search.status` | Include reference semantic build failure; lexical-ready does not imply semantic success. |
| `find_docs` through either profile | Preserve lexical results/not-ready semantics; include known reference semantic build failure when relevant. |
| `search_docs` through either profile | Known build or query embedding failure remains an RPC error with fixed safe message and `data.semantic_failure`; retain existing non-embedding reasonCode fields for other errors. Do not return a misleading empty semantic success. |

Early terminal baseline/policy failures and transport cancellation retain precedence. Do not add failure fields to graph, unrelated metadata or `list_platform`. Existing lexical-only reference initialization is not `ReferenceSearchLifecycle::Failed` solely because embeddings failed.

## Compatibility and output budgets

Against the exact source input, publish machine contract 2.3, shared search hits/not-ready schema 5 and status schema 2; `list_platform` stays 1 and graph stays unchanged. This is an additive safe failure object with existing lexical/error semantics, not the separate structured-indexing protocol. Update declarations, tools/list output schemas, contract resource/fingerprints and examples together. Optional object schemas reject unknown codes/fields and malformed numeric fields; validate actual outputs and error data separately (RPC error data is not a successful outputSchema).

Retain the existing approximately four UTF-8 bytes/token hit-response budget and its minimum-envelope behavior. Reserve the entire safe failure object and every text/degraded/freshness copy before selecting hits; measure actual text plus compact structured JSON in tests. If the mandatory empty failure envelope fits, the response must fit `B <= 4 * max_output_tokens`. If it cannot fit, return that minimal failure envelope with zero hits and `budget_exhausted: true`; never drop the failure or force an oversized hit. This narrowly extends the existing soft minimum, not a new `budget_too_small` RPC error. `search.status` has no such token-budget input; bounded RPC errors and existing exemptions remain unchanged. Do not redesign unrelated budgets.

## Audit matrix and execution order

| Aspect | Applicable risk and owned control |
| --- | --- |
| Correctness/data | Exact serializer bytes, response permutation, range mapping, file/reference atomicity and fenced existing commits; V2–V7. |
| Reliability | Typed failed completion, deterministic oversize, unchanged retry/cancel/ownership rules and successful reset; V2–V9/V11. |
| Performance/scaling/cost | Linear packing; existing queues/concurrency; more requests acknowledged; no per-status work; V2–V7/V11. |
| Security | Closed diagnostics, safe config error and no raw provider/source data; V1/V2/V8–V11. |
| Compatibility/operations | Exact versions and current soft budget floor; env policy D1; previous-binary rollback; V1/V9/V10. |
| Infrastructure/migration | Only isolated local loopback fixtures; no customer service, new persistent table or deployment. Migration is not applicable. |

Order: effective configuration → common planner/transport → each native caller with adjacent tests → existing lifecycle failure capture → public response/schema/budget contract → docs → actual MCP fixture → repository gates/review. Every task has a bounded outcome and executable evidence; none asks the implementer to choose a product policy or await a future external publication. D1 is resolved; this order requires no further product decision.

Completion proof: bounded work can either succeed completely or fail truthfully without corrupting committed data; failing/oversize fixtures are expected test outcomes, not a required blocked final gate. All success criteria concern local code/fixtures owned by this change. No mandatory consumer/customer deployment is deferred into acceptance. Final independent re-review confirmed this completion proof. Added elements are limited to the one byte setting (R1), common planner/guard and typed failure (R2/R3/R4), existing-owner result propagation (R3/R4), and optional response/schema/budget additions (R4/R5).
