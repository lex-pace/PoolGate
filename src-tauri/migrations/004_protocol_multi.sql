-- Version 4: multi-protocol (responses / chat / anthropic / gemini) + route takeover
ALTER TABLE providers ADD COLUMN protocols TEXT;
ALTER TABLE providers ADD COLUMN route_takeover INTEGER NOT NULL DEFAULT 1;
ALTER TABLE accounts ADD COLUMN protocols TEXT;
ALTER TABLE accounts ADD COLUMN route_takeover INTEGER NOT NULL DEFAULT 1;
UPDATE providers SET protocols = json_array(protocol) WHERE protocols IS NULL;
