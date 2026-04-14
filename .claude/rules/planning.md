---
paths:
  - "docs/**"
  - "**/roadmap.md"
  - "**/phase-*.md"
  - "**/status.md"
  - "**/frontend-summary.md"
  - "**/*VERIFICATION*"
---

# Development Plan Writing Rules

> **Desktop App Adaptation (VoiceGate):** The phase-file template below is the original SQLx/Axum version. VoiceGate is a Rust desktop app with no database, no HTTP API, and no ORM. Phase files under `docs/voicegate/` use an **adapted 7-section template** — see the table at the end of this file. The rest of the methodology (roadmap, research log, gap analysis passes, verification YAML) applies unchanged.

All development plans follow a structured methodology. Plans live in `docs/<plan-name>/` and consist of 6 mandatory files.

## Plan Structure

| File | Purpose |
|------|---------|
| `roadmap.md` | Master blueprint: phase table, dependency graph, entity/engine inventories, gap analysis log |
| `research.md` | Domain research (APIs, algorithms, protocols), decisions log with date/decision/rationale |
| `phase-XX.md` | Individual phase specs -- the core deliverable (see template below) |
| `status.md` | Living tracker: phase status table, cumulative totals, blockers |
| `frontend-summary.md` | Cross-team handoff -- updated after EACH phase completion, never batched |
| `PHASES-X-Y-Z-VERIFICATION.md` | Verification reports with YAML frontmatter (score, status, per-truth pass/fail) |

## Roadmap.md Sections (in order)

1. **Header** -- Title, start date, target description, scope with hard numbers (entities, routes, engines, reports)
2. **Phase Overview Table** -- Columns: #, Phase Name, Scope, Size (S/M/L/XL), Depends On, Status
3. **Dependency Graph** -- ASCII art showing phase relationships and parallel tracks
4. **New Entities by Phase** -- Table mapping each phase to new database tables
5. **New Business Engines by Phase** -- Table mapping each phase to new domain services
6. **Gap Analysis Additions** -- Running log updated after each pass (count, categories, distribution across phases)

## Phase File Template (SQLx/Axum)

Every `phase-XX.md` MUST have these sections in this exact order:

### Header
```
# Phase N: <Name>

**Goal:** <One sentence describing what this phase delivers>

**Dependencies:** Phase X, Phase Y (or "None")
**Complexity:** S | M | L | XL
```

### Section 1: SQLx Migration Changes
- New tables: full SQL migration blocks, copy-paste ready
- Modified tables: ALTER TABLE statements with exact column definitions
- New enum types: DO $$ BEGIN CREATE TYPE ... END $$; blocks
- Indexes: CREATE INDEX IF NOT EXISTS statements
- All SQL must use `IF NOT EXISTS` for idempotency
- Use `TIMESTAMPTZ` (never TIMESTAMP), `UUID PRIMARY KEY DEFAULT gen_random_uuid()`

Example:
```sql
CREATE TABLE IF NOT EXISTS calls (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    organization_id UUID NOT NULL REFERENCES organizations(id),
    agent_id UUID NOT NULL REFERENCES agents(id),
    phone_number VARCHAR(20) NOT NULL,
    status VARCHAR(50) NOT NULL DEFAULT 'pending',
    started_at TIMESTAMPTZ,
    ended_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_calls_org_id ON calls(organization_id);
CREATE INDEX IF NOT EXISTS idx_calls_agent_id ON calls(agent_id);
CREATE INDEX IF NOT EXISTS idx_calls_status ON calls(status);
```

### Section 2: DDD Implementation (Rust)
- **Domain entity**: struct definition with derives (`Debug, Serialize, Deserialize, FromRow, ToSchema`)
- **Create/Update structs**: with `Deserialize, ToSchema` derives, Update fields all `Option<T>`
- **Repository trait**: method signatures with parameter types (in `domain/repositories/`)
- **Repository impl**: `PgXxxRepository` notes on joins, RETURNING clause usage (in `infrastructure/repositories/`)
- **utoipa schemas**: list of request/response DTOs and their purpose
- **Route table**: `| Method | Path | Description |` format (Axum handlers in `presentation/routes/`)
- **Router function**: `pub fn router() -> Router<AppState>`

### Section 3: Business Logic
- Domain service structs with method signatures and trait bounds
- `thiserror` error enum variants for this domain
- Step-by-step logic for each method (numbered steps, not prose)
- Configuration and settings (stored in DB or environment)

### Section 4: Infrastructure Updates
- Tenant scoping: which new tables get `organization_id` (vs inherit via FK)
- Redis job definitions (or "No new jobs needed")
- Docker compose changes (or "No infrastructure changes needed")
- Existing entity/schema/route updates required by this phase

### Section 5: Verification
Numbered checklist of concrete steps:
1. `make migrate` -- migrations apply without error
2. `cargo check` -- compile-time SQL verification passes
3. `cargo clippy -- -D warnings` -- no lint errors
4. `cargo test` -- all tests pass
5. MCP-curl test each endpoint (specific examples with expected responses)
6. Check Swagger UI at `http://localhost:3000/swagger-ui` -- new endpoints appear
7. `make generate-api` -- Orval regenerates frontend types
8. Run existing tests -- no regressions

### Section 6+: PRD Gap Additions
Appended by gap analysis passes. Numbered subsections (6.1, 6.2, ...) each containing:
- Migration additions with exact SQL
- New route tables
- Business logic additions
- Reference to gap ID and severity

---

## Gap Analysis Methodology (Mandatory)

Iterative passes are REQUIRED. Do not mark a plan "ready for implementation" until a verification pass finds 0 true gaps.

### Pass 1 (Initial)
After phase files are written, compare EVERY PRD entity, field, endpoint, business rule, state machine, setup screen, and integration point against phase specs. Log each gap with:
- **Severity:** CRITICAL / HIGH / MEDIUM / LOW
- **Category:** Missing Table, Missing Fields, Missing Endpoint, Missing Logic, Missing Report, Missing Setup, Missing Integration, Missing Dashboard, Incomplete Coverage
- **Target phase** for incorporation

Append gaps to respective phase files as Section 6.x subsections. Update `roadmap.md` gap log.

### Pass 2+ (Iterative)
Re-compare updated phase files against PRD. Focus areas that passes commonly miss:
- State machines (transition tables, type-specific rules)
- Field completeness (compare every PRD field against schema)
- Integration points (events published/consumed, notification triggers)
- Setup/config endpoints
- Report drill-down and dynamic grouping params
- AI-specific: prompt templates, model configuration, fallback chains

Continue passes until a pass finds 0 true gaps.

### Verification Pass (Final)
Audit N representative items across all phases (mix of Critical/High/Medium/Low). For each item, verify it exists in the phase files with:
- Complete SQL migration
- Route table entry
- Repository trait method
- Business logic description

Report as YAML frontmatter in `PHASES-X-Y-Z-VERIFICATION.md`:
```yaml
---
phase: <plan-name>-phases-X-Y-Z
verified: <ISO timestamp>
status: complete | gaps_found
score: N/M must-haves verified
gaps: [...]
---
```

## Status.md Sections

1. **Phase Status Table** -- Columns: #, Phase, Status, Started, Completed, Tables Added, Tables Modified, Routes Added, Services Added
2. **Cumulative Totals** -- Columns: Metric, Before, Current, Target
3. **Gap Analysis Summary** -- Per-pass summary with counts and category breakdown
4. **Blockers & Notes** -- Critical dependencies, blocking items, parallel track notes

## Rules

- SQL migrations MUST be copy-paste ready -- exact column names, types, constraints, indexes
- Route tables MUST use `| Method | Path | Description |` format
- Service specs MUST include method signatures and numbered step-by-step logic
- Each phase MUST state what it does NOT touch ("No new tenant-scoped tables", "No new Redis jobs")
- Verification steps MUST be concrete (specific commands, specific assertions), never vague ("verify it works")
- Frontend summary MUST be updated after EACH phase, not batched at the end
- Phase sizes: S (<5 routes), M (5-15 routes), L (15-25 routes), XL (25+ routes or complex engines)
- Gap severity: CRITICAL = blocks other phases, HIGH = missing core functionality, MEDIUM = missing enhancement, LOW = nice-to-have

---

## Desktop App Phase Template (VoiceGate)

VoiceGate is a Rust desktop binary, not a backend API. The SQLx migrations / DDD layering / Axum routes sections do not apply. Phase files under `docs/voicegate/` use these **7 sections in order**:

| # | Section | Purpose |
|---|---------|---------|
| 1 | **Module & File Changes** | Exact paths to create / modify / delete under `src/`, `scripts/`, `models/`, `tests/fixtures/`, `assets/`. Replaces "SQLx Migration Changes". |
| 2 | **Dependencies & Build Config** | New `Cargo.toml` entries (crate, version, features, `[target.'cfg(...)'.dependencies]`), feature flags, `[profile.release]` changes, ONNX Runtime shared library handling, Python model-download scripts. |
| 3 | **Types, Traits & Public API** | Struct definitions with derives, trait signatures (`VirtualMic`, `SpeakerVerifier`, `AudioGate`), `thiserror` error enums, config struct additions. This is the contract surface and replaces "DDD Implementation". |
| 4 | **Runtime Behavior** | Numbered step-by-step logic per subsystem. Threading model ownership (which thread owns what). Real-time constraints: MUST NOT allocate / lock / block in audio callbacks. Ring-buffer sizing. ONNX session lifetime. Stateful model state (Silero VAD GRU) persistence. EMA + hysteresis math. Replaces "Business Logic". |
| 5 | **Cross-Platform & Resource Handling** | `#[cfg(target_os)]` split points. File-path resolution via `dirs`. Model asset lookup. Permission/capability errors (PipeWire perms, VB-Cable missing). Graceful device disconnect. Replaces "Infrastructure Updates". |
| 6 | **Verification** | Concrete commands: `cargo check`, `cargo clippy -- -D warnings`, `cargo test <specific_test_name>`, manual smoke tests. Phase-specific acceptance thresholds (discrimination cosine < 0.5, latency < 50 ms, CPU < 10%). |
| 6+ | **PRD Gap Additions** | Same methodology as above — numbered 6.1, 6.2, ... appended by gap-analysis passes. |

**Roadmap table adaptations for VoiceGate:**
- "New Entities by Phase" -> **"New Modules by Phase"** (Rust source modules under `src/`)
- "New Business Engines by Phase" -> **"New ONNX Models & External Assets by Phase"** (ONNX files, shell scripts, fixture WAVs)

**Phase size rubric for VoiceGate** (replaces the route-count rubric, which does not apply):
- **S**: <3 new modules, no new crates
- **M**: 3-6 new modules or 1-3 new crates
- **L**: >6 new modules OR requires a new ONNX model OR introduces a new platform backend
- **XL**: Reserved for v2+ items

**Always-applicable rules** (unchanged from the backend template):
- Verification steps MUST be concrete
- Frontend summary updated after EACH phase, not batched
- Gap severity CRITICAL/HIGH/MEDIUM/LOW, same meaning
- Iterative passes until 0 true gaps
- Final YAML verification report

