-- ============================================================
-- 020: 自定义应用监控
--   tool_definition 增加 custom_fields_json：自定义应用的可选 JSONL 字段映射。
--   结构：{"input":["/usage/input_tokens",...],"output":[...],"cache":[...],
--          "model":[...],"ts":[...],"cost":[...]}；留空/NULL = 使用内置通用默认字段。
--   自定义应用本身 = tool_definition 中 tool_id 以 `custom:` 开头的行
--   （display_name = 应用名，custom_paths_json = 日志路径列表）。
-- ============================================================

ALTER TABLE tool_definition ADD COLUMN custom_fields_json TEXT;
