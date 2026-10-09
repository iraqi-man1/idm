import { AppWindow, FileArchive, FileImage, FileMusic, FileText, FileVideo, File } from "lucide-react";
import type { Category } from "@/bindings/Category";
import { cn } from "@/lib/utils";

const MAP: Record<Category, { icon: typeof File; cls: string }> = {
  video: { icon: FileVideo, cls: "text-violet-500 bg-violet-500/10" },
  music: { icon: FileMusic, cls: "text-pink-500 bg-pink-500/10" },
  document: { icon: FileText, cls: "text-sky-600 bg-sky-500/10" },
  archive: { icon: FileArchive, cls: "text-amber-600 bg-amber-500/10" },
  program: { icon: AppWindow, cls: "text-emerald-600 bg-emerald-500/10" },
  image: { icon: FileImage, cls: "text-teal-600 bg-teal-500/10" },
  other: { icon: File, cls: "text-muted-foreground bg-muted" },
};

export function FileIcon({ category, className }: { category: Category; className?: string }) {
  const { icon: Icon, cls } = MAP[category];
  return (
    <span className={cn("inline-flex size-7 shrink-0 items-center justify-center rounded-md", cls, className)}>
      <Icon className="size-4" />
    </span>
  );
}

export function categoryIcon(category: Category) {
  return MAP[category].icon;
}
