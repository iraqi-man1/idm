/** First automatic update check after start (lets the window settle). */
export const AUTO_CHECK_DELAY_MS = 15_000;
/** Minimum time between successful automatic update checks. */
export const AUTO_CHECK_INTERVAL_MS = 24 * 60 * 60 * 1000;
/** How often the checker wakes up (a failed check is retried then). */
export const AUTO_CHECK_WAKE_MS = 60 * 60 * 1000;

/**
 * Whether the automatic update check should run now: only when the build
 * has an updater signing key, the user left "Check for updates
 * automatically" on, and the last successful check is at least a day old.
 */
export function autoCheckDue(opts: {
  configured: boolean;
  enabled: boolean;
  lastCheck: number | null;
  now: number;
}): boolean {
  if (!opts.configured || !opts.enabled) return false;
  return opts.lastCheck === null || opts.now - opts.lastCheck >= AUTO_CHECK_INTERVAL_MS;
}

export interface FoundUpdate {
  version: string;
}

/**
 * The automatic update checker: call the returned function on every wake-up.
 * It checks when due, never runs two checks at once, announces each new
 * version once, and does not count a failed check (e.g. offline right after
 * login) as the day's check.
 */
export function createAutoChecker(deps: {
  check: () => Promise<FoundUpdate | null>;
  notify: (update: FoundUpdate) => void;
  onError: (error: unknown) => void;
  now: () => number;
}) {
  let lastCheck: number | null = null;
  let announced: string | null = null;
  let running = false;
  return async (opts: { configured: boolean; enabled: boolean }) => {
    if (running || !autoCheckDue({ ...opts, lastCheck, now: deps.now() })) return;
    running = true;
    const started = deps.now();
    try {
      const update = await deps.check();
      lastCheck = started;
      if (update && update.version !== announced) {
        announced = update.version;
        deps.notify(update);
      }
    } catch (e) {
      deps.onError(e);
    } finally {
      running = false;
    }
  };
}
