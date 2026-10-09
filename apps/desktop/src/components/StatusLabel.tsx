import { AlertCircle, CheckCircle2, Clock, Loader2, Pause, RotateCw, Square, Download, Cog } from "lucide-react";
import { useTranslation } from "react-i18next";
import type { DownloadStatus } from "@/bindings/DownloadStatus";
import type { ErrorKind } from "@/bindings/ErrorKind";
import { cn } from "@/lib/utils";

const STYLE: Record<DownloadStatus, { icon: typeof Clock; cls: string; spin?: boolean }> = {
  queued: { icon: Clock, cls: "text-muted-foreground" },
  connecting: { icon: Loader2, cls: "text-primary", spin: true },
  downloading: { icon: Download, cls: "text-primary" },
  paused: { icon: Pause, cls: "text-warning" },
  retrying: { icon: RotateCw, cls: "text-warning" },
  processing: { icon: Cog, cls: "text-info", spin: true },
  completed: { icon: CheckCircle2, cls: "text-success" },
  failed: { icon: AlertCircle, cls: "text-danger" },
  cancelled: { icon: Square, cls: "text-muted-foreground" },
};

export function StatusLabel({
  status,
  errorKind,
  scheduled,
  className,
}: {
  status: DownloadStatus;
  errorKind?: ErrorKind | null;
  scheduled?: boolean;
  className?: string;
}) {
  const { t } = useTranslation();
  const s = STYLE[status];
  const Icon = s.icon;
  const text =
    status === "failed" && errorKind
      ? t(`errors.${errorKind}`)
      : status === "queued" && scheduled
        ? t("status.scheduled")
        : t(`status.${status}`);
  return (
    <span className={cn("inline-flex min-w-0 items-center gap-1.5", s.cls, className)}>
      <Icon className={cn("size-3.5 shrink-0", s.spin && "animate-spin")} />
      <span className="truncate">{text}</span>
    </span>
  );
}

export function statusTone(status: DownloadStatus): "primary" | "success" | "warning" | "danger" | "muted" {
  switch (status) {
    case "completed":
      return "success";
    case "failed":
      return "danger";
    case "paused":
    case "retrying":
      return "warning";
    case "cancelled":
    case "queued":
      return "muted";
    default:
      return "primary";
  }
}
