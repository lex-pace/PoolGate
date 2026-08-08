import React, { useState } from "react";
import { Card, CBody } from "@/components/ui/Card";
import { Button } from "@/components/ui/Button";
import { Input } from "@/components/ui/Input";

interface Props { onComplete: () => void; }

export default function Wizard({ onComplete }: Props) {
  const [step, setStep] = useState(1);
  const [copied, setCopied] = useState(false);

  return (
    <div className="flex items-center justify-center min-h-full">
      <div className="w-full max-w-lg space-y-6">
        {/* 步骤指示器 */}
        <div className="flex items-center justify-center gap-2">
          {[1, 2, 3].map((s) => (
            <div key={s} className="flex items-center gap-2">
              <div className={`w-8 h-8 rounded-full flex items-center justify-center text-sm font-medium transition-colors
                ${s < step ? "bg-[var(--color-ok)] text-white" : s === step ? "bg-[var(--color-brand)] text-white" : ""}`}
                style={{ backgroundColor: s > step ? "var(--bg-hover)" : undefined, color: s > step ? "var(--text-dim)" : undefined }}>
                {s < step ? "✓" : s}
              </div>
              {s < 3 && <div className={`w-10 h-0.5 ${s < step ? "bg-[var(--color-ok)]" : ""}`} style={{ backgroundColor: s >= step ? "var(--bg-hover)" : undefined }} />}
            </div>
          ))}
        </div>

        {/* Step 1 */}
        {step === 1 && (
          <Card>
            <CBody className="space-y-4 pt-2">
              <div className="text-center">
                <h2 className="text-lg font-bold" style={{ color: "var(--text-primary)" }}>Step 1/3</h2>
                <p className="text-xs mt-1" style={{ color: "var(--text-dim)" }}>添加你的第一个服务商</p>
              </div>
              <div className="grid grid-cols-2 gap-3">
                {["从 Sub2API 导入", "从 CPA 导入"].map((t) => (
                  <div key={t}
                    className="border-2 border-dashed rounded-md p-5 text-center cursor-pointer transition-colors hover:border-[var(--color-brand)]"
                    style={{ borderColor: "var(--border-default)" }}>
                    <div className="text-2xl mb-1">📂</div>
                    <div className="text-sm font-medium" style={{ color: "var(--text-primary)" }}>{t}</div>
                  </div>
                ))}
              </div>
              <div className="relative"><div className="absolute inset-0 flex items-center"><div className="w-full border-t" style={{ borderColor: "var(--border-default)" }} /></div><div className="relative flex justify-center"><span className="px-2 text-xs" style={{ backgroundColor: "var(--bg-surface)", color: "var(--text-dim)" }}>或手动添加</span></div></div>
              <Input label="服务商名称" placeholder="如: OpenAI Official" />
              <Input label="Base URL" defaultValue="https://api.openai.com/v1" />
              <Input label="API Key" type="password" placeholder="sk-..." />
            </CBody>
          </Card>
        )}

        {/* Step 2 */}
        {step === 2 && (
          <Card>
            <CBody className="space-y-4 pt-2">
              <div className="text-center">
                <h2 className="text-lg font-bold" style={{ color: "var(--text-primary)" }}>Step 2/3</h2>
                <p className="text-xs mt-1" style={{ color: "var(--text-dim)" }}>创建分组并分配账号</p>
              </div>
              <Input label="分组名称" placeholder="如: Claude 专用" />
              <div className="p-4 rounded-md border text-sm" style={{ backgroundColor: "var(--bg-elevated)", borderColor: "var(--border-default)" }}>
                <div className="font-medium mb-1" style={{ color: "var(--text-primary)" }}>当前可用账号</div>
                <div className="text-xs" style={{ color: "var(--text-dim)" }}>添加服务商后，账号将自动出现在此处</div>
              </div>
            </CBody>
          </Card>
        )}

        {/* Step 3 */}
        {step === 3 && (
          <Card>
            <CBody className="space-y-4 pt-2">
              <div className="text-center">
                <h2 className="text-lg font-bold" style={{ color: "var(--text-primary)" }}>Step 3/3</h2>
                <p className="text-xs mt-1" style={{ color: "var(--text-dim)" }}>复制配置，开始使用</p>
              </div>
              <p className="text-xs text-center" style={{ color: "var(--text-dim)" }}>所有工具统一连接 <span style={{ color: "var(--color-brand)" }}>http://127.0.0.1:9800</span></p>
              <div className="space-y-3">
                {[
                  { name: "Claude Code", code: 'export ANTHROPIC_BASE_URL="http://127.0.0.1:9800"' },
                  { name: "Codex / Grok / MiMo", code: 'export OPENAI_BASE_URL="http://127.0.0.1:9800/v1"' },
                ].map((t) => (
                  <div key={t.name} className="p-4 rounded-md border" style={{ backgroundColor: "var(--bg-elevated)", borderColor: "var(--border-default)" }}>
                    <div className="text-sm font-medium mb-2" style={{ color: "var(--text-primary)" }}>{t.name}</div>
                    <pre className="text-xs mb-3" style={{ color: "var(--text-secondary)" }}>{t.code}</pre>
                    <Button size="sm" variant="secondary" className="w-full" onClick={() => { navigator.clipboard.writeText(t.code); setCopied(true); setTimeout(() => setCopied(false), 2000); }}>
                      {copied ? "✓ 已复制" : "📋 复制配置"}
                    </Button>
                  </div>
                ))}
              </div>
            </CBody>
          </Card>
        )}

        {/* 导航 */}
        <div className="flex items-center justify-between">
          <Button variant="ghost" size="sm" disabled={step === 1} onClick={() => setStep(step - 1)}>← 上一步</Button>
          <div className="flex gap-2">
            <Button variant="ghost" size="sm" onClick={onComplete}>跳过</Button>
            {step < 3 ? (
              <Button size="sm" onClick={() => setStep(step + 1)}>下一步 →</Button>
            ) : (
              <Button size="sm" variant="success" onClick={onComplete}>✓ 完成，开始使用</Button>
            )}
          </div>
        </div>
      </div>
    </div>
  );
}
