import { useEffect, useState } from "react";
import { Input } from "@/components/ui/input";
import { cn } from "@/lib/utils";

/** Text input that saves on blur / Enter. */
export function LazyInput({
  value,
  onCommit,
  className,
  placeholder,
  dir = "ltr",
  type = "text",
}: {
  value: string;
  onCommit: (v: string) => void;
  className?: string;
  placeholder?: string;
  dir?: string;
  type?: string;
}) {
  const [v, setV] = useState(value);
  useEffect(() => setV(value), [value]);
  return (
    <Input
      value={v}
      type={type}
      dir={dir}
      placeholder={placeholder}
      className={className}
      onChange={(e) => setV(e.target.value)}
      onBlur={() => v !== value && onCommit(v)}
      onKeyDown={(e) => e.key === "Enter" && (e.target as HTMLInputElement).blur()}
    />
  );
}

export function NumberInput({ value, min, max, onCommit, className }: { value: number; min: number; max: number; onCommit: (v: number) => void; className?: string }) {
  return (
    <LazyInput
      className={cn("w-24 text-end tabular", className)}
      value={String(value)}
      onCommit={(v) => {
        const n = Math.round(Number(v));
        if (Number.isFinite(n)) onCommit(Math.min(max, Math.max(min, n)));
      }}
    />
  );
}
