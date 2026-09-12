## 1. Effective limits and transport

- [x] T1 Implement the effective byte-limit configuration from design.md in EmbedderConfig and existing MCP/CLI constructors, preserving current count/concurrency settings. Check V1.
- [x] T2 Implement the common serializer-backed planner and single-request transport guard, typed safe failures and response alignment validation in embedder/error/ports; keep existing per-request retry/timeout behavior. Check V2.

## 2. Native caller integration

- [x] T3 Integrate planner ranges into fenced and parallel pending-chunk paths; retain valid commits, typed failure and cancellation/fence precedence, without complete publication after failure. Check V3.
- [x] T4 Integrate file indexing and collection sync with range offsets and cached/missing input alignment while preserving file atomicity. Check V4.
- [x] T5 Integrate reference document batches and carry semantic failure in the existing reference replacement outcome without failing successful lexical publication. Check V5.
- [x] T6 Integrate both overlay embedding paths with planner ranges, existing cache/publication boundaries and cancellation checks. Check V6.
- [x] T7 Integrate the common planner through EmbeddingGenerator into SharedEmbeddingPublisher; preserve bounded workers, semantic keys and completion-marker behavior. Check V7.

## 3. Lifecycle, MCP contract and qualification

- [x] T8 Carry typed failures through existing workspace/overlay/reference semantic lifecycle owners and clear them at the correct attempt boundaries; keep query errors request-local. Check V8.
- [x] T9 Expose safe semantic_failure across the covered response/error boundaries, account for the complete envelope and update versions, output schemas and fingerprints together. Check V9.
- [x] T10 Document effective limits, direct request/planner semantics, failure codes, response scope, budget minimum, cost and rollback with schema-validated examples. Check V10.
- [x] T11 Run the bounded actual MCP/loopback embedding acceptance fixture, including successful packing, partial failure and later recovery, using only owned local fixture state. Check V11.
- [x] T12 Run repository gates and strict OpenSpec validation; record requirement-level evidence and limitations for the mandatory independent implementation/complexity review and remediation cycle. Check V12. No archive, release, installation, commit or push.


## Review cycle

After T12's repository gates pass, keep the cursor at T12 with stage `ревью` and perform the mandatory independent ponytail-review then implementation-vs-plan review. Materialize every mandatory finding below, repair and repeat both reviews; finish with the full final verification stage. Checked implementation tasks do not replace this completion gate.


## Разрывы ревью

- [x] G1 Restore the existing unconfigured-semantic refusal order in search_docs while preserving terminal baseline precedence for known embedding failures. Evidence: existing external-reference semantic-validation regressions plus the full search/MCP payload and workspace suites.
- [x] G2 Reject invalid direct-library byte configuration before empty/cached publisher and standalone overlay fast paths. R1 S2; verify direct-library empty/cached regressions return embedding_invalid_config with zero HTTP.
- [x] G3 Align overlay embedding-failure status prose with the existing search_code lexical fallback. Optional P3 from MCP review; verify status tests preserve the structured failure.
- [x] G4 Make the new standalone invalid-config regression satisfy the mandatory clippy err_expect lint without adding Debug requirements to production types. Verify the regression and workspace clippy.
- [x] G5 Correct verification totals to count only each Cargo target's final summary, excluding child-test summaries, and finish the bounded MCP evidence collection. Verify the collector against nested-summary and incomplete-target cases; production source remains unchanged.
<!-- GOAL_CURSOR -->
- [ ] G6 Restore blocking, timeout-bounded reads in the shared native payload HTTP fixture after nonblocking accept. Verify the failed reference recovery test, payload/lifecycle suites and Linux/Windows CI on the pushed PR head.
  - Этап цикла: финальная проверка
  - Состояние шага: socket fix прошёл exact reference recovery, payload 19/19, lifecycle 44/44, fmt и strict bsl-search clippy; оба независимых ревью GO.
  - Следующее действие: опубликовать исправление и дождаться Linux/Windows CI на новом SHA; G6 остаётся открытым до remote success.
  - Файлы шага: crates/bsl-search/src/embedder/payload_tests.rs, openspec/changes/bound-embedding-batches-by-payload/tasks.md, openspec/changes/bound-embedding-batches-by-payload/verification.md.
