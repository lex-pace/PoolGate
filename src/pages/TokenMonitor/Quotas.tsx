import React from "react";
import { Card, CTitle, CBody } from "@/components/ui/Card";
import { Badge } from "@/components/ui/Badge";
import { QuotaWindowCard } from "@/components/token-monitor/QuotaWindowCard";
import { useQuotaAccounts } from "@/components/token-monitor/token-monitor-data";
import { accountDisplayName, accountIdentity } from "@/lib/account-display";
import { useAccountDisplay } from "@/components/ui/AccountDisplay";

export default function Quotas() {
  const { data: accounts } = useQuotaAccounts();
  const { mode } = useAccountDisplay();

  return (
    <div className="grid grid-cols-1 lg:grid-cols-2 gap-4">
      {(accounts ?? []).map((account) => (
        <Card key={account.account_id}>
          <div className="flex items-center justify-between pr-4 pt-3">
            <CTitle>
              <span className="flex items-center gap-2">
                {accountDisplayName(account, mode)}
                <Badge variant={account.status === "active" ? "ok" : "err"} dot>
                  {account.status === "active" ? "已连接" : "异常"}
                </Badge>
              </span>
            </CTitle>
            <span className="text-[11px] text-[var(--text-tertiary)] tabular-nums">
              {accountIdentity(account, mode) || "身份不可用"}
            </span>
          </div>
          <CBody>
            <div className="mb-3 flex items-center gap-2 text-[11px] text-[var(--text-tertiary)]">
              <span>{account.plan_name || "计费方案不可用"}</span>
              <span>·</span>
              <span>{account.windows.length} 个额度窗口</span>
              {account.last_success_at && (
                <>
                  <span>·</span>
                  <span>更新于 {new Date(account.last_success_at).toLocaleTimeString("zh-CN", { hour12: false })}</span>
                </>
              )}
            </div>
            <div className="grid grid-cols-1 gap-2.5">
              {account.windows.map((w) => (
                <QuotaWindowCard key={w.window_key} window={w} />
              ))}
            </div>
          </CBody>
        </Card>
      ))}
      {(accounts ?? []).length === 0 && (
        <Card className="lg:col-span-2">
          <CBody>
            <div className="py-12 text-center text-sm text-[var(--text-tertiary)]">
              尚未绑定额度账号。可绑定 Claude / Codex / DeepSeek / OpenRouter 等账号查看剩余额度。
            </div>
          </CBody>
        </Card>
      )}
    </div>
  );
}
