import { cn } from "@/lib/utils";

export function Badge({
  tone = "muted",
  className,
  children,
}: {
  tone?: "muted" | "primary" | "success" | "warning" | "danger";
  className?: string;
  children: React.ReactNode;
}) {
  const cls = {
    muted: "bg-muted text-muted-foreground",
    primary: "bg-primary-soft text-primary",
    success: "bg-success-soft text-success",
    warning: "bg-warning-soft text-warning",
    danger: "bg-danger-soft text-danger",
  }[tone];
  return (
    <span className={cn("inline-flex items-center gap-1 rounded-full px-2 py-0.5 text-[11px] font-medium leading-4", cls, className)}>
      {children}
    </span>
  );
}
