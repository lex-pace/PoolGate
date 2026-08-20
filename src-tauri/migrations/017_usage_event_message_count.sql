-- ============================================================
-- 017: usage_event.message_count — 会话消息数（tokscale 会话投影用）
-- 逐轮事件携带的「该 (client, session, model) 组的消息数」；仅 tokscale 权威采集
-- 填充，手写适配器/导入数据为 NULL（会话投影缺失时回退 COUNT(*)）。
-- 纯元数据追加，不读正文；旧库 ALTER 安全（NULL 即未知）。
-- ============================================================
ALTER TABLE usage_event ADD COLUMN message_count INTEGER;
