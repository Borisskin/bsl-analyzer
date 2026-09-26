## ADDED Requirements

### Requirement: Startup provenance and baseline
The system SHALL record process and store identity, available build identity, workspace roots, mode, schema/text versions and read-consistent initial counts before its startup migrations or ingest mutations. Collection SHALL occur outside request paths and lease critical sections. Missing, unavailable and unstable identity/counts SHALL be distinct. Baseline, overlay rows and overlay value-cache counts SHALL remain separate.

#### Scenario: Existing index is reopened
- **WHEN** a daemon opens a populated workspace index
- **THEN** its initial vector counts and identities are captured before its migrations or reindexing
- **AND** overlapping daemon generations and their operations remain distinguishable

#### Scenario: Missing unreadable or legacy store
- **WHEN** startup observes an absent database, an unreadable database or an older schema
- **THEN** the snapshot distinguishes absent zero rows from unavailable values and counts supported legacy tables before migration
- **AND** telemetry does not repair/create the database or change its normal open outcome

### Requirement: Explain reindex decisions
The system SHALL distinguish unchanged content, missing records, changed hashes, cleared hashes, hash lookup errors, read failures, context changes, deletion, migrations, root/mode transitions and explicit rebuilds without changing indexing policy. Decision context SHALL propagate to mutation owners across worker boundaries. Unknown historical causes SHALL remain unknown.

#### Scenario: Hash lookup fails
- **WHEN** reading a stored hash fails in the fused ingestion path
- **THEN** the failure is recorded as a lookup error with operation and bounded file identity
- **AND** it is not reported as a new file or proven content change, and the existing fallback outcome is preserved

#### Scenario: Decision matrix
- **WHEN** unchanged, missing, empty-hash, changed-hash, unreadable-file, context or root/mode cases are processed
- **THEN** the corresponding reason and summary count are recorded under the correct operation
- **AND** decisions, retry/refusal handling and indexing outcomes match the existing policy

### Requirement: Record committed vector invalidation
Every production path deleting existing SQLite vectors or setting them to NULL SHALL emit operation intent and outcome, including schema resets. The audited active transactional paths SHALL report exact prior non-NULL counts, separately from hash metadata, overlay cache entries and embedding writes. Counts SHALL NOT be derived from matched chunk rows or generation deltas. Existing SQL predicates, transaction boundaries, locking/fencing and generation semantics SHALL remain unchanged. Telemetry read/registration failures and the explicitly inventoried dormant autocommit primitives MAY report unavailable preimages; unavailable SHALL be null with count quality, never an invented zero. No normal active transactional path MAY use this exception to avoid exact attribution.

#### Scenario: Mutation commits
- **WHEN** a replacement, deletion or context update commits
- **THEN** the journal associates actual lost-vector counts with reason, process and operation after the existing commit succeeds
- **AND** cascade and overlapping selectors are counted once at the transaction owner

#### Scenario: Mutation rolls back
- **WHEN** a mutation fails or is cancelled before commit
- **THEN** its tentative counts are not reported as committed vector loss
- **AND** an unconfirmed rollback or commit outcome is explicitly unknown

#### Scenario: Cancellation follows committed batches
- **WHEN** earlier batches commit and a later batch is refused or cancelled
- **THEN** the parent terminal event preserves earlier committed totals and excludes the uncommitted batch

#### Scenario: Hash-only or preserving migration
- **WHEN** an embedding-text migration clears hashes or a root-key migration preserves embeddings
- **THEN** the event reports zero immediate logical vector loss and the relevant metadata/version transition
- **AND** a later destructive reindex owns its own actual loss count

#### Scenario: Dormant primitive or telemetry read failure
- **WHEN** an inventoried autocommit API lacks an atomic preimage or telemetry cannot safely observe it
- **THEN** successful statement effects are distinguished from unavailable vector counts
- **AND** instrumentation adds no transaction or indexing failure

#### Scenario: Derived artifact is rebuilt
- **WHEN** a persisted or live vector index is removed, rejected or rebuilt from SQLite embeddings
- **THEN** the journal identifies artifact reason/outcome and zero SQLite vector loss from that artifact operation
- **AND** filesystem removal is not reported as rolled back merely because a surrounding SQLite transaction rolls back

### Requirement: Observe embedding resume
The system SHALL log embedding pass start/resume and completed, interrupted, failed or skipped outcomes with committed writes and pending counts where available. An unchanged warm restart SHALL retain embeddings and a partial restart SHALL resume missing embeddings under the existing indexing policy.

#### Scenario: Restart with a partially embedded index
- **WHEN** unchanged files contain both ready and pending embeddings
- **THEN** existing vectors remain intact and the pass reports continuation of pending work

#### Scenario: Warm restart and pass termination
- **WHEN** a fully embedded unchanged fixture restarts, or a pass completes, fails, is interrupted or is skipped
- **THEN** restart has zero committed vector losses and no redundant embedding submissions
- **AND** each started pass has an honest terminal outcome when normal control flow permits, retaining any earlier committed writes

### Requirement: Persistent bounded diagnostic output
Valid MCP server invocations SHALL persist the dedicated lifecycle target automatically through existing tracing infrastructure, subject to explicit target filtering. Storage SHALL use the platform user state directory and the design's shared ring: eight segments of at most 4 MiB plus one empty lock file per workspace. Normal records SHALL use bounded operation summaries at 128 files or one second and a final partial summary, with at most ten file examples each; per-file transaction detail SHALL be DEBUG-only. Every record SHALL be at most 8 KiB, and queued payload SHALL be at most 4 MiB per process. Storage SHALL survive ordinary process restart and reboot, with restrictive access and safe overlapping-writer behavior. Sources, vectors, secrets, endpoint URLs, lease tokens and raw error text SHALL NOT be logged. Stdout SHALL remain protocol-only.

#### Scenario: Restart and rotation
- **WHEN** a daemon restarts and retention thresholds are reached
- **THEN** retained prior-run records remain readable, with whole-record rotation within the shared byte/file bound
- **AND** a partial last record does not corrupt subsequent appended records

#### Scenario: Journal cannot be written
- **WHEN** state-directory discovery, permissions, lock or write operations fail
- **THEN** a rate-limited existing diagnostic channel reports the failure without recursion or protocol stdout
- **AND** search behavior remains unchanged even when that fallback channel is also unavailable

#### Scenario: Concurrent writers and privacy
- **WHEN** two daemon generations write for the same workspace or encounter an unsafe filesystem target
- **THEN** the shared lock serializes complete records/rotation within the global workspace bound
- **AND** private Unix modes or Windows DACLs protect output; symlink/reparse/foreign targets disable logging instead of modifying unrelated files

#### Scenario: Queue overflow and filtering
- **WHEN** the queue saturates or logging filters change
- **THEN** producers remain nonblocking and dropped records are counted for the next deliverable gap summary
- **AND** the journal defaults to scoped INFO even under broad default warn, while explicit target off/debug is honored

### Requirement: Bounded overhead and honest evidence
Instrumentation SHALL avoid new full-corpus scans per file/batch, request-time journal filesystem work and journal-related lease blocking. Startup snapshots and actual whole-store reset counts MAY scan once at their defined boundaries. Documentation SHALL state that crashes between commit and journal delivery, buffered summary/queue loss, unavailable storage and retention eviction leave ambiguous history. Power-loss durability and proof of an unobserved historical root cause SHALL NOT be claimed. Validation SHALL use isolated deterministic fixtures and existing repository checks without live embedding-provider or production-reindex prerequisites.

#### Scenario: Large operation
- **WHEN** many files are replaced
- **THEN** normal output and counters follow bounded batch aggregation, not per-chunk logging or repeated corpus scans
- **AND** a stalled writer cannot block indexing or lease progress

#### Scenario: Failure and collection evidence
- **WHEN** the commit-to-delivery window or sink failure is exercised and an operator collects retained segments
- **THEN** indexing matches control, collection follows documented generation ordering, and missing/unavailable evidence is explicit
- **AND** validation records actual executed checks separately from unexecuted platform checks without requiring installation, publication or a new approval to complete the local handoff
