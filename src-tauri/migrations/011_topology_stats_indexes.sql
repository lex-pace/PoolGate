-- Topology V2 stats indexes: batch GROUP BY over provider / account needs
-- covering lookups by provider_id, account_id and request_at. The provider
-- inspector and the four-layer topology both aggregate real upstream attempts.
CREATE INDEX IF NOT EXISTS idx_request_logs_provider_request_at
    ON request_logs(provider_id, request_at);

CREATE INDEX IF NOT EXISTS idx_request_logs_account_provider
    ON request_logs(account_id, provider_id, request_at);
