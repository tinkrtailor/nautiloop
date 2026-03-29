-- Add resolved_default_branch to loops table.
-- Frozen at loop creation time so PR creation and merge use the same base
-- that /start branched from, regardless of config changes.
ALTER TABLE loops ADD COLUMN IF NOT EXISTS resolved_default_branch TEXT;

-- Backfill existing loops with 'main' as the default to prevent NULL fallback
-- to live cluster config for in-flight loops after migration.
UPDATE loops SET resolved_default_branch = 'main' WHERE resolved_default_branch IS NULL;
