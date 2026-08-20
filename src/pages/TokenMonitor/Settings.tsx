import React from "react";
import { Card, CTitle, CBody } from "@/components/ui/Card";
import { Badge } from "@/components/ui/Badge";

const principles: Array<{ title: string; body: string; tone: "ok" | "warn" | "mute" | "brand" }> = [
  { title: "隐私优先", body: "不持久化 Prompt / Response / 源代码 / 文件正文；项目路径以不可逆 Hash 存储；凭证只进系统钥匙串，数据库仅存引用。", tone: "ok" },
  { title: "能力诚实", body: "源数据没有的维度一律显示「不可用」，不猜测、不填 0。", tone: "warn" },
  { title: "性能红线", body: "无定时全盘扫描；SQLite 批量写入、远离请求热路径；空闲 CPU < 1%，内存增量 < 80MB；托盘打开 P95 < 150ms。", tone: "mute" },
  { title: "不二次采集", body: "网关用量以 request_logs 为权威源，采集事件与其 UNION 合并去重，同一 request_id 不重复计量。", tone: "brand" },
];

export default function SettingsView() {
  return (
    <div className="grid grid-cols-1 lg:grid-cols-2 gap-4">
      <Card>
        <CTitle>运行原则</CTitle>
        <CBody className="space-y-3">
          {principles.map((p) => (
            <div key={p.title} className="rounded-xl border border-[var(--border)] p-3">
              <div className="flex items-center gap-2">
                <Badge variant={p.tone} dot>{p.title}</Badge>
              </div>
              <p className="mt-1.5 text-xs leading-5 text-[var(--text-secondary)]">{p.body}</p>
            </div>
          ))}
        </CBody>
      </Card>
      <Card>
        <CTitle>数据源</CTitle>
        <CBody className="space-y-2 text-xs leading-5 text-[var(--text-secondary)]">
          <p>· <strong>网关用量</strong>：request_logs（权威）+ usage_event（采集事件）UNION 合并去重</p>
          <p>· <strong>本地工具</strong>：Claude Code / Codex / Cursor 等本地会话 JSONL（仅读取元数据字段）</p>
          <p>· <strong>额度</strong>：官方 API / 本地登录态 / 仪表盘会话（视供应商能力而定）</p>
          <p>· <strong>调度</strong>：文件系统 Watcher + 检查点，无定时全盘扫描</p>
        </CBody>
      </Card>
    </div>
  );
}
