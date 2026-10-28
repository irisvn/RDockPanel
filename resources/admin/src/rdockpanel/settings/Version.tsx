// Settings → Version: what the panel runs, whether a newer release is out
// (checked every 6 hours, or by hand), what changed, the installer's command
// that updates it, what it runs with, its history, and its old versions
// cleaned up. Releases only (tags X.Y.Z): a `main` image is a development
// build. The panel never updates by itself: an administrator presses Update
// (or runs the installer's command).
import { useRef, useState } from "react";
import { type PageProps } from "@rustcms/admin-sdk";
import { useAdminQuery } from "../lib/query";
import { Alert, Button, CodeBlock, ErrorState, Loading } from "@rustcms/admin-sdk/ui";
import { EmptyState, ErrorNote, Facts, Section, StatusBadge, TableScroll, useAsk } from "../lib/ui";
import { dateTime, relative, size } from "../lib/format";
import { Markdown } from "../lib/markdown";

type Release = { version: string; name: string | null; notes: string | null; published_at: string | null; url: string | null };
type VersionStatus = {
  rdockpanel: {
    version: string;
    commit: string | null;
    image: string | null;
    digest: string | null;
    channel: "release" | "development" | "source";
    installed: boolean;
    source: string;
  };
  check: { checked_at: string | null; latest: string | null; error: string | null; error_at: string | null; cooldown: number };
  update: string | null;
  notes: Release[];
  command: string | null;
  components: Record<string, string | undefined>;
  history: Array<{ kind: string; target: string; status: string; error: string | null; log: string | null; at: string }>;
  updating: { from: string; to: string; stage: string; at: string } | null;
  newer_migrations: string[];
  can_update: boolean;
};

const STAGES: Record<string, string> = {
  queued: "Starting…",
  waiting: "Waiting for running deployments and backups to end…",
  "backing up": "Backing up the panel's database…",
  updating: "Swarm replaces the panel (it is unavailable for a moment)…",
};
type Cleanup = { containers: number; images: Array<{ id: string; tags: string[]; size: number }>; space: number };

const CHANNELS: Record<string, string> = {
  release: "Release",
  development: "Development build (main)",
  source: "From source",
};

const COMPONENTS: Array<[string, string]> = [
  ["rustcms", "RustCMS Core"],
  ["traefik", "Traefik"],
  ["mysql", "MySQL"],
  ["sqlite", "SQLite"],
  ["cloudflared", "cloudflared"],
  ["docker", "Docker Engine"],
  ["os", "Operating system"],
  ["kernel", "Kernel"],
  ["architecture", "Architecture"],
];

export function Version(props: PageProps<unknown>) {
  const t = props.t;
  // Followed while an update runs (the panel stops then: errors expected).
  const status = useAdminQuery<VersionStatus>(props, "/apps/system/version", {
    refetchInterval: (q) => (q.state.data?.updating || q.state.error ? 3000 : false),
  });
  // The version this page was loaded with: once another runs, reload it.
  const loaded = useRef<string | null>(null);
  if (status.data && loaded.current === null) loaded.current = status.data.rdockpanel.version;
  const cleanup = useAdminQuery<{ cleanup: Cleanup }>(props, "/apps/system/cleanup");
  const [checking, setChecking] = useState(false);
  const [cleaning, setCleaning] = useState(false);
  const [error, setError] = useState<unknown>(null);
  const ask = useAsk();
  if (status.isLoading) return <Loading />;
  if (!status.data) return <ErrorState error={status.error} />;
  const s = status.data;
  const p = s.rdockpanel;
  const checkNow = async () => {
    setChecking(true);
    setError(null);
    try {
      await props.api.post("/apps/system/version/check", {});
      await status.refetch();
    } catch (err) {
      setError(err);
    } finally {
      setChecking(false);
    }
  };
  const updateTo = async (version: string) => {
    const ok = await ask({
      title: t("Update RDockPanel to {v}?").replace("{v}", version),
      body: t(
        "New deployments and backups wait; those running end first. The panel's database is backed up, then Swarm replaces the panel: it is unavailable for about a minute. Your services keep running. If the new version does not start, Swarm goes back to {from} (which runs on the database as the new one left it; the backup is kept).",
      ).replace("{from}", p.version),
      action: t("Update to {v}").replace("{v}", version),
    });
    if (!ok) return;
    setError(null);
    try {
      await props.api.post("/apps/system/version/update", { version });
      await status.refetch();
    } catch (err) {
      setError(err);
    }
  };
  const removeOld = async (c: Cleanup) => {
    const ok = await ask({
      title: t("Remove the panel's old versions?"),
      body: t("{n} stopped containers of old tasks and {m} images ({size}). The running version and the one before it are kept; the removed tasks' logs are gone.")
        .replace("{n}", String(c.containers))
        .replace("{m}", String(c.images.length))
        .replace("{size}", size(c.space, props.locale)),
      action: t("Remove old versions"),
      destructive: true,
    });
    if (!ok) return;
    setCleaning(true);
    setError(null);
    try {
      const out = await props.api.post<{ cleanup: Cleanup }>("/apps/system/cleanup", {});
      props.toast.success(t("Removed {m} images, freed {size}.").replace("{m}", String(out.cleanup.images.length)).replace("{size}", size(out.cleanup.space, props.locale)));
      await cleanup.refetch();
    } catch (err) {
      setError(err);
    } finally {
      setCleaning(false);
    }
  };
  const checked = s.check.checked_at ? relative(s.check.checked_at, props.locale) : t("never");
  const c = cleanup.data?.cleanup;
  return (
    <>
      <ErrorNote error={error} />
      {s.updating && (
        <Alert>
          {`${t("Updating to {v}").replace("{v}", s.updating.to)}: ${t(STAGES[s.updating.stage] ?? s.updating.stage)}`}
          {status.error ? ` ${t("(the panel is restarting)")}` : ""}
        </Alert>
      )}
      {s.newer_migrations.length > 0 && (
        <Alert variant="warning">
          {t("A newer version migrated this database ({names}): this one runs on it as it is (an update rolled back). Update again once the newer release is fixed.").replace("{names}", s.newer_migrations.join(", "))}
        </Alert>
      )}
      {loaded.current && loaded.current !== p.version && (
        <Alert>
          <span className="rdp2-row rdp2-wrap">
            {t("RDockPanel {v} runs now.").replace("{v}", p.version)}
            <Button size="sm" variant="outline" onClick={() => window.location.reload()}>
              {t("Reload the page")}
            </Button>
          </span>
        </Alert>
      )}
      <Section
        title="RDockPanel"
        description={t("The panel checks for a newer release every 6 hours and says so here and on the Dashboard. It never updates by itself: you press Update.")}
        actions={
          p.installed && (
            <Button size="sm" variant="outline" disabled={checking} onClick={checkNow}>
              {checking ? t("Checking…") : t("Check now")}
            </Button>
          )
        }
      >
        <Facts
          items={[
            [t("Version"), <span className="rdp2-row">{p.version}{s.update ? <StatusBadge tone="busy" label={t("Update available")} /> : p.channel === "release" && s.check.latest && !s.check.error ? <StatusBadge tone="ok" label={t("Up to date")} /> : null}</span>],
            [
              t("Commit"),
              p.commit ? (
                <a className="rdp2-mono" href={`${p.source}/commit/${p.commit}`} target="_blank" rel="noreferrer">
                  {p.commit.slice(0, 7)}
                </a>
              ) : (
                "–"
              ),
            ],
            [t("Channel"), t(CHANNELS[p.channel] ?? p.channel)],
            [t("Image"), p.image ?? "–", { mono: true }],
            [t("Latest release"), s.check.latest ?? t("unknown")],
            [t("Last checked"), <span title={s.check.checked_at ? dateTime(s.check.checked_at, props.locale) : undefined}>{checked}</span>],
          ]}
        />
        {s.check.error && (
          <Alert variant="warning">
            {`${t("The last check failed")}: ${s.check.error}`}
            {s.check.checked_at && ` ${t("(what is shown is from {when})").replace("{when}", checked)}`}
          </Alert>
        )}
        {p.channel === "source" && <Alert>{t("This panel runs from source: update it with git pull and a new build.")}</Alert>}
        {p.channel === "development" && (
          <Alert>
            {s.check.latest
              ? t("A development build (main): it is not compared with releases. The newest release is {v}; to move to it, run this on the server (as root):").replace("{v}", s.check.latest)
              : t("A development build (main): it is not compared with releases.")}
          </Alert>
        )}
        {s.update && (
          <Alert variant="warning">
            <span className="rdp2-row rdp2-wrap">
              {t("RDockPanel {v} is out.").replace("{v}", s.update)}
              {s.can_update && !s.updating && (
                <Button size="sm" onClick={() => void updateTo(s.update as string)}>
                  {t("Update to {v}").replace("{v}", s.update)}
                </Button>
              )}
            </span>
          </Alert>
        )}
        {(s.update || p.channel === "development") && s.command && (
          <>
            {s.update && <p className="rdp2-small rdp2-muted">{t("Or run this on the server (as root): the installer keeps your settings.")}</p>}
            <CodeBlock code={s.command} label={t("Installer command")} />
          </>
        )}
      </Section>

      {s.notes.length > 0 && (
        <Section title={s.update ? t("What changed") : t("Release notes")} description={s.update ? t("From {from} to {to}.").replace("{from}", p.version).replace("{to}", s.update) : undefined}>
          {s.notes.map((r) => (
            <div key={r.version} className="rdp2-form">
              <div className="rdp2-row rdp2-wrap">
                <strong>{r.name ?? `RDockPanel ${r.version}`}</strong>
                {r.published_at && <span className="rdp2-small rdp2-muted">{dateTime(r.published_at, props.locale)}</span>}
                {r.url && (
                  <a className="rdp2-small" href={r.url} target="_blank" rel="noreferrer">
                    GitHub
                  </a>
                )}
              </div>
              {r.notes && (
                <div className="rdp2-notes">
                  <Markdown text={r.notes} />
                </div>
              )}
            </div>
          ))}
        </Section>
      )}

      <Section title={t("Runs with")} description={t("What the panel was built with and runs on.")}>
        <Facts
          items={COMPONENTS.filter(([key]) => s.components[key]).map(([key, label]) => [t(label), s.components[key] as string, { mono: key !== "os" }] as [string, string, { mono: boolean }])}
        />
      </Section>

      {p.installed && (
        <Section
          title={t("Old versions")}
          description={t("Docker keeps the stopped containers of each service's last tasks, and with them their images. The running version and the one before it are always kept.")}
          actions={
            c && c.images.length + c.containers > 0 ? (
              <Button size="sm" variant="outline" disabled={cleaning} onClick={() => void removeOld(c)}>
                {t("Remove old versions…")}
              </Button>
            ) : null
          }
        >
          {cleanup.error ? (
            <ErrorNote error={cleanup.error} />
          ) : !c ? (
            <Loading />
          ) : c.images.length + c.containers === 0 ? (
            <p className="rdp2-muted">{t("Nothing old to remove.")}</p>
          ) : (
            <p>
              {t("{n} stopped containers and {m} images ({size}) can go:")
                .replace("{n}", String(c.containers))
                .replace("{m}", String(c.images.length))
                .replace("{size}", size(c.space, props.locale))}{" "}
              <span className="rdp2-mono rdp2-muted">{c.images.flatMap((i) => i.tags).join(", ")}</span>
            </p>
          )}
        </Section>
      )}

      <Section title={t("History")} flush>
        {s.history.length === 0 ? (
          <EmptyState title={t("No version change or restart yet.")} />
        ) : (
          <TableScroll>
            <table className="rdp2-table">
              <thead>
                <tr>
                  <th>{t("When")}</th>
                  <th>{t("What")}</th>
                  <th>{t("Detail")}</th>
                  <th>{t("State")}</th>
                </tr>
              </thead>
              <tbody>
                {s.history.map((h, i) => (
                  <tr key={i}>
                    <td className="rdp2-nowrap" title={dateTime(h.at, props.locale)}>
                      {relative(h.at, props.locale)}
                    </td>
                    <td>{h.kind === "panel.version" ? t("Version changed") : h.kind === "panel.update" ? t("Update") : t("Panel restarted")}</td>
                    <td className="rdp2-mono" title={h.log ?? undefined}>
                      {h.kind === "panel.restart" ? "" : h.target}
                      {h.error && <div className="rdp2-small rdp2-error">{h.error}</div>}
                      {h.kind === "panel.update" && h.log && !h.error && <div className="rdp2-small rdp2-muted">{h.log}</div>}
                    </td>
                    <td>
                      <StatusBadge tone={h.status === "ok" ? "ok" : h.status === "failed" ? "bad" : "busy"} label={t(h.status === "ok" ? "Done" : h.status === "failed" ? "Failed" : "Running")} title={h.error ?? undefined} />
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </TableScroll>
        )}
      </Section>
    </>
  );
}
