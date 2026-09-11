## ADDED Requirements

### Requirement: Uniform indexing snapshot across response surfaces

R1. The server SHALL attach the design.md indexing schema to every covered non-error response. Targets SHALL be unique, bounded, ordered and scoped, with all six fields and explicit nulls. Existing RPC errors and cancellation semantics SHALL remain errors.

#### Scenario: Workspace responses
- **WHEN** workspace status or search_code returns initializing, baseline warming, superseded retry, semantic-pending/unavailable fallback, hits or zero hits
- **THEN** the response includes lexical and semantic targets, with coherent available embedding counters even when lexical results are already available or empty.
- **AND** verification uses S1 in verification.md.

#### Scenario: Reference responses in both profiles
- **WHEN** find_docs or search_docs returns local/remote/fallback hits, zero hits or not-ready in either profile, or reference status returns
- **THEN** the response includes reference only and never implies workspace vector readiness.
- **AND** verification uses S2 in verification.md.

#### Scenario: Graph response boundaries
- **WHEN** graph status or a graph loading/retry envelope returns, including resolve supersession through metadata loading
- **THEN** the response includes graph only from its real owner and preserves revision/stale/reload fields; normal graph data/schema and unrelated metadata responses remain outside this contract.
- **AND** verification uses S3 in verification.md.

### Requirement: Truthful per-target readiness

R2. Readiness SHALL require current native scope, identity and successful publication, independently per target. A configured provider, graph readiness, inactive progress, completed batches or successful fallback SHALL NOT alone establish semantic readiness. Unknown evidence SHALL remain unknown under the exact qualification rules in design.md.

#### Scenario: Local coverage and identity
- **WHEN** local coverage is partial, a count read fails, a fallback lacks native identity proof, or an overlay generation becomes dirty
- **THEN** semantic is waiting/running for known pending work or unknown for unverified evidence, never ready; a qualified current sidecar or successful complete publication with no debt can establish ready.
- **AND** verification uses S4 in verification.md.

#### Scenario: Disabled and failed native build
- **WHEN** semantics are disabled or native initialization/build/persistence fails
- **THEN** semantic is respectively disabled or failed while lexical readiness remains independent.
- **AND** verification uses S5 in verification.md.

#### Scenario: Persistence and genuinely empty scope
- **WHEN** embedding computation finishes before persistence/live installation or a queued rerun completes
- **THEN** semantic remains running/waiting; successful final publication including a genuinely empty scope can establish ready without invented vectors or percentages.
- **AND** verification uses S6 in verification.md.

#### Scenario: Remote publication qualification
- **WHEN** a remote baseline and required overlay are used
- **THEN** ready requires the same-row snapshot/fingerprint, matching model/dimension, existing completion marker, current unexpired generation and clean qualified overlay; missing/malformed/expired evidence cannot qualify ready.
- **AND** verification uses S7 in verification.md.

#### Scenario: Query-only provider failure
- **WHEN** an already qualified index encounters a transient query embedding timeout
- **THEN** its indexing target remains ready while the existing lexical fallback/degraded/freshness behavior is preserved; no health probe is added.
- **AND** verification uses S8 in verification.md.

#### Scenario: Graph and reference owner states
- **WHEN** graph is stale/reloading/failed or the reference loader is uninitialized/loading/ready/failed
- **THEN** targets reflect those owner lifecycles under design.md, without fabricated intermediate phases/counters or cross-target readiness.
- **AND** verification uses S9 in verification.md.

### Requirement: Coherent optional counters and terminal transitions

R3. Progress SHALL represent one generation, pass, phase and unit. Known completed SHALL NOT exceed total. Unknown/zero totals SHALL NOT imply percentages. Terminal/inactive/waiting states SHALL have null progress. New pass updates and cleanup SHALL be fenced against previous generations.

#### Scenario: Concurrent transitions and restart
- **WHEN** sampling overlaps begin/reset/phase/update/finish, old callbacks arrive, or the process restarts
- **THEN** one coherent sample or unknown/null is returned, pass identities do not alias, and old callbacks cannot overwrite a newer attempt.
- **AND** verification uses S10 in verification.md.

#### Scenario: Terminal outcomes and request cancellation
- **WHEN** a native worker fails, is cancelled or superseded after partial work
- **THEN** its terminal outcome and pass identity remain until a new begin with null progress; cleanup cannot mark it ready, and cancelling a request alone does not cancel shared indexing.
- **AND** verification uses S11 in verification.md.

### Requirement: Bounded observational behavior

R4. Telemetry SHALL introduce no build, provider call, full-store scan, extra SQL round trip or engine-lock wait. Existing lazy starts/probes MAY remain. Short snapshot locks SHALL exclude I/O, engine locks and formatting. Fields SHALL contain only the bounded safe vocabulary in design.md.

#### Scenario: Slow owner and safe fields
- **WHEN** an engine resource is held during slow work or a snapshot owner is contended while status is polled
- **THEN** telemetry uses a nonblocking snapshot or unknown/null and issues no additional work; native error text, endpoints, credentials, source text and absolute customer paths never appear in indexing.
- **AND** verification uses S12 in verification.md.

### Requirement: Versioned discovery and response budgets

R5. The server SHALL publish the exact schema/machine versions and fingerprints in design.md, require indexing in every affected schema branch, and preserve legacy meanings. Budgeted responses SHALL include all mandatory telemetry and obey the defined byte bound; documented existing exemptions SHALL remain.

#### Scenario: Strict discovery
- **WHEN** a client reads tools/list and the contract resource and validates actual responses
- **THEN** machine 3.0, search 5/status 2, graph descriptor 34 and indexing 1 agree with advertised schemas/fingerprints; removing indexing from any affected response fails validation while unaffected graph data and list_platform remain valid.
- **AND** verification uses S13 in verification.md.

#### Scenario: Exact and undersized budgets
- **WHEN** a budgeted search or graph-loading response is at its minimum, below it, empty or degraded
- **THEN** the exact-fit response obeys B <= 4 * max_output_tokens; an undersized request returns -32602 budget_too_small with minimum_output_tokens and never an oversized success or missing indexing.
- **AND** verification uses S14 in verification.md.

#### Scenario: Text and legacy agreement
- **WHEN** text and legacy progress accompany indexing during active, waiting, persisting or terminal work
- **THEN** all projections use one captured snapshot, no stale counters or zero-total percentages appear, and JSON alone conveys the lifecycle.
- **AND** verification uses S15 in verification.md.
