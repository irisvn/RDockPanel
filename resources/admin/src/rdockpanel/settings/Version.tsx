// Settings → Version: what the panel runs, whether a newer release is out
// (checked every 6 hours, or by hand), what changed, the installer's command
// that updates it, what it runs with, its history, and its old versions
// cleaned up. Releases only (tags X.Y.Z): a `main` image is a development
// build. The panel never updates itself: the user runs the command.
import { useState } from "react";
import { type PageProps, useAdminQuery } from "@rustcms/admin-sdk";
import { Alert, Button, CodeBlock, ErrorState, Loading } from "@rustcms/admin-sdk/ui";
import { EmptyState, ErrorNote, Facts, Section, StatusBadge, TableScroll, useAsk } from "../lib/ui";
import { dateTime, relative, size } from "../lib/format";

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
  history: Array<{ kind: string; target: string; status: string; error: string | null; at: string }>;
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
  ["cloudflared", "cloudflared"],
  ["docker", "Docker Engine"],
  ["os", "Operating system"],
  ["kernel", "Kernel"],
  ["architecture", "Architecture"],
];

export function Version(props: PageProps<unknown>) {
  const t = props.t;
  const status = useAdminQuery<VersionStatus>(props, "/apps/system/version");
  const cleanup = useAdminQuery<{ cleanup: Cleanup }>(props, "/apps/system/cleanup");
  const [checking, setChecking] = useState(false);
  const [cleaning, setCleaning] = useState(false);
  const [error, setError] = useState<unknown>(null);
  const ask = useAsk();
  if (status.isLoading) return <Loading />;
  if (status.error || !status.data) return <ErrorState error={status.error} />;
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
      <Section
        title="RDockPanel"
        description={t("The panel checks for a newer release every 6 hours and says so here and on the Dashboard. It never updates itself.")}
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
        {s.update && <Alert variant="warning">{t("RDockPanel {v} is out. To update, run this on the server (as root): the installer keeps your settings.").replace("{v}", s.update)}</Alert>}
        {(s.update || p.channel === "development") && s.command && <CodeBlock code={s.command} label={t("Installer command")} />}
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
              {r.notes && <div className="rdp2-notes">{r.notes}</div>}
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
                    <td>{h.kind === "panel.version" ? t("Version changed") : t("Panel restarted")}</td>
                    <td className="rdp2-mono">{h.kind === "panel.version" ? h.target : ""}</td>
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
