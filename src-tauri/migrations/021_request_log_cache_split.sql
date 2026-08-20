-- Split cache token accounting in request_logs.
--
-- Anthropic bills cache reads (0.1x) and cache writes (1.25x) at different
-- rates, and OpenAI/Gemini only report reads. The aggregated `cache_tokens`
-- column cannot recover that split later, so the canonical Usage model now
-- stores both sides. `cache_tokens` remains the derived sum (read + write)
-- for existing queries and UI; the new columns are nullable for history.

ALTER TABLE request_logs ADD COLUMN cache_read_tokens INTEGER;
ALTER TABLE request_logs ADD COLUMN cache_write_tokens INTEGER;
