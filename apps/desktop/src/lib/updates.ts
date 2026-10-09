/** First automatic update check after start (lets the window settle). */
export const AUTO_CHECK_DELAY_MS = 15_000;
/** Minimum time between automatic update checks. */
export const AUTO_CHECK_INTERVAL_MS = 24 * 60 * 60 * 1000;

/**
 * Whether the automatic update check should run now: only when the build
 * has an updater signing key, the user left "Check for updates
 * automatically" on, and the last check is at least a day old.
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
