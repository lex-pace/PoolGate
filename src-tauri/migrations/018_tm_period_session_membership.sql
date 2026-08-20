-- ============================================================
-- 018: tm_period_session — Token Monitor 各周期会话成员表
--
-- 对齐开源 Token Monitor：会话列表按「tokscale 周期扫描」决定成员（--today /
-- --week / --month 各返回该周期内有活动的会话），而非用会话文件 mtime 的
-- last_active_at 做日期过滤（旧逻辑对部分工具解析不到文件，全部塌缩到扫描时刻，
-- 导致「今日/近7天」会话缺失且活动时间错误）。
--
-- 采集时写入：period = day|7d|month，session_id = tokscale 原始 sessionId
-- （与 usage_event.session_id / tm_session.external_session_id 同值）。
-- 旧库 CREATE TABLE 安全（新表）；清理由快照替换逻辑负责（每次重建）。
-- ============================================================
CREATE TABLE IF NOT EXISTS tm_period_session (
    period      TEXT NOT NULL,
    session_id  TEXT NOT NULL,
    tool_id     TEXT NOT NULL DEFAULT '',
    -- 该周期内 (client, session, model) 组消息数之和（展示用，可空）
    message_count INTEGER,
    PRIMARY KEY (period, session_id)
);
CREATE INDEX IF NOT EXISTS idx_tm_period_session_period
    ON tm_period_session (period);
