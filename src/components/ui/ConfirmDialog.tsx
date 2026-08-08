import React from "react";
import { Modal } from "./Modal";
import { Button } from "./Button";
import { AlertTriangle } from "lucide-react";

interface ConfirmDialogProps {
  open: boolean;
  onClose: () => void;
  onConfirm: () => void;
  title?: string;
  message: string;
  confirmText?: string;
  cancelText?: string;
  variant?: "danger" | "default";
  loading?: boolean;
}

export function ConfirmDialog({
  open,
  onClose,
  onConfirm,
  title = "确认操作",
  message,
  confirmText = "确定",
  cancelText = "取消",
  variant = "default",
  loading = false,
}: ConfirmDialogProps) {
  return (
    <Modal open={open} onClose={onClose} title={title}>
      <div className="space-y-5">
        <div className="flex items-start gap-3">
          {variant === "danger" && (
            <div
              className="flex items-center justify-center w-9 h-9 rounded-full shrink-0 mt-0.5"
              style={{ backgroundColor: "var(--color-err-bg)", color: "var(--color-err)" }}
            >
              <AlertTriangle size={18} />
            </div>
          )}
          <p className="text-sm leading-relaxed" style={{ color: "var(--text-secondary)" }}>
            {message}
          </p>
        </div>
        <div className="flex justify-end gap-3 pt-1">
          <Button variant="secondary" onClick={onClose} disabled={loading}>
            {cancelText}
          </Button>
          <Button
            variant={variant === "danger" ? "ghost" : undefined}
            onClick={onConfirm}
            disabled={loading}
            className={variant === "danger" ? "text-[var(--color-err)] hover:bg-[var(--color-err-bg)] hover:text-[var(--color-err)]" : ""}
          >
            {loading ? "处理中..." : confirmText}
          </Button>
        </div>
      </div>
    </Modal>
  );
}
