import { cn } from "@/lib/utils";

/** A labelled settings/form row: label + hint on one side, control on the other. */
export function Field({
  label,
  hint,
  children,
  className,
  stacked = false,
  htmlFor,
}: {
  label: React.ReactNode;
  hint?: React.ReactNode;
  children: React.ReactNode;
  className?: string;
  stacked?: boolean;
  htmlFor?: string;
}) {
  return (
    <div className={cn("flex gap-4 py-3", stacked ? "flex-col gap-2" : "items-center justify-between", className)}>
      <div className="min-w-0 flex-1">
        <label htmlFor={htmlFor} className="block text-[13px] font-medium">
          {label}
        </label>
        {hint && <p className="mt-0.5 text-xs text-muted-foreground">{hint}</p>}
      </div>
      <div className={cn(stacked ? "w-full" : "shrink-0")}>{children}</div>
    </div>
  );
}

export function Section({ title, children, className }: { title?: React.ReactNode; children: React.ReactNode; className?: string }) {
  return (
    <section className={cn("rounded-xl border border-border bg-surface px-4 shadow-card", className)}>
      {title && <h3 className="border-b border-border py-2.5 text-[13px] font-semibold">{title}</h3>}
      <div className="divide-y divide-border">{children}</div>
    </section>
  );
}
