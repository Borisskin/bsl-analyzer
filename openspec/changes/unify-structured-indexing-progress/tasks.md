## 1. Native ownership

- [x] T1 Implement generation-fenced IndexProgress and all four producer/empty-persistence exits in bsl-search engine.rs; update existing direct counter consumers and preserve is_active guard/keepalive behavior. Check V1.
- [x] T2 Add the shared closed indexing wire type and legacy/text projection in MCP search types/status rendering helpers, including bounded identity/reason fields and null rules. Check V2.

## 2. Qualified lifecycle evidence

- [x] T3 Capture local lexical/semantic qualification from existing bootstrap/embed/sync and overlay outcomes, fence invalidation and final publication, and preserve ready on query-only provider failure. Check V3.
- [x] T4 Extend existing external_baseline PostgreSQL snapshot_details and MCP baseline cache evidence; aggregate current baseline identity/publication with required overlay without extra probes. Own an ephemeral PostgreSQL reader fixture and run V4.
- [x] T5 Expose coherent graph and reference owner snapshots using existing graph lifecycle and reference loader state, including stale/reload/terminal handling and null native counters. Check V5.

## 3. Response integration

- [x] T6 Attach lexical/semantic indexing to workspace status and every non-error search_code path in lib.rs and search status/hybrid/render handlers; reuse one snapshot for text/legacy projection. Check V6.
- [x] T7 Attach reference indexing to reference status and every find_docs/search_docs outcome in both profiles, including local/remote/fallback/empty and not-ready paths. Check V7.
- [x] T8 Attach graph indexing to graph status/loading at graph dispatch, including resolve superseded retries from metadata::loading, without changing unrelated metadata or normal graph data envelopes. Check V8.
- [x] T9 Enforce final mandatory-envelope byte accounting on existing budgeted search/graph-loading operations and implement budget_too_small error/minimum data; preserve budget exemptions and cancellation/error precedence. Check V9.
- [x] T10 Publish machine 3.0, search 5/status 2, graph descriptor 34 and indexing 1 with affected outputSchemas, fingerprints/goldens and schema validation; keep list_platform 1. Check V10.

## 4. Qualification and handoff

- [x] T11 Add and run bounded synthetic MCP polling plus held-lock/call-count and redaction checks using existing local test facilities and a controlled loopback embedding stub; own all fixture setup. Check V11.
- [x] T12 Update docs/mcp/README.md and TOOLS_AND_EXTENSION.md with validated JSON examples, coverage/null/terminal rules, budget/error/exemption semantics, version qualification and rollback. Check V12.
- [x] T13 Run repository checks and strict OpenSpec validation; record actual results and limitations in verification.md. This closes the original implementation tasks before the independent review cycle below. Check V13; no release, archive, commit or push.


## Post-implementation review and final verification

After all original tasks are evidenced and checked, keep the cursor at T13 for independent Ponytail and implementation-vs-plan reviews. Materialize mandatory findings as unchecked tasks under `Разрывы ревью`, repair and re-review until clean, then run final OpenSpec verify and V13 gates. T13 being checked does not by itself claim the change complete.

## Разрывы ревью

- [x] G1 Bind remote lexical/semantic readiness to the manifest identity used by the qualified overlay (R2/S7). Check A→B and same-ID/new-fingerprint transitions remain non-ready until the matching overlay is qualified.
