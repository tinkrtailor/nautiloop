-- Add merge_events table for tracking PR merge history (NFR-8)
CREATE TABLE IF NOT EXISTS merge_events (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    loop_id         UUID NOT NULL REFERENCES loops(id) ON DELETE CASCADE,
    merge_sha       TEXT NOT NULL,
    merge_strategy  TEXT NOT NULL,
    ci_status       TEXT NOT NULL DEFAULT 'unknown',
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX idx_merge_events_loop_id ON merge_events(loop_id);
