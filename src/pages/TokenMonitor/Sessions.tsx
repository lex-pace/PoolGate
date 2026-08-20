import React from "react";
import { Card, CTitle, CBody } from "@/components/ui/Card";
import { ActiveSessionList } from "@/components/token-monitor/ActiveSessionList";
import { useActiveSessions } from "@/components/token-monitor/token-monitor-data";

export default function Sessions() {
  const { data: sessions } = useActiveSessions();

  return (
    <Card>
      <div className="pr-4 pt-3">
        <CTitle>会话（仅元数据摘要，不读正文）</CTitle>
      </div>
      <CBody>
        <ActiveSessionList sessions={sessions ?? []} />
      </CBody>
    </Card>
  );
}
