import React from "react";
import { Badge } from "@/components/ui/Badge";

export interface CapabilitySet {
  token: boolean; model: boolean; session: boolean; project: boolean;
  cache_tokens: boolean; cost: boolean;
}

/** 能力徽标组：源数据没有的维度不显示（能力诚实）。 */
export function CapabilityBadge({ caps }: { caps: CapabilitySet }) {
  const items: Array<{ key: string; label: string; on: boolean }> = [
    { key: "token", label: "Tokens", on: caps.token },
    { key: "model", label: "模型", on: caps.model },
    { key: "session", label: "会话", on: caps.session },
    { key: "project", label: "项目", on: caps.project },
    { key: "cache", label: "缓存", on: caps.cache_tokens },
    { key: "cost", label: "费用", on: caps.cost },
  ];
  return (
    <span className="inline-flex flex-wrap gap-1">
      {items.filter((i) => i.on).map((i) => (
        <Badge key={i.key} variant="brand">{i.label}</Badge>
      ))}
      {items.every((i) => !i.on) && <Badge variant="mute">仅元数据</Badge>}
    </span>
  );
}
