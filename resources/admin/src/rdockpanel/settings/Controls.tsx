// Settings → Panel's controls, like EasyPanel's: Panel ▾ (its logs, a
// restart), Server ▾ (its public address found again, a reboot), Traefik ▾
// (its logs, a restart), Docker ▾ (prunes). The panel's administrators'
// (`system.manage`). Its version and updates: Settings → Version.
import { useState } from "react";
import { type PageProps } from "@rustcms/admin-sdk";
import { useAdminQuery } from "../lib/query";
import { Alert, Button, CodeBlock, Dialog, DropdownMenu, DropdownMenuItem, DropdownMenuSeparator, Input } from "@rustcms/admin-sdk/ui";
import { bytes, ErrorNote, Facts, RowActions, Section, Status, TableScroll, useAsk } from "../lib/ui";
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

/** Waits for the panel to answer again, then reloads the page; false when
 * it is not back within 3 minutes. */
async function reloadWhenBack(base: string): Promise<boolean> {
  const deadline = Date.now() + 180_000;
  // It stops first: give it time to go down.
  await new Promise((r) => setTimeout(r, 4000));
  while (Date.now() < deadline) {
    try {
      // Each try bounded: a hung request must not outlive the deadline.
      const r = await fetch("/health", { cache: "no-store", signal: AbortSignal.timeout(5000) });
      if (r.ok) {
        window.location.assign(base);
        return true;
      }
    } catch {
      // Not back yet.
    }
    await new Promise((r) => setTimeout(r, 2000));
  }
  return false;
}

/** The page's footer: what runs (versions, image) and where (host, address). */
export function SystemFooter(props: PageProps<unknown>) {
  const t = props.t;
  const s = useAdminQuery<Status>(props, "/apps/system/status").data;
  if (!s) return null;
  return (
    <p className="rdp2-small rdp2-muted rdp2-versions">
      RDockPanel {s.version}
      {s.versions.commit && ` (${s.versions.commit.slice(0, 7)})`} · RustCMS Core {s.versions.rustcms} · {s.installed ? (s.image ?? t("installed")) : t("from source")} · {s.hostname}
      {s.public_ip && ` · ${s.public_ip}`}
    </p>
  );
}

export function Controls(props: PageProps<unknown>) {
  const t = props.t;
  const status = useAdminQuery<Status>(props, "/apps/system/status");
  // Traefik's tasks, and whether Docker answers (the status needs it).
  const proxy = useAdminQuery<{ traefik: { found: boolean; tasks?: Array<{ state: string | null }> } }>(props, "/apps/traefik/status", {
    refetchInterval: 10000,
  });
  const traefikState = !proxy.data
    ? proxy.error
      ? "unknown"
      : "starting"
    : !proxy.data.traefik.found
      ? "off"
      : (proxy.data.traefik.tasks ?? []).some((task) => task.state === "running")
        ? "running"
        : "failed";
  const dockerState = status.error || proxy.error ? "unknown" : status.data ? "running" : "starting";
  const [logs, setLogs] = useState<"panel" | "traefik" | null>(null);
  const [rebooting, setRebooting] = useState(false);
  const [waiting, setWaiting] = useState<string | null>(null);
  const [error, setError] = useState<unknown>(null);
  const ask = useAsk();
  const s = status.data;
  const late = t("Not back after 3 minutes: check the server (docker service ps rdockpanel), then reload this page.");
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
    const ok = await ask({
      title: t("Restart the panel?"),
      body: t("It is unavailable for a few seconds; the page reloads when it is back."),
      action: t("Restart panel"),
      destructive: true,
    });
    if (!ok) return;
    if (await call(() => props.api.post("/apps/system/restart", {}))) {
      setWaiting(t("The panel restarts…"));
      void reloadWhenBack(window.location.href).then((back) => back || setWaiting(late));
    }
  };
  const restartTraefik = async () => {
    const ok = await ask({ title: t("Restart Traefik?"), body: t("Domains stop answering for a few seconds."), action: t("Restart Traefik"), destructive: true });
    if (!ok) return;
    if (await call(() => props.api.post("/apps/traefik/restart", {}))) props.toast.success(t("Traefik restarts."));
  };
  const refreshIp = async () => {
    const out = await call(() => props.api.post<{ public_ip: string; previous: string; changed: boolean }>("/apps/system/ip", {}));
    if (!out) return;
    props.toast.success(out.changed ? `${t("Public address")}: ${out.public_ip} (${t("was")} ${out.previous || t("unknown")})` : `${t("Public address unchanged")}: ${out.public_ip}`);
    status.refetch();
  };
  const prune = async (kind: string, label: string, question: string) => {
    if (!(await ask({ title: `${t(label)}?`, body: t(question), action: t(label), destructive: true }))) return;
    const out = await call(() => props.api.post<{ report: Report }>(`/apps/system/prune/${kind}`, {}));
    if (!out) return;
    const r = out.report;
    const n = r.containers_deleted + r.images_deleted + (r.volumes_deleted ?? 0) + (r.build_caches_deleted ?? 0);
    props.toast.success(`${t("Removed")}: ${n} · ${t("space freed")}: ${bytes(r.space_reclaimed)}`);
  };
  const more = (label: string) => t("More actions for {name}").replace("{name}", label);
  return (
    <Section title={t("System")} description={t("What the panel runs on. Restarts and reboots show their progress here.")} flush>
      {(waiting || error != null) && (
        <div className="rdp2-flush-note rdp2-form">
          {waiting && <Alert>{waiting}</Alert>}
          <ErrorNote error={error} />
        </div>
      )}
      <TableScroll>
        <table className="rdp2-table">
          <thead>
            <tr>
              <th>{t("Part")}</th>
              <th>{t("State")}</th>
              <th className="rdp2-right">
                <span className="rdp2-sr">{t("Actions")}</span>
              </th>
            </tr>
          </thead>
          <tbody>
            <tr>
              <td>
                <div className="rdp2-strong-cell">RDockPanel</div>
                <div className="rdp2-muted rdp2-small rdp2-ellipsis rdp2-cell-clip">
                  {s ? `${s.version} · ${s.installed ? (s.image ?? t("installed")) : t("from source")}` : "–"}
                </div>
              </td>
              <td>
                <Status kind="health" status={waiting ? "starting" : "running"} t={t} />
              </td>
              <td>
                <RowActions label={more("RDockPanel")} menu={[{ label: t("Restart…"), disabled: !s?.service, onSelect: () => void restartPanel() }]}>
                  <Button size="sm" variant="ghost" onClick={() => setLogs("panel")}>
                    {t("Logs")}
                  </Button>
                  <Button size="sm" variant="ghost" onClick={() => props.navigate("/p/settings/version")}>
                    {t("Version")}
                  </Button>
                </RowActions>
              </td>
            </tr>
            <tr>
              <td>
                <div className="rdp2-strong-cell">{t("Server")}</div>
                <div className="rdp2-muted rdp2-small">{s ? `${s.hostname}${s.public_ip ? ` · ${s.public_ip}` : ""}` : "–"}</div>
              </td>
              <td>
                <Status kind="health" status="running" t={t} />
              </td>
              <td>
                <RowActions
                  label={more(t("Server"))}
                  menu={[{ label: t("Reboot…"), destructive: true, disabled: !s?.can_reboot, onSelect: () => setRebooting(true) }]}
                >
                  <Button size="sm" variant="ghost" onClick={refreshIp}>
                    {t("Refresh IP")}
                  </Button>
                </RowActions>
              </td>
            </tr>
            <tr>
              <td>
                <div className="rdp2-strong-cell">Traefik</div>
                <div className="rdp2-muted rdp2-small">{t("Routes every domain")}</div>
              </td>
              <td>
                <Status kind="health" status={traefikState} t={t} />
              </td>
              <td>
                <RowActions label={more("Traefik")} menu={[{ label: t("Restart…"), onSelect: () => void restartTraefik() }]}>
                  <Button size="sm" variant="ghost" onClick={() => setLogs("traefik")}>
                    {t("Logs")}
                  </Button>
                </RowActions>
              </td>
            </tr>
            <tr>
              <td>
                <div className="rdp2-strong-cell">Docker</div>
                <div className="rdp2-muted rdp2-small">{t("Clean up what nothing uses (never the panel's)")}</div>
              </td>
              <td>
                <Status kind="health" status={dockerState} t={t} />
              </td>
              <td>
                <RowActions
                  label={more("Docker")}
                  menu={PRUNES.map((p) => ({ label: `${t(p.label)}…`, destructive: p.kind === "volumes", onSelect: () => void prune(p.kind, p.label, p.question) }))}
                />
              </td>
            </tr>
          </tbody>
        </table>
      </TableScroll>
      {logs && (
        <Dialog open onClose={() => setLogs(null)} title={logs === "panel" ? t("RDockPanel logs") : t("Traefik logs")} className="rdp2-wide">
          {logs === "panel" && !s?.service && <p className="rdp2-small rdp2-muted">{t("From source: the last lines this process wrote.")}</p>}
          <Logs {...props} url={logs === "panel" ? "/apps/system/logs/300" : "/apps/traefik/logs/200"} />
        </Dialog>
      )}
      {rebooting && s && <RebootDialog {...props} hostname={s.hostname} onClose={() => setRebooting(false)} onDone={() => {
        setRebooting(false);
        setWaiting(t("The server reboots… the page reloads when the panel is back."));
        void reloadWhenBack(window.location.href).then((back) => back || setWaiting(late));
      }} />}
    </Section>
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
