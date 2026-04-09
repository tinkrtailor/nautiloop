-- Issue #96: nemo resume should support FAILED loops, reusing the worktree.
-- Mirrors paused_from_state / reauth_from_state: captures which stage was
-- running when the loop transitioned to Failed so the resume path can
-- redispatch the correct stage without guessing or losing the round.
ALTER TABLE loops ADD COLUMN IF NOT EXISTS failed_from_state loop_state;
