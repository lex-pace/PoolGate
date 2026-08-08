-- Protocol-specific upstream Base URLs for multi-protocol providers.
-- JSON object example: {"chat":"https://api.example.com/v1","anthropic":"https://api.example.com/anthropic"}
ALTER TABLE providers ADD COLUMN base_urls TEXT;
