import { open } from "@tauri-apps/plugin-dialog";
import {
  Battery,
  Download,
  FolderOpen,
  Globe,
  Info,
  Network,
  Palette,
  RefreshCw,
  Settings2,
  Film,
} from "lucide-react";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import type { AppSettings } from "@/bindings/AppSettings";
import { Button } from "@/components/ui/button";
import { Field, Section } from "@/components/ui/field";
import { Input, Textarea } from "@/components/ui/input";
import { LazyInput, NumberInput } from "@/components/ui/lazy-input";
import { Select } from "@/components/ui/select";
import { Switch } from "@/components/ui/switch";
import { api, errorMessage } from "@/lib/api";
import { formatBytes, parseSize } from "@/lib/format";
import { cn } from "@/lib/utils";
import { BrowserSection } from "@/pages/settings/BrowserSection";
import { MediaTools } from "@/pages/settings/MediaTools";
import { UpdatesSection } from "@/pages/settings/UpdatesSection";
import { useSettings } from "@/stores/settings";
import { useUi } from "@/stores/ui";

type Updater = (s: AppSettings) => AppSettings;

function useSave() {
  const { t } = useTranslation();
  const save = useSettings((s) => s.save);
  return (fn: Updater) =>
    save(fn).catch((e) => {
      toast.error(t("common.error"), { description: errorMessage(e) });
    });
}

function FolderInput({ value, onCommit, placeholder }: { value: string; onCommit: (v: string) => void; placeholder?: string }) {
  const { t } = useTranslation();
  return (
    <div className="flex w-[420px] gap-2">
      <LazyInput value={value} onCommit={onCommit} placeholder={placeholder} />
      <Button
        variant="secondary"
        onClick={async () => {
          const d = await open({ directory: true, defaultPath: value || undefined });
          if (typeof d === "string") onCommit(d);
        }}
      >
        <FolderOpen /> {t("common.browse")}
      </Button>
    </div>
  );
}

function SpeedInput({ value, onCommit }: { value: number; onCommit: (v: number) => void }) {
  const { t } = useTranslation();
  return (
    <LazyInput
      className="w-36 text-end"
      value={value ? formatBytes(value) : ""}
      placeholder={t("add.unlimited")}
      onCommit={(v) => {
        if (!v.trim()) return onCommit(0);
        const n = parseSize(v);
        if (n === null) toast.error(t("settings.downloads.speedLimitHint"));
        else onCommit(n);
      }}
    />
  );
}

function General({ s }: { s: AppSettings }) {
  const { t } = useTranslation();
  const save = useSave();
  const [defaultDir, setDefaultDir] = useState("");
  useEffect(() => {
    void api.defaultDownloadDir().then(setDefaultDir);
  }, [s.general.download_dir]);
  const g = s.general;
  const sw = (key: keyof AppSettings["general"]) => (
    <Switch checked={g[key] as boolean} onCheckedChange={(v) => save((x) => ({ ...x, general: { ...x.general, [key]: v } }))} />
  );
  return (
    <div className="space-y-4">
      <Section>
        <Field label={t("settings.general.downloadDir")} hint={t("settings.general.downloadDirHint")} stacked>
          <FolderInput value={g.download_dir} placeholder={defaultDir} onCommit={(v) => save((x) => ({ ...x, general: { ...x.general, download_dir: v } }))} />
        </Field>
        <Field label={t("settings.general.categoryFolders")} hint={t("settings.general.categoryFoldersHint")}>
          {sw("category_folders")}
        </Field>
        <Field label={t("settings.general.conflict")}>
          <Select
            value={g.conflict_policy}
            onChange={(v) => save((x) => ({ ...x, general: { ...x.general, conflict_policy: v } }))}
            options={[
              { value: "rename", label: t("settings.general.conflictRename") },
              { value: "overwrite", label: t("settings.general.conflictOverwrite") },
            ]}
          />
        </Field>
        <Field label={t("settings.general.duplicate")}>
          <Select
            value={g.duplicate_policy}
            onChange={(v) => save((x) => ({ ...x, general: { ...x.general, duplicate_policy: v } }))}
            options={[
              { value: "ask", label: t("settings.general.duplicateAsk") },
              { value: "allow", label: t("settings.general.duplicateAllow") },
              { value: "skip", label: t("settings.general.duplicateSkip") },
            ]}
          />
        </Field>
      </Section>
      <Section>
        <Field label={t("settings.general.launchAtStartup")}>{sw("launch_at_startup")}</Field>
        <Field label={t("settings.general.startMinimized")}>{sw("start_minimized")}</Field>
        <Field label={t("settings.general.closeToTray")}>{sw("close_to_tray")}</Field>
        <Field label={t("settings.general.notifyComplete")}>{sw("notify_on_complete")}</Field>
        <Field label={t("settings.general.notifyError")}>{sw("notify_on_error")}</Field>
        <Field label={t("settings.general.progressWindow")}>{sw("show_progress_window")}</Field>
        <Field label={t("settings.general.browserDialog")}>{sw("show_add_dialog_for_browser")}</Field>
        <Field label={t("settings.general.clipboard")} hint={t("settings.general.clipboardHint")}>
          {sw("clipboard_monitor")}
        </Field>
      </Section>
    </div>
  );
}

function Downloads({ s }: { s: AppSettings }) {
  const { t } = useTranslation();
  const save = useSave();
  const d = s.downloads;
  const set = <K extends keyof AppSettings["downloads"]>(k: K, v: AppSettings["downloads"][K]) =>
    save((x) => ({ ...x, downloads: { ...x.downloads, [k]: v } }));
  return (
    <div className="space-y-4">
      <Section>
        <Field label={t("settings.downloads.maxConcurrent")}>
          <NumberInput value={d.max_concurrent} min={1} max={32} onCommit={(v) => set("max_concurrent", v)} />
        </Field>
        <Field label={t("settings.downloads.connections")} hint={t("settings.downloads.connectionsHint")}>
          <Select
            value={String(d.connections_per_download)}
            onChange={(v) => set("connections_per_download", Number(v))}
            className="min-w-20"
            options={["1", "2", "4", "8", "16", "24", "32"].map((c) => ({ value: c, label: c }))}
          />
        </Field>
        <Field label={t("settings.downloads.adaptive")}>
          <Switch checked={d.adaptive_connections} onCheckedChange={(v) => set("adaptive_connections", v)} />
        </Field>
        <Field label={t("settings.downloads.minSegment")}>
          <Select
            value={String(d.min_segment_size)}
            onChange={(v) => set("min_segment_size", Number(v))}
            options={[256, 512, 1024, 2048, 4096, 8192].map((k) => ({ value: String(k * 1024), label: formatBytes(k * 1024) }))}
          />
        </Field>
        <Field label={t("settings.downloads.speedLimit")} hint={t("settings.downloads.speedLimitHint")}>
          <SpeedInput value={d.speed_limit} onCommit={(v) => set("speed_limit", v)} />
        </Field>
      </Section>
      <Section>
        <Field label={t("settings.downloads.retries")}>
          <NumberInput value={d.max_retries} min={0} max={1000} onCommit={(v) => set("max_retries", v)} />
        </Field>
        <Field label={t("settings.downloads.retryDelay")}>
          <NumberInput value={d.retry_delay_secs} min={1} max={600} onCommit={(v) => set("retry_delay_secs", v)} />
        </Field>
        <Field label={t("settings.downloads.resumeOnStartup")}>
          <Switch checked={d.resume_on_startup} onCheckedChange={(v) => set("resume_on_startup", v)} />
        </Field>
        <Field label={t("settings.downloads.preallocate")}>
          <Switch checked={d.preallocate} onCheckedChange={(v) => set("preallocate", v)} />
        </Field>
        <Field label={t("settings.downloads.tempDir")} hint={t("settings.downloads.tempDirHint")} stacked>
          <FolderInput value={d.temp_dir} onCommit={(v) => set("temp_dir", v)} />
        </Field>
      </Section>
    </div>
  );
}

function NetworkSettings({ s }: { s: AppSettings }) {
  const { t } = useTranslation();
  const save = useSave();
  const info = useSettings((x) => x.info);
  const replace = useSettings((x) => x.replace);
  const n = s.network;
  const p = n.proxy;
  const [pw, setPw] = useState("");
  const setN = <K extends keyof AppSettings["network"]>(k: K, v: AppSettings["network"][K]) =>
    save((x) => ({ ...x, network: { ...x.network, [k]: v } }));
  const setP = <K extends keyof AppSettings["network"]["proxy"]>(k: K, v: AppSettings["network"]["proxy"][K]) =>
    save((x) => ({ ...x, network: { ...x.network, proxy: { ...x.network.proxy, [k]: v } } }));
  const manual = p.mode === "http" || p.mode === "socks5";
  const savePassword = async (value: string | null) => {
    try {
      replace(await api.setProxyPassword(value));
      setPw("");
      toast.success(t("common.saved"));
    } catch (e) {
      toast.error(t("common.error"), { description: errorMessage(e) });
    }
  };
  return (
    <div className="space-y-4">
      <Section title={t("settings.network.proxy")}>
        <Field label={t("settings.network.proxy")}>
          <Select
            value={p.mode}
            onChange={(v) => setP("mode", v)}
            options={[
              { value: "system", label: t("settings.network.proxySystem") },
              { value: "none", label: t("settings.network.proxyNone") },
              { value: "http", label: t("settings.network.proxyHttp") },
              { value: "socks5", label: t("settings.network.proxySocks") },
            ]}
          />
        </Field>
        {manual && (
          <>
            <Field label={t("settings.network.host")}>
              <div className="flex gap-2">
                <LazyInput className="w-56" value={p.host} onCommit={(v) => setP("host", v.trim())} placeholder="proxy.example.com" />
                <NumberInput value={p.port} min={1} max={65535} onCommit={(v) => setP("port", v)} />
              </div>
            </Field>
            <Field label={t("settings.network.username")}>
              <LazyInput className="w-56" value={p.username} onCommit={(v) => setP("username", v)} />
            </Field>
            <Field label={t("settings.network.password")} hint={p.has_password ? t("settings.network.passwordStored") : undefined}>
              <div className="flex gap-2">
                <Input type="password" className="w-44" value={pw} autoComplete="off" onChange={(e) => setPw(e.target.value)} />
                <Button variant="secondary" disabled={!pw} onClick={() => savePassword(pw)}>
                  {t("settings.network.setPassword")}
                </Button>
                {p.has_password && (
                  <Button variant="ghost" onClick={() => savePassword(null)}>
                    {t("settings.network.clearPassword")}
                  </Button>
                )}
              </div>
            </Field>
            <Field label={t("settings.network.bypass")}>
              <LazyInput className="w-72" value={p.bypass} onCommit={(v) => setP("bypass", v)} />
            </Field>
          </>
        )}
        {info && !info.secure_storage && <p className="py-3 text-xs text-warning">{t("settings.network.noSecureStorage")}</p>}
      </Section>
      <Section>
        <Field label={t("settings.network.connectTimeout")}>
          <NumberInput value={n.connect_timeout_secs} min={3} max={300} onCommit={(v) => setN("connect_timeout_secs", v)} />
        </Field>
        <Field label={t("settings.network.readTimeout")}>
          <NumberInput value={n.read_timeout_secs} min={5} max={600} onCommit={(v) => setN("read_timeout_secs", v)} />
        </Field>
        <Field label={t("settings.network.redirects")}>
          <NumberInput value={n.max_redirects} min={0} max={30} onCommit={(v) => setN("max_redirects", v)} />
        </Field>
        <Field label={t("settings.network.userAgent")} stacked>
          <LazyInput className="font-mono text-xs" value={n.user_agent} onCommit={(v) => setN("user_agent", v)} />
        </Field>
      </Section>
    </div>
  );
}

function MediaSettings({ s }: { s: AppSettings }) {
  const { t } = useTranslation();
  const save = useSave();
  const m = s.media;
  const set = <K extends keyof AppSettings["media"]>(k: K, v: AppSettings["media"][K]) =>
    save((x) => ({ ...x, media: { ...x.media, [k]: v } }));
  return (
    <div className="space-y-4">
      <Section>
        <Field label={t("settings.media.quality")}>
          <Select
            value={m.preferred_quality}
            onChange={(v) => set("preferred_quality", v)}
            options={[
              { value: "best", label: t("settings.media.best") },
              { value: "p2160", label: "2160p (4K)" },
              { value: "p1440", label: "1440p" },
              { value: "p1080", label: "1080p" },
              { value: "p720", label: "720p" },
              { value: "p480", label: "480p" },
              { value: "p360", label: "360p" },
              { value: "audio_only", label: t("settings.media.audioOnly") },
            ]}
          />
        </Field>
        <Field label={t("settings.media.container")}>
          <Select
            value={m.output_container}
            onChange={(v) => set("output_container", v)}
            options={[
              { value: "mp4", label: "MP4" },
              { value: "mkv", label: "MKV" },
              { value: "original", label: t("settings.media.original") },
              { value: "m4a", label: "M4A" },
              { value: "mp3", label: "MP3" },
            ]}
          />
        </Field>
        <Field label={t("settings.media.subtitles")}>
          <Switch checked={m.download_subtitles} onCheckedChange={(v) => set("download_subtitles", v)} />
        </Field>
        <Field label={t("settings.media.subtitleLangs")} hint={t("settings.media.subtitleLangsHint")}>
          <LazyInput
            className="w-48"
            value={m.subtitle_languages.join(", ")}
            onCommit={(v) =>
              set(
                "subtitle_languages",
                v.split(/[\s,]+/).map((x) => x.trim()).filter(Boolean),
              )
            }
          />
        </Field>
        <Field label={t("settings.media.embedSubtitles")}>
          <Switch checked={m.embed_subtitles} onCheckedChange={(v) => set("embed_subtitles", v)} />
        </Field>
        <Field label={t("settings.media.segmentConcurrency")}>
          <NumberInput value={m.segment_concurrency} min={1} max={16} onCommit={(v) => set("segment_concurrency", v)} />
        </Field>
      </Section>
      <MediaTools />
    </div>
  );
}

function Appearance({ s }: { s: AppSettings }) {
  const { t } = useTranslation();
  const save = useSave();
  const a = s.appearance;
  return (
    <Section>
      <Field label={t("settings.appearance.theme")}>
        <div className="inline-flex rounded-md border border-border-strong p-0.5">
          {(["system", "light", "dark"] as const).map((m) => (
            <button
              key={m}
              onClick={() => save((x) => ({ ...x, appearance: { ...x.appearance, theme: m } }))}
              className={cn("rounded px-3 py-1 text-[13px]", a.theme === m ? "bg-primary text-primary-foreground" : "hover:bg-accent")}
            >
              {t(`settings.appearance.${m}`)}
            </button>
          ))}
        </div>
      </Field>
      <Field label={t("settings.appearance.language")}>
        <Select
          value={a.language}
          onChange={(v) => save((x) => ({ ...x, appearance: { ...x.appearance, language: v } }))}
          options={[
            { value: "system", label: t("settings.appearance.languageSystem") },
            { value: "en", label: t("settings.appearance.english") },
            { value: "ar", label: t("settings.appearance.arabic") },
          ]}
        />
      </Field>
    </Section>
  );
}

function Power({ s }: { s: AppSettings }) {
  const { t } = useTranslation();
  const save = useSave();
  const p = s.power;
  const set = <K extends keyof AppSettings["power"]>(k: K, v: AppSettings["power"][K]) =>
    save((x) => ({ ...x, power: { ...x.power, [k]: v } }));
  return (
    <Section>
      <Field label={t("settings.power.battery")}>
        <Switch checked={p.pause_on_low_battery} onCheckedChange={(v) => set("pause_on_low_battery", v)} />
      </Field>
      <Field label={t("settings.power.threshold")}>
        <NumberInput value={p.battery_threshold} min={5} max={95} onCommit={(v) => set("battery_threshold", v)} />
      </Field>
      <Field label={t("settings.power.metered")}>
        <Switch checked={p.pause_on_metered} onCheckedChange={(v) => set("pause_on_metered", v)} />
      </Field>
    </Section>
  );
}

function About() {
  const { t } = useTranslation();
  const info = useSettings((s) => s.info);
  if (!info) return null;
  return (
    <div className="space-y-4">
      <Section>
        <div className="flex items-center gap-4 py-4">
          <img src="/app-icon.svg" alt="" className="size-14" />
          <div>
            <p className="text-base font-semibold">{t("app.name")}</p>
            <p className="text-[13px] text-muted-foreground">
              {t("settings.about.version")} {info.version} · {info.os} {info.arch}
            </p>
          </div>
        </div>
        <Field label={t("settings.about.dataDir")}>
          <span className="font-mono text-xs text-muted-foreground" dir="ltr" data-selectable>
            {info.data_dir}
          </span>
        </Field>
        <Field label={t("settings.about.logDir")}>
          <span className="font-mono text-xs text-muted-foreground" dir="ltr" data-selectable>
            {info.log_dir}
          </span>
        </Field>
        <Field label={t("settings.about.secureStorage")}>
          <span className={info.secure_storage ? "text-success" : "text-warning"}>
            {info.secure_storage ? t("settings.about.available") : t("settings.about.unavailable")}
          </span>
        </Field>
      </Section>
      <p className="px-1 text-xs text-muted-foreground">{t("settings.about.notices")}</p>
      <p className="px-1 text-xs text-muted-foreground">{t("settings.about.licenses")}</p>
    </div>
  );
}

const SECTIONS = [
  { id: "general", icon: Settings2 },
  { id: "downloads", icon: Download },
  { id: "network", icon: Network },
  { id: "browser", icon: Globe },
  { id: "media", icon: Film },
  { id: "appearance", icon: Palette },
  { id: "power", icon: Battery },
  { id: "updates", icon: RefreshCw },
  { id: "about", icon: Info },
] as const;

export function SettingsPage() {
  const { t } = useTranslation();
  const s = useSettings((x) => x.settings);
  const section = useUi((x) => x.settingsSection);
  const setPage = useUi((x) => x.setPage);
  if (!s) return <div className="p-6 text-muted-foreground">{t("common.loading")}</div>;
  return (
    <div className="flex min-h-0 flex-1">
      <nav className="w-52 shrink-0 space-y-0.5 border-e border-border bg-surface p-3">
        <h2 className="px-2 pb-2 text-[15px] font-semibold">{t("settings.title")}</h2>
        {SECTIONS.map(({ id, icon: Icon }) => (
          <button
            key={id}
            onClick={() => setPage("settings", id)}
            className={cn(
              "flex h-8 w-full items-center gap-2.5 rounded-md px-2.5 text-[13px]",
              section === id ? "bg-primary-soft font-medium text-primary" : "text-muted-foreground hover:bg-accent hover:text-foreground",
            )}
          >
            <Icon className="size-4" />
            {t(`settings.sections.${id}`)}
          </button>
        ))}
      </nav>
      <div className="min-w-0 flex-1 overflow-y-auto">
        <div className="mx-auto max-w-3xl p-6">
          <h1 className="mb-4 text-lg font-semibold">{t(`settings.sections.${section}`)}</h1>
          {section === "general" && <General s={s} />}
          {section === "downloads" && <Downloads s={s} />}
          {section === "network" && <NetworkSettings s={s} />}
          {section === "browser" && <BrowserSection s={s} />}
          {section === "media" && <MediaSettings s={s} />}
          {section === "appearance" && <Appearance s={s} />}
          {section === "power" && <Power s={s} />}
          {section === "updates" && <UpdatesSection s={s} />}
          {section === "about" && <About />}
        </div>
      </div>
    </div>
  );
}

export { LazyInput, NumberInput, useSave, Textarea };
