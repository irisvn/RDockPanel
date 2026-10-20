// What a status reads (docs/ui-guidelines.md §4): four tones shared by every
// kind of thing, and the word for each kind, so a finished deployment never
// reads "Running". Words are English sources translated by `t()`.

export type Tone = "ok" | "busy" | "bad" | "off";
export type StatusKind = "service" | "deployment" | "action" | "container" | "health";

type Entry = [Tone, string];

const SERVICE: Record<string, Entry> = {
  running: ["ok", "Running"],
  deploying: ["busy", "Deploying"],
  queued: ["busy", "Deploying"],
  running_deployment: ["busy", "Deploying"],
  starting: ["busy", "Starting"],
  failed: ["bad", "Failed"],
  stopped: ["off", "Stopped"],
  new: ["off", "Not deployed"],
};

const DEPLOYMENT: Record<string, Entry> = {
  queued: ["busy", "Queued"],
  running: ["busy", "Running"],
  succeeded: ["ok", "Succeeded"],
  failed: ["bad", "Failed"],
  interrupted: ["bad", "Interrupted"],
  rolled_back: ["off", "Rolled back"],
  cancelled: ["off", "Cancelled"],
};

const ACTION: Record<string, Entry> = {
  queued: ["busy", "Queued"],
  running: ["busy", "Running"],
  ok: ["ok", "Done"],
  succeeded: ["ok", "Done"],
  done: ["ok", "Done"],
  failed: ["bad", "Failed"],
  interrupted: ["bad", "Interrupted"],
};

const CONTAINER: Record<string, Entry> = {
  running: ["ok", "Running"],
  restarting: ["busy", "Restarting"],
  removing: ["busy", "Removing"],
  dead: ["bad", "Dead"],
  exited: ["off", "Exited"],
  paused: ["off", "Paused"],
  created: ["off", "Created"],
};

const HEALTH: Record<string, Entry> = {
  connected: ["ok", "Connected"],
  healthy: ["ok", "Healthy"],
  running: ["ok", "Running"],
  starting: ["busy", "Starting"],
  unhealthy: ["bad", "Unhealthy"],
  "not-connected": ["bad", "Not connected"],
  failed: ["bad", "Failed"],
  off: ["off", "Off"],
  stopped: ["off", "Stopped"],
};

const KINDS: Record<StatusKind, Record<string, Entry>> = {
  service: SERVICE,
  deployment: DEPLOYMENT,
  action: ACTION,
  container: CONTAINER,
  health: HEALTH,
};

/** The tone and the (untranslated) word of `status` for a thing of `kind`;
 * an unknown status reads as itself, in the off tone. */
export function statusOf(kind: StatusKind, status: string | null | undefined): Entry {
  const key = (status ?? "").toLowerCase();
  return KINDS[kind][key] ?? ["off", key ? key.charAt(0).toUpperCase() + key.slice(1).replace(/_/g, " ") : "Unknown"];
}
