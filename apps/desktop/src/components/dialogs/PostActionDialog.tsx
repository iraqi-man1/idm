import { Power } from "lucide-react";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { api, events, type PostActionNotice } from "@/lib/api";

/** Countdown before the action that follows a finished queue (shut down, sleep...). */
export function PostActionDialog() {
  const { t } = useTranslation();
  const [notice, setNotice] = useState<PostActionNotice | null>(null);
  const [left, setLeft] = useState(0);

  useEffect(() => {
    const subs = [
      events.postAction((n) => {
        setNotice(n);
        setLeft(n.seconds);
      }),
      events.postActionCancelled(() => setNotice(null)),
      events.postActionFailed((error) => {
        setNotice(null);
        toast.error(t("postAction.failed"), { description: error });
      }),
    ];
    return () => subs.forEach((p) => void p.then((u) => u()));
  }, [t]);

  useEffect(() => {
    if (!notice) return;
    const timer = setInterval(() => setLeft((s) => Math.max(0, s - 1)), 1000);
    return () => clearInterval(timer);
  }, [notice]);

  const cancel = () => {
    setNotice(null);
    void api.cancelPostAction();
  };

  return (
    <Dialog open={notice !== null} onOpenChange={(o) => !o && cancel()}>
      <DialogContent className="w-[min(440px,calc(100vw-32px))]" data-testid="post-action-dialog">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            <Power className="size-5 text-warning" /> {t("postAction.title")}
          </DialogTitle>
          <DialogDescription>
            {notice && t(`postAction.body_${notice.action}`, { queue: notice.queue, seconds: left })}
          </DialogDescription>
        </DialogHeader>
        <div className="h-1.5 overflow-hidden rounded-full bg-muted">
          <div
            className="h-full bg-warning transition-[width] duration-1000 ease-linear"
            style={{ width: `${notice ? (left / notice.seconds) * 100 : 0}%` }}
          />
        </div>
        <DialogFooter>
          <Button variant="secondary" onClick={() => void api.runPostActionNow()}>
            {t("postAction.now")}
          </Button>
          <Button autoFocus onClick={cancel}>
            {t("common.cancel")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
