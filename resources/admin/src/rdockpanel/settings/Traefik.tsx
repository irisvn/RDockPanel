// Settings → Traefik, like EasyPanel's: its state (image, ports, tasks),
// "Restart", its live logs, and middlewares of your own (`custom-<name>`,
// YAML; a domain uses one as `custom-<name>@file`), written with no restart.
// The panel's administrators' (`system.manage`).
import { useState } from "react";
import { fieldErrors, type PageProps, useEvents } from "@rustcms/admin-sdk";
import { useAdminQuery } from "../lib/query";
import { Button, ErrorState, Loading, Textarea } from "@rustcms/admin-sdk/ui";
import { ErrorNote, Facts, Field, Section, StatusBadge, useAsk, when } from "../lib/ui";
import { LogViewer } from "../lib/viewers";

type Status = {
  traefik: {
    found: boolean;
    service?: string;
    image?: string;
    ports?: Array<{ published: number | null; target: number | null }>;
    tasks?: Array<{ state: string | null; since: string | null; error: string | null }>;
    update?: string | null;
  };
  middlewares: string;
};

const EXAMPLE = `custom-ratelimit:
  rateLimit:
    average: 100
    burst: 50
custom-security:
  headers:
    frameDeny: true
    stsSeconds: 31536000
`;

/** A service's live log (`url`: an SSE route sending `lines`). */
/** Logs at `url` (its line count at the end: `/apps/traefik/logs/200`),
 * followed live by the shared viewer, with the line choice. */
export function Logs(props: PageProps<unknown> & { url?: string }) {
  const base = (props.url ?? "/apps/traefik/logs/200").replace(/\/\d+$/, "");
  return <LogViewer {...props} url={(n) => `${base}/${n}`} />;
}

export function Traefik(props: PageProps<unknown>) {
  const t = props.t;
  const status = useAdminQuery<Status>(props, "/apps/traefik/status", { refetchInterval: 5000 });
  const [yaml, setYaml] = useState<string | null>(null);
  const [fields, setFields] = useState<Record<string, string[]>>({});
  const [error, setError] = useState<unknown>(null);
  const [busy, setBusy] = useState(false);
  const [showLogs, setShowLogs] = useState(false);
  const ask = useAsk();
  if (status.isLoading) return <Loading />;
  if (status.error || !status.data) return <ErrorState error={status.error} />;
  const s = status.data.traefik;
  const text = yaml ?? status.data.middlewares;
  const running = (s.tasks ?? []).filter((task) => task.state === "running").length;
  return (
    <Section
      title="Traefik"
      description={t("The proxy in front of every domain: its state, a restart, its logs, and middlewares of your own.")}
      actions={
        s.found && (
          <>
            <Button variant="outline" onClick={() => setShowLogs((v) => !v)}>
              {showLogs ? t("Hide logs") : t("Logs")}
            </Button>
            <Button
              variant="outline"
              disabled={busy}
              onClick={async () => {
                const ok = await ask({ title: t("Restart Traefik?"), body: t("Domains stop answering for a few seconds."), action: t("Restart Traefik"), destructive: true });
                if (!ok) return;
                setBusy(true);
                setError(null);
                try {
                  await props.api.post("/apps/traefik/restart", {});
                  props.toast.success(t("Traefik restarts."));
                  status.refetch();
                } catch (err) {
                  setError(err);
                } finally {
                  setBusy(false);
                }
              }}
            >
              {t("Restart")}
            </Button>
          </>
        )
      }
    >
      <ErrorNote error={error} />
      {!s.found ? (
        <p className="rdp2-muted">{t("Traefik is not a Swarm service of the panel's here.")}</p>
      ) : (
        <Facts
          items={[
            [t("Service"), <span className="rdp2-mono">{s.service}</span>],
            [t("Image"), <span className="rdp2-mono">{s.image}</span>],
            [
              t("Ports"),
              (s.ports ?? []).map((p) => `${p.published}→${p.target}`).join(", ") || t("none published"),
            ],
            [
              t("Tasks"),
              <StatusBadge status={running > 0 ? "running" : "failed"} label={`${running} ${t("running")}`} />,
            ],
            [t("Since"), when(s.tasks?.[0]?.since, props.locale)],
          ]}
        />
      )}
      {(s.tasks ?? []).some((task) => task.error) && (
        <p className="rdp2-error rdp2-small">{s.tasks?.find((task) => task.error)?.error}</p>
      )}
      {showLogs && s.found && <Logs {...props} />}
      <form
        className="rdp2-form"
        onSubmit={async (e) => {
          e.preventDefault();
          setFields({});
          setError(null);
          try {
            const out = await props.api.put<{ middlewares: string[] }>("/apps/traefik/middlewares", { yaml: text });
            props.toast.success(
              out.middlewares.length
                ? `${t("Saved: a domain uses them as")} ${out.middlewares.join(", ")}`
                : t("Saved."),
            );
            setYaml(null);
            status.refetch();
          } catch (err) {
            const f = fieldErrors(err);
            if (Object.keys(f).length) setFields(f);
            else setError(err);
          }
        }}
      >
        <Field
          label={t("Custom middlewares (YAML)")}
          help={t("Names start with custom-; a domain lists one as custom-name@file. Headers, limits, redirects, prefixes, compression…; no forwardAuth, errors or plugins.")}
          error={fields.yaml}
        >
          <Textarea className="rdp2-mono" rows={8} value={text} placeholder={EXAMPLE} onChange={(e) => setYaml(e.target.value)} />
        </Field>
        <div className="rdp2-row rdp2-end">
          <Button type="submit" disabled={yaml === null}>
            {t("Save")}
          </Button>
        </div>
      </form>
    </Section>
  );
}
