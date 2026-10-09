import { cn } from "@/lib/utils";

/** Determinate or indeterminate bar. `tone` reflects the download state. */
export function ProgressBar({
  value,
  tone = "primary",
  animated = false,
  className,
}: {
  value: number | null;
  tone?: "primary" | "success" | "warning" | "danger" | "muted";
  animated?: boolean;
  className?: string;
}) {
  const color = {
    primary: "bg-primary",
    success: "bg-success",
    warning: "bg-warning",
    danger: "bg-danger",
    muted: "bg-muted-foreground/50",
  }[tone];
  const pct = value === null ? 100 : Math.max(0, Math.min(100, value * 100));
  return (
    <div
      className={cn("relative h-1.5 w-full overflow-hidden rounded-full bg-muted", className)}
      role="progressbar"
      aria-valuemin={0}
      aria-valuemax={100}
      aria-valuenow={value === null ? undefined : Math.round(pct)}
    >
      <div
        className={cn("h-full rounded-full transition-[width] duration-300 ease-out", color, (animated || value === null) && "progress-stripes", value === null && "opacity-60")}
        style={{ width: `${pct}%` }}
      />
    </div>
  );
}
