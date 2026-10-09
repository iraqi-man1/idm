import { useEffect, useRef } from "react";
import { formatSpeed } from "@/lib/format";

function cssVar(name: string): string {
  return getComputedStyle(document.documentElement).getPropertyValue(name).trim() || "#2563eb";
}

/** Area chart of speed samples (one per second, oldest first). */
export function SpeedGraph({ samples, height = 110, capacity = 120 }: { samples: number[]; height?: number; capacity?: number }) {
  const ref = useRef<HTMLCanvasElement>(null);
  const wrap = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const canvas = ref.current;
    const container = wrap.current;
    if (!canvas || !container) return;
    const draw = () => {
      const dpr = window.devicePixelRatio || 1;
      const w = container.clientWidth;
      const h = height;
      canvas.width = Math.max(1, Math.floor(w * dpr));
      canvas.height = Math.floor(h * dpr);
      canvas.style.width = `${w}px`;
      canvas.style.height = `${h}px`;
      const ctx = canvas.getContext("2d");
      if (!ctx) return;
      ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
      ctx.clearRect(0, 0, w, h);

      const data = samples.slice(-capacity);
      const max = Math.max(1024, ...data) * 1.15;
      const border = cssVar("--border");
      const primary = cssVar("--primary");
      const muted = cssVar("--muted-foreground");

      // grid
      ctx.strokeStyle = border;
      ctx.lineWidth = 1;
      for (let i = 1; i < 4; i++) {
        const y = Math.round((h * i) / 4) + 0.5;
        ctx.beginPath();
        ctx.moveTo(0, y);
        ctx.lineTo(w, y);
        ctx.stroke();
      }
      if (data.length >= 2) {
        const step = w / (capacity - 1);
        const x0 = w - (data.length - 1) * step;
        const pts = data.map((v, i) => [x0 + i * step, h - (v / max) * (h - 6)] as const);
        const grad = ctx.createLinearGradient(0, 0, 0, h);
        grad.addColorStop(0, primary + "55");
        grad.addColorStop(1, primary + "05");
        ctx.beginPath();
        ctx.moveTo(pts[0][0], h);
        for (const [x, y] of pts) ctx.lineTo(x, y);
        ctx.lineTo(pts[pts.length - 1][0], h);
        ctx.closePath();
        ctx.fillStyle = grad;
        ctx.fill();
        ctx.beginPath();
        pts.forEach(([x, y], i) => (i ? ctx.lineTo(x, y) : ctx.moveTo(x, y)));
        ctx.strokeStyle = primary;
        ctx.lineWidth = 1.75;
        ctx.lineJoin = "round";
        ctx.stroke();
      }
      ctx.fillStyle = muted;
      ctx.font = "11px system-ui, sans-serif";
      ctx.fillText(formatSpeed(max / 1.15), 6, 13);
    };
    draw();
    const ro = new ResizeObserver(draw);
    ro.observe(container);
    return () => ro.disconnect();
  }, [samples, height, capacity]);

  return (
    <div ref={wrap} className="w-full" dir="ltr">
      <canvas ref={ref} className="block rounded-md" />
    </div>
  );
}
