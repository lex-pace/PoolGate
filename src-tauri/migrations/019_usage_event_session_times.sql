-- ============================================================
-- 019: usage_event.session_started_at / session_last_active_at
--
-- 会话级真实时间戳（UTC ISO8601，可空）：
-- - tokscale 权威采集从会话文件真实时间解析填充（首条/末条消息时间戳，
--   文件缺失时回退 mtime；都不存在为 NULL）；
-- - 手写适配器/导入数据为 NULL。
--
-- 会话投影据此聚合 started_at / last_active_at（替代此前从 occurred_at
-- 取 MIN/MAX 的 mtime 兜底口径），让会话列表排序与下钻时间完全精确。
-- 纯元数据追加，不读正文；旧库 ALTER 安全（NULL 即未知）。
-- ============================================================
ALTER TABLE usage_event ADD COLUMN session_started_at TEXT;
ALTER TABLE usage_event ADD COLUMN session_last_active_at TEXT;
