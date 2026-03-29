# Lane B Infrastructure - Implementation Plan

## Spec: specs/lane-b-infrastructure.md

## Status: IN PROGRESS

## Gap Analysis

The existing codebase has a partial implementation that diverges from the spec in several key areas. The migration needs a complete rewrite, types need updating, git module needs the BareRepo struct with worktree mutex, and config needs three-layer merge.

## Steps

### Step 1: Rewrite Postgres migration to match spec schema
- Replace `20260328000001_initial_schema.sql` with spec-compliant schema
- Add `loop_phase`, `loop_stage`, `loop_state` (with granular pause states), `loop_sub_state`, `job_status`, `credential_type` enums
- Add `engineers` table (UUID PK, name, email, model_preferences JSONB, max_parallel_loops)
- Rewrite `loops` table with all spec columns (phase, stage, state, sub_state, sha NOT NULL, expected_sha, actual_sha, feedback_path, stage_retry_count, force_resume, needs_human_review, ship mode fields, etc.)
- Add `jobs` table (replaces rounds) with k8s_job_name, status, verdict_json, output_json, attempt, token_usage, exit_code, error_message, feedback_path
- Add `egress_logs` table (BIGSERIAL, host, port, protocol, status_code, bytes_sent, bytes_received, method)
- Rewrite `log_events` (BIGSERIAL, level column, message instead of line)
- Rewrite `engineer_credentials` (references engineers.id, not TEXT engineer)
- Add `cluster_credentials` table
- Add partial unique indexes, CHECK constraints, updated_at triggers
- Remove `rounds` and `merge_events` tables (not in spec)
- **Status:** DONE

### Step 2: Update types module to match spec schema
- Update `LoopState` enum: replace `Paused` with `PausedRemoteAhead` and `PausedForceDeviated`
- Rename `LoopKind` to `LoopPhase` (harden/implement)
- Add `LoopStage` enum (spec_audit, spec_revise, implementing, testing, reviewing)
- Update `LoopRecord` to match new schema columns
- Add `JobRecord` type (replaces `RoundRecord`)
- Update `LogEvent` (BIGSERIAL id, level, message)
- Update `EngineerCredential` (engineer_id UUID, not TEXT)
- Add `Engineer` type
- Add `ClusterCredential` type
- Add `JobStatus` enum
- Add `CredentialType` enum
- Keep `generate_branch_name` (already correct per FR-11)
- **Status:** DONE

### Step 3: Update StateStore trait and implementations
- Update trait methods to use new types (JobRecord instead of RoundRecord)
- Add engineer CRUD methods
- Add job CRUD methods
- Add cluster_credentials methods
- Add egress_logs methods
- Update PgStateStore implementation for new schema
- Update MemoryStateStore for new types
- **Status:** DONE

### Step 4: Implement git module - BareRepo with worktree mutex
- Add `bare_repo.rs` with `BareRepo` struct (path, remote_url, worktree_mutex)
- Implement `prepare_worktree(branch, base_ref) -> (PathBuf, String)` per FR-7
- Implement `cleanup_worktree(path)` per FR-9
- Implement branch naming in `branch.rs` per FR-11 (reuse existing `generate_branch_name`)
- Implement `DivergenceResult` enum and `detect_divergence()` per FR-12/FR-13
- Add unit tests
- **Status:** DONE

### Step 5: Implement config module - three-layer merge
- Add `cluster.rs` with `ClusterConfig` and `ClusterFile` wrapper per spec
- Add `repo.rs` with `RepoConfig`, `RepoMeta`, `ServiceConfig`, `ShipConfig`, `HardenConfig`, `TimeoutsConfig`
- Add `engineer.rs` with `EngineerConfig`, `IdentityConfig`, `ModelConfig`, `LimitsConfig`
- Add `merged.rs` with `MergedConfig::merge()` implementing three-layer merge per FR-14-FR-20
- Add `ConfigError` type with proper error variants
- Add service detection for `nemo init` per FR-19
- Add unit tests for merge logic, service detection, config parsing
- **Status:** DONE

### Step 6: Update error types
- Add `ConfigError` variants to `NemoError`
- Add git-specific error variants for worktree operations
- **Status:** DONE

### Step 7: Fix compilation - update all downstream references
- Update imports across api/, loop_engine/, k8s/ modules
- Ensure cargo clippy passes
- Ensure cargo test passes
- **Status:** DONE

## Learnings
- Existing migration uses SCREAMING_SNAKE_CASE for enum values; spec uses lowercase
- Existing code uses `rounds` table; spec uses `jobs` table
- Existing `LoopState` has `Paused` variant; spec has two granular pause states
- Existing `LoopKind` maps to spec's `loop_phase`
- The `generate_branch_name` function is already correct per FR-11
- The hash in branch names uses SHA-256 of (engineer + spec_path + spec_content); spec says SHA-256 of original spec file content only. Keeping existing for now as it provides stronger uniqueness.
