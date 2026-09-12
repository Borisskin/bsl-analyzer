## ADDED Requirements

### Requirement: Explicit effective request-byte configuration

R1. The native embedder SHALL receive the effective finite positive byte ceiling under the accepted configuration policy in design.md. Existing count/concurrency policies SHALL remain unchanged. MCP and CLI SHALL use the same new environment-setting precedence; library callers SHALL be able to supply the effective value without another configuration layer.

#### Scenario: Configuration precedence
- **WHEN** embeddings are enabled and the request limit is resolved
- **THEN** the default is 1,048,576 bytes and a positive EMBEDDING_MAX_REQUEST_BYTES override applies consistently in MCP and CLI
- **AND** verification uses S1/V1 in verification.md.

#### Scenario: Invalid configuration
- **WHEN** a supplied limit is zero, malformed or overflowing
- **THEN** it cannot silently turn enabled semantics off or produce an unbounded request
- **AND** it returns embedding_invalid_config before network activity without exposing the raw value; verification uses S2/V1.

### Requirement: Exact common request boundaries and mapping

R2. Every repository embedding work-set owner SHALL use the common serializer-backed planner with its existing count cap and the effective byte ceiling. The common transport SHALL check the exact complete body bytes it sends, including optional envelope fields. Packing SHALL be linear, preserve input identity/order and use existing execution owners.

#### Scenario: Multi-document work exceeds byte ceiling
- **WHEN** a count-valid work set needs several bounded HTTP requests
- **THEN** the caller executes contiguous planner ranges with unchanged count/concurrency controls and per-request checkpoints
- **AND** no successful request is resent because a later range failed; verification uses S3/V2–V7.

#### Scenario: Serialization and exact boundary
- **WHEN** input or envelope fields contain UTF-8, JSON escapes or optional routing/dimensions
- **THEN** accounting includes their actual serialized bytes and accepts equality with the ceiling
- **AND** the sender does not send a larger body; verification uses S4/V2.

#### Scenario: Oversized singleton and direct request guard
- **WHEN** a singleton cannot fit or a low-level single-request API receives an unplanned oversized multi-input request
- **THEN** it fails with the corresponding safe input/request-limit code before network retries, without truncation or skipped-success
- **AND** whole work sets are split by their callers through the common planner, not hidden inside a single interactive transport call; verification uses S5/V2.

#### Scenario: Returned vector alignment
- **WHEN** responses arrive across splits, remainders, parallel workers or shuffled response indices
- **THEN** vectors map to the original chunk IDs/semantic keys and only an exact response-index permutation with valid shapes is accepted
- **AND** duplicate, missing or out-of-range indices fail rather than silently misassign vectors; verification uses S6/V2–V7.

### Requirement: Partial failure cannot publish semantic completion

R3. Embedding failure SHALL remain observable through the owning result/lifecycle. Existing committed vectors SHALL remain valid under their original fences; missing work SHALL stay retryable. No incomplete file/reference corpus or failed pass SHALL be advertised as semantically complete. Cancellation, ownership outcomes and existing publication deadlines SHALL retain precedence.

#### Scenario: A later request fails
- **WHEN** some bounded requests have committed vectors and a later request fails
- **THEN** earlier committed rows remain, pending work is not marked complete, and the pass returns its typed failure before complete publication
- **AND** keeping the prior live index on failure is permitted; verification uses S7/V3/V4/V6/V7/V8.

#### Scenario: Cancellation and publication fencing
- **WHEN** cancellation, Released, Superseded or TransientRefusal occurs between packed requests or before publication
- **THEN** the existing owner outcome wins and an old worker cannot publish completion or overwrite the new owner's failure state
- **AND** request cancellation alone does not cancel shared indexing; verification uses S8/V3–V8/V11.

#### Scenario: Reference lexical fallback
- **WHEN** reference corpus embedding fails but its lexical publication succeeds
- **THEN** find_docs remains available with the existing FTS stamp and retry policy while semantic failure is retained separately
- **AND** search_docs does not report an empty semantic success for that known failed build; verification uses S9/V5/V8/V9.

### Requirement: Safe current embedding failure diagnostics

R4. Embedding failures SHALL use the closed code/object contract in design.md. Structured and textual diagnostics SHALL exclude source, endpoints, credentials, raw provider bodies and arbitrary error text. Existing runtime owners SHALL retain build failures; query failures SHALL remain request-local. Projection SHALL introduce no additional network/probe/scan work.

#### Scenario: Classified request response and provider failures
- **WHEN** request size, response size, timeout, transport, provider or response-validation failure is established
- **THEN** MCP status/fallback/error boundaries expose the corresponding safe code, with only allowed numeric evidence
- **AND** unknown embedding errors use the generic safe code, never a raw preview; verification uses S10/V2/V8/V9/V11.

#### Scenario: Successful retry or new attempt
- **WHEN** a retry or later build succeeds or a new owned attempt begins
- **THEN** its diagnostics do not retain a stale failure from that attempt
- **AND** a transient query failure does not mark the built index failed or clear another build owner's failure; verification uses S11/V8/V9/V11.

### Requirement: Discoverable compatible failure responses

R5. The affected existing MCP responses SHALL follow the scope, exact schema versions and budget rules in design.md. The optional failure object SHALL be closed and discoverable; existing lexical fallback and non-embedding RPC/cancellation semantics SHALL remain intact.

#### Scenario: Contract discovery and profile coverage
- **WHEN** clients discover tools/list and the contract resource and use either workspace or reference docs routes
- **THEN** machine 2.3, search 5/status 2 and fingerprints describe the actual safe failure fields while graph and list_platform remain unchanged
- **AND** invalid codes/fields fail schema validation; verification uses S12/V9/V10.

#### Scenario: Failure envelope budget
- **WHEN** a hit response carries semantic_failure under max_output_tokens
- **THEN** the entire failure envelope is charged before hits and the text plus compact JSON fits the budget whenever the mandatory minimum can fit
- **AND** a smaller budget keeps the minimal safe failure envelope with zero hits and budget_exhausted true, preserving the existing soft minimum rather than adding a new RPC error; verification uses S13/V9/V10.
