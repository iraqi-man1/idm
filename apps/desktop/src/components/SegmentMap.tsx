import type { SegmentProgress } from "@/bindings/SegmentProgress";
import { cn } from "@/lib/utils";

/**
 * Visual map of the file: each segment's downloaded part is drawn at its
 * real position. Data comes straight from the engine's segment table.
 */
export function SegmentMap({ total, segments, className }: { total: number; segments: SegmentProgress[]; className?: string }) {
  if (!total) return null;
  return (
    <div className={cn("relative h-5 w-full overflow-hidden rounded-md border border-border bg-muted", className)} dir="ltr">
      {segments.map((s) => {
        const left = (s.start / total) * 100;
        const width = (s.written / total) * 100;
        const end = s.end ?? total;
        return (
          <div key={`${s.index}-${s.start}`}>
            <div
              className={cn("absolute inset-y-0", s.done ? "bg-primary/85" : "bg-primary/70", s.active && !s.done && "progress-stripes")}
              style={{ left: `${left}%`, width: `${Math.max(width, s.written > 0 ? 0.15 : 0)}%` }}
            />
            {/* boundary marker */}
            <div className="absolute inset-y-0 w-px bg-surface/70" style={{ left: `${(end / total) * 100}%` }} />
          </div>
        );
      })}
    </div>
  );
}
