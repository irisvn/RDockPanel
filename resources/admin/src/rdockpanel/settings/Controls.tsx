// Settings → Panel's controls, like EasyPanel's: Panel ▾ (its logs, a
// restart, whether a newer image is out), Server ▾ (its public address found
// again, a reboot), Traefik ▾ (its logs, a restart), Docker ▾ (prunes). The
// panel's administrators' (`system.manage`). An update is the installer's
// command, run on the server: the panel never replaces itself.
import { useState } from "react";
import { type PageProps, useAdminQuery } from "@rustcms/admin-sdk";
import { Alert, Button, CodeBlock, Dialog, DropdownMenu, DropdownMenuItem, DropdownMenuSeparator, Input } from "@rustcms/admin-sdk/ui";
import { ErrorNote, Facts, bytes } from "../lib/ui";
import { Logs } from "./Traefik";

type Status = {
  version: string;
  versions: { rdockpanel: string; commit: string | null; rustcms: string };
  installed: boolean;
  image: string | null;
  hostname: string;
  public_ip: string | null;
  service: boolean;
  can_reboot: boolean;
};

type Update = {
  installed: boolean;
  image?: string | null;
  running?: string | null;
  latest?: string | null;
  update?: boolean | null;
  command?: string;
};

type Report = { containers_deleted: number; images_deleted: number; volumes_deleted?: number; build_caches_deleted?: number; space_reclaimed: number };

const PRUNES: Array<{ kind: string; label: string; question: string }> = [
  { kind: "containers", label: "Prune stopped containers", question: "Remove every stopped container (not the panel's)?" },
  { kind: "images", label: "Prune unused images", question: "Remove every image no container uses (tagged ones too, not the panel's)? They are pulled again when needed." },
  { kind: "build_cache", label: "Prune build cache", question: "Remove the builder's cache? The next builds start from scratch." },
  {
    kind: "volumes",
    label: "Prune unused volumes",
    question: "Remove volumes no container uses? The data of the panel's services (rdp_…), Compose projects and the panel itself are always kept.",
  },
];

/** Waits for the panel to answer again, then reloads the page. */
async function reloadWhenBack(base: string) {
  const deadline = Date.now() + 180_000;
  // It stops first: give it time to go down.
  await new Promise((r) => setTimeout(r, 4000));
  while (Date.now() < deadline) {
    try {
      const r = await fetch("/health", { cache: "no-store" });
      if (r.ok) {
        window.location.assign(base);
        return;
      }
    } catch {
      // Not back yet.
    }
    await new Promise((r) => setTimeout(r, 2000));
  }
}

export function Controls(props: PageProps<unknown>) {
  const t = props.t;
  const status = useAdminQuery<Status>(props, "/apps/system/status");
  const [logs, setLogs] = useState<"panel" | "traefik" | null>(null);
  const [update, setUpdate] = useState<Update | null>(null);
  const [rebooting, setRebooting] = useState(false);
  const [waiting, setWaiting] = useState<string | null>(null);
  const [error, setError] = useState<unknown>(null);
  const s = status.data;
  const call = async <T,>(run: () => Promise<T>): Promise<T | undefined> => {
    setError(null);
    try {
      return await run();
    } catch (err) {
      setError(err);
      return undefined;
    }
  };
  const restartPanel = async () => {
    if (!window.confirm(t("Restart the panel? It is unavailable for a few seconds; the page reloads when it is back."))) return;
    if (await call(() => props.api.post("/apps/system/restart", {}))) {
      setWaiting(t("The panel restarts…"));
      void reloadWhenBack(window.location.href);
    }
  };
  const restartTraefik = async () => {
    if (!window.confirm(t("Restart Traefik? Domains stop answering for a few seconds."))) return;
    if (await call(() => props.api.post("/apps/traefik/restart", {}))) props.toast.success(t("Traefik restarts."));
  };
  const refreshIp = async () => {
    const out = await call(() => props.api.post<{ public_ip: string; previous: string; changed: boolean }>("/apps/system/ip", {}));
    if (!out) return;
    props.toast.success(out.changed ? `${t("Public address")}: ${out.public_ip} (${t("was")} ${out.previous || t("unknown")})` : `${t("Public address unchanged")}: ${out.public_ip}`);
    status.refetch();
  };
  const prune = async (kind: string, question: string) => {
    if (!window.confirm(t(question))) return;
    const out = await call(() => props.api.post<{ report: Report }>(`/apps/system/prune/${kind}`, {}));
    if (!out) return;
    const r = out.report;
    const n = r.containers_deleted + r.images_deleted + (r.volumes_deleted ?? 0) + (r.build_caches_deleted ?? 0);
    props.toast.success(`${t("Removed")}: ${n} · ${t("space freed")}: ${bytes(r.space_reclaimed)}`);
  };
  return (
    <div className="rdp2-form">
      <div className="rdp2-actions">
        <DropdownMenu variant="outline" trigger={<>RDockPanel ▾</>}>
          <DropdownMenuItem onSelect={() => setLogs("panel")}>{t("Logs")}</DropdownMenuItem>
          <DropdownMenuItem disabled={!s?.service} onSelect={restartPanel}>
            {t("Restart")}
          </DropdownMenuItem>
          <DropdownMenuItem onSelect={async () => setUpdate((await call(() => props.api.post<Update>("/apps/system/update", {}))) ?? null)}>
            {t("Check for updates")}
          </DropdownMenuItem>
        </DropdownMenu>
        <DropdownMenu variant="outline" trigger={<>{t("Server")} ▾</>}>
          <DropdownMenuItem onSelect={refreshIp}>{t("Refresh IP")}</DropdownMenuItem>
          <DropdownMenuSeparator />
          <DropdownMenuItem destructive disabled={!s?.can_reboot} onSelect={() => setRebooting(true)}>
            {t("Reboot")}
          </DropdownMenuItem>
        </DropdownMenu>
        <DropdownMenu variant="outline" trigger={<>Traefik ▾</>}>
          <DropdownMenuItem onSelect={() => setLogs("traefik")}>{t("Logs")}</DropdownMenuItem>
          <DropdownMenuItem onSelect={restartTraefik}>{t("Restart")}</DropdownMenuItem>
        </DropdownMenu>
        <DropdownMenu variant="outline" trigger={<>Docker ▾</>}>
          {PRUNES.map((p) => (
            <DropdownMenuItem key={p.kind} destructive={p.kind === "volumes"} onSelect={() => prune(p.kind, p.question)}>
              {t(p.label)}
            </DropdownMenuItem>
          ))}
        </DropdownMenu>
      </div>
      {s && (
        <p className="rdp2-small rdp2-muted">
          RDockPanel {s.version}
          {s.versions.commit && ` (${s.versions.commit.slice(0, 7)})`} · RustCMS Core {s.versions.rustcms} · {s.installed ? (s.image ?? t("installed")) : t("from source")} · {s.hostname}
          {s.public_ip && ` · ${s.public_ip}`}
        </p>
      )}
      {waiting && <Alert>{waiting}</Alert>}
      <ErrorNote error={error} />
      {logs && (
        <Dialog open onClose={() => setLogs(null)} title={logs === "panel" ? t("RDockPanel logs") : t("Traefik logs")} className="rdp2-wide">
          {logs === "panel" && !s?.service && <p className="rdp2-small rdp2-muted">{t("From source: the last lines this process wrote.")}</p>}
          <Logs {...props} url={logs === "panel" ? "/apps/system/logs/300" : "/apps/traefik/logs/200"} />
        </Dialog>
      )}
      {update && (
        <Dialog open onClose={() => setUpdate(null)} title={t("Check for updates")}>
          {!update.installed ? (
            <p>{t("This panel runs from source: update it with git pull and a new build.")}</p>
          ) : (
            <div className="rdp2-form">
              <Facts
                items={[
                  [t("Image"), <span className="rdp2-mono">{update.image ?? t("unknown")}</span>],
                  [t("Running"), <span className="rdp2-mono rdp2-ellipsis">{update.running ?? t("unknown")}</span>],
                  [t("Latest"), <span className="rdp2-mono rdp2-ellipsis">{update.latest ?? t("unknown")}</span>],
                ]}
              />
              {update.update === true && <Alert variant="warning">{t("A newer image is out. Run this on the server (as root): the installer keeps your settings.")}</Alert>}
              {update.update === false && <Alert>{t("The panel runs the latest image.")}</Alert>}
              {update.update == null && <Alert>{t("The registry did not say (offline, or it needs a login). The installer updates it anyway:")}</Alert>}
              {update.update !== false && update.command && <CodeBlock code={update.command} label={t("Installer command")} />}
            </div>
          )}
        </Dialog>
      )}
      {rebooting && s && <RebootDialog {...props} hostname={s.hostname} onClose={() => setRebooting(false)} onDone={() => {
        setRebooting(false);
        setWaiting(t("The server reboots… the page reloads when the panel is back."));
        void reloadWhenBack(window.location.href);
      }} />}
    </div>
  );
}

function RebootDialog(props: PageProps<unknown> & { hostname: string; onClose: () => void; onDone: () => void }) {
  const t = props.t;
  const [typed, setTyped] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<unknown>(null);
  return (
    <Dialog open onClose={props.onClose} title={t("Reboot the server")}>
      <form
        className="rdp2-form"
        onSubmit={async (e) => {
          e.preventDefault();
          setBusy(true);
          setError(null);
          try {
            await props.api.post("/apps/system/reboot", { confirm: typed });
            props.onDone();
          } catch (err) {
            setError(err);
          } finally {
            setBusy(false);
          }
        }}
      >
        <Alert variant="destructive">{t("Every service, the panel and Traefik stop until the server is back (usually a minute or two).")}</Alert>
        <label className="rdp2-form">
          <span className="rdp2-small">
            {t("Type the server's hostname to confirm")}: <strong className="rdp2-mono">{props.hostname}</strong>
          </span>
          <Input autoFocus value={typed} onChange={(e) => setTyped(e.target.value)} />
        </label>
        <ErrorNote error={error} />
        <div className="rdp2-row rdp2-end">
          <Button type="button" variant="ghost" onClick={props.onClose}>
            {t("Cancel")}
          </Button>
          <Button type="submit" variant="destructive" disabled={busy || typed.trim() !== props.hostname}>
            {t("Reboot")}
          </Button>
        </div>
      </form>
    </Dialog>
  );
}
