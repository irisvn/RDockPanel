// A service's page: its header (status, Deploy and the other actions, always
// visible) and its tabs, in the URL (`/p/dashboard/<project>/<service>/<tab>`).
// Saving never deploys: the page says when a change waits for Deploy.
import { type ReactNode, useEffect, useRef, useState } from "react";
import { type PageProps, useAdminQuery, useEvents } from "@rustcms/admin-sdk";
import { Alert, Button, Checkbox, Dialog, ErrorState, Input, Loading, NativeSelect, Switch, Textarea } from "@rustcms/admin-sdk/ui";
import { CopyButton, ErrorNote, Facts, Field, Section, Sparkline, StatusBadge, Tabs, bytes, when } from "../lib/ui";
import { ResourceError, resources } from "../lib/resources";
import { BackupsTab } from "./Backups";
import { MaintenanceTab, RedirectsTab, ScriptsTab, SecurityTab } from "./Access";
import type { Deployment, Domain, ServiceDetail } from "./types";
import { DomainForm, plainDomain } from "./DomainForm";

type Props = PageProps<unknown> & { project: string; serviceId: number; tab: string };
type T = (s: string) => string;

const TABS = ["overview", "credentials", "backups", "deployments", "environment", "domains", "redirects", "security", "maintenance", "scripts", "source", "storage", "advanced"] as const;

export function ServicePage(props: Props) {
  const t = props.t;
  const detail = useAdminQuery<ServiceDetail>(props, `/apps/services/${props.serviceId}`, { refetchInterval: 3000 });
  const [error, setError] = useState<unknown>(null);
  if (detail.isLoading) return <Loading />;
  if (detail.error || !detail.data) return <ErrorState error={detail.error} />;
  const d = detail.data;
  const s = d.service;
  const tab = (TABS as readonly string[]).includes(props.tab) ? props.tab : "overview";
  const base = `/p/dashboard/${props.project}/${s.name}`;
  const refresh = () => detail.refetch();

  const run = async (name: string, question?: string) => {
    if (question && !window.confirm(t(question))) return;
    setError(null);
    try {
      await resources.action("services", name, [s.id]);
      if (name !== "republish") props.navigate(`${base}/overview`);
      refresh();
    } catch (err) {
      setError(err);
    }
  };
  const saved = () => {
    props.toast.success(t("Saved. Deploy to apply it."));
    refresh();
  };
  const primary = d.domains[0];
  const running = s.status === "running";

  return (
    <div className="rdp2-page">
      <nav className="rdp2-crumbs">
        <button type="button" className="rdp2-link" onClick={() => props.navigate("/p/dashboard")}>
          {t("Dashboard")}
        </button>
        <span>/</span>
        <button type="button" className="rdp2-link" onClick={() => props.navigate(`/p/dashboard/${props.project}`)}>
          {d.project.name}
        </button>
        <span>/</span>
        <strong>{s.name}</strong>
        <StatusBadge status={s.status} busy={s.busy} label={t(s.busy ? "deploying" : s.status)} />
      </nav>
      <div className="rdp2-actions">
        {d.can.deploy && (
          <>
            <Button disabled={s.busy} onClick={() => run("deploy")}>
              {t("Deploy")}
            </Button>
            {s.status === "stopped" ? (
              <Button variant="outline" disabled={s.busy} onClick={() => run("start")}>
                {t("Start")}
              </Button>
            ) : (
              <Button variant="outline" disabled={s.busy || s.status === "new"} onClick={() => run("stop", "Stop this service? Its tasks are removed.")}>
                {t("Stop")}
              </Button>
            )}
            <Button variant="outline" disabled={s.busy || !running} onClick={() => run("restart")}>
              {t("Restart")}
            </Button>
            <Button
              variant="outline"
              disabled={s.busy}
              title={t("Build again without the build cache (an image: pull it again), then replace every task")}
              onClick={() => run("rebuild", "Force rebuild? It builds from scratch (or pulls the image again) and replaces every task.")}
            >
              {t("Force rebuild")}
            </Button>
          </>
        )}
        {primary && running && (
          <a className="rdp2-button-link" href={`${primary.https ? "https" : "http"}://${primary.host}${primary.path}`} target="_blank" rel="noreferrer">
            {t("Open")} ↗
          </a>
        )}
      </div>
      {d.pending && s.status !== "new" && <Alert variant="warning">{t("The configuration changed since the last deployment: deploy to apply it.")}</Alert>}
      <ErrorNote error={error} />
      <Tabs
        value={tab}
        onSelect={(id) => {
          props.navigate(`${base}/${id}`);
        }}
        tabs={[
          { id: "overview", label: t("Overview") },
          ...(d.database ? [{ id: "credentials", label: t("Credentials") }] : []),
          ...(d.database || d.mounts.some((m) => m.kind === "volume") ? [{ id: "backups", label: t("Backups") }] : []),
          { id: "deployments", label: t("Deployments") },
          { id: "environment", label: t("Environment") },
          { id: "domains", label: t("Domains") },
          ...(d.database ? [] : [
                { id: "redirects", label: t("Redirects") },
                { id: "security", label: t("Security") },
                { id: "maintenance", label: t("Maintenance") },
                ...(d.service.kind === "compose" ? [] : [{ id: "scripts", label: t("Scripts") }]),
              ]),
          { id: "source", label: t("Source") },
          { id: "storage", label: t("Storage") },
          { id: "advanced", label: t("Advanced") },
        ]}
      >
        {tab === "overview" && <OverviewTab {...props} d={d} />}
        {tab === "credentials" && d.database && <CredentialsTab {...props} d={d} />}
        {tab === "backups" && <BackupsTab {...props} d={d} />}
        {tab === "deployments" && <DeploymentsTab {...props} d={d} onError={setError} refresh={refresh} />}
        {tab === "environment" && <EnvironmentTab {...props} d={d} onSaved={saved} />}
        {tab === "domains" && !d.can.domains.view && <p className="rdp2-muted">{t("You may not see domains.")}</p>}
        {tab === "domains" && d.can.domains.view && <DomainsTab t={t} d={d} onSaved={saved} onError={setError} />}
        {tab === "redirects" && <RedirectsTab {...props} d={d} onSaved={saved} />}
        {tab === "security" && <SecurityTab {...props} d={d} onSaved={saved} />}
        {tab === "maintenance" && <MaintenanceTab {...props} d={d} onSaved={saved} />}
        {tab === "scripts" && <ScriptsTab {...props} d={d} onSaved={saved} />}
        {tab === "source" && <SourceTab t={t} d={d} api={props.api} onSaved={saved} />}
        {tab === "storage" && !d.can.mounts.view && <p className="rdp2-muted">{t("You may not see mounts.")}</p>}
        {tab === "storage" && d.can.mounts.view && <StorageTab t={t} d={d} onSaved={saved} onError={setError} />}
        {tab === "advanced" && (
          <AdvancedTab
            t={t}
            d={d}
            onSaved={saved}
            onError={setError}
            onDestroy={() => run("destroy", "Remove this service from Docker? Its volumes are kept.")}
            onDeleted={() => props.navigate(`/p/dashboard/${props.project}`)}
          />
        )}
      </Tabs>
    </div>
  );
}

/** A deployment's log: read once when it is over, followed while it runs. */
function DeploymentLog(props: PageProps<unknown> & { deployment: Deployment; onFinished: () => void }) {
  return props.deployment.finished ? <FinishedLog {...props} /> : <LiveLog {...props} />;
}

function FinishedLog(props: PageProps<unknown> & { deployment: Deployment }) {
  const row = useAdminQuery<{ log: string | null }>(props, `/apps/deployments/${props.deployment.id}`);
  return <pre className="rdp2-log">{row.data?.log ?? " "}</pre>;
}

function LiveLog(props: PageProps<unknown> & { deployment: Deployment; onFinished: () => void }) {
  const [log, setLog] = useState("");
  const pre = useRef<HTMLPreElement>(null);
  useEvents(props, `/deploy/deployments/${props.deployment.id}/log`, {
    onEvent(name, data) {
      if (name === "log") {
        const v = data as { reset?: string; append?: string };
        setLog((old) => (v.reset !== undefined ? v.reset : old + (v.append ?? "")));
      } else if (name === "status" && (data as { finished?: boolean }).finished) {
        props.onFinished();
      }
    },
  });
  useEffect(() => {
    if (pre.current) pre.current.scrollTop = pre.current.scrollHeight;
  }, [log]);
  return (
    <pre ref={pre} className="rdp2-log">
      {log || " "}
    </pre>
  );
}

function OverviewTab(props: Props & { d: ServiceDetail }) {
  const { d, t } = props;
  const s = d.service;
  return (
    <div className="rdp2-stack">
      <Section title={t("Service")}>
        <Facts
          items={[
            [t("Image"), <code>{s.image}</code>],
            [t("Replicas"), String(s.replicas)],
            [t("Internal host"), (
              <span className="rdp2-row">
                <code>{d.internal_host}</code>
                <CopyButton text={d.internal_host} label={t("Copy")} copied={t("Copied")} />
              </span>
            )],
            [t("Domain"), d.domains[0] ? <code>{d.domains[0].host}</code> : <span className="rdp2-muted">{t("none")}</span>],
            [t("First deployed"), s.deployed_at ? when(s.deployed_at, props.locale) : t("never")],
          ]}
        />
      </Section>
      {s.status === "running" && <Monitor {...props} />}
      <RecentErrors {...props} d={d} />
      <Notes {...props} d={d} />
      {d.latest && (
        <Section
          title={`${t("Deployment")} #${d.latest.id} · ${t(d.latest.rebuild ? "force rebuild" : d.latest.trigger)}`}
          actions={<StatusBadge status={d.latest.status} label={t(d.latest.status)} />}
        >
          {d.latest.error && <Alert variant="destructive">{d.latest.error}</Alert>}
          <DeploymentLog {...props} deployment={d.latest} onFinished={() => undefined} />
        </Section>
      )}
      {!d.latest && d.can.deployments && <p className="rdp2-muted">{t("Not deployed yet: press Deploy.")}</p>}
    </div>
  );
}

type Errors = {
  tasks: Array<{ at: string | null; slot: number | null; state: string; error: string; exit_code: number | null }>;
  deployment: { id: number; trigger: string; error: string | null; created_at: string; finished_at: string | null; log_tail: string } | null;
};

/** Its tasks that failed lately and its last failed deployment (EasyPanel's "recent errors"). */
function RecentErrors(props: Props & { d: ServiceDetail }) {
  const { d, t } = props;
  const errors = useAdminQuery<Errors>(props, `/apps/services/${d.service.id}/errors`, { refetchInterval: 10000 });
  const [open, setOpen] = useState(false);
  const e = errors.data;
  // The latest deployment (shown below) is not repeated.
  const deployment = e?.deployment && e.deployment.id !== d.latest?.id ? e.deployment : null;
  if (!e || (e.tasks.length === 0 && !deployment)) return null;
  return (
    <Section title={t("Recent errors")}>
      {e.tasks.length > 0 && (
        <ul className="rdp2-list">
          {e.tasks.slice(0, open ? undefined : 3).map((task, i) => (
            <li key={i} className="rdp2-list-row">
              <span className="rdp2-error rdp2-small">
                {t(task.state)}
                {task.exit_code != null && task.exit_code !== 0 && ` · ${t("exit code")} ${task.exit_code}`}
                {task.error && ` · ${task.error}`}
              </span>
              <span className="rdp2-muted rdp2-small">{when(task.at, props.locale)}</span>
            </li>
          ))}
        </ul>
      )}
      {e.tasks.length > 3 && (
        <Button size="sm" variant="ghost" onClick={() => setOpen((v) => !v)}>
          {open ? t("Show less") : `${t("Show all")} (${e.tasks.length})`}
        </Button>
      )}
      {deployment && (
        <div className="rdp2-form">
          <p className="rdp2-small">
            <strong>
              {t("Deployment")} #{deployment.id} · {t(deployment.trigger)}
            </strong>{" "}
            <span className="rdp2-muted">{when(deployment.finished_at ?? deployment.created_at, props.locale)}</span>
          </p>
          {deployment.error && <Alert variant="destructive">{deployment.error}</Alert>}
          {deployment.log_tail && <pre className="rdp2-log rdp2-mono rdp2-small">{deployment.log_tail}</pre>}
          <div className="rdp2-row">
            <Button size="sm" variant="outline" onClick={() => props.navigate(`/p/dashboard/${props.project}/${d.service.name}/deployments`)}>
              {t("All deployments")}
            </Button>
          </div>
        </div>
      )}
    </Section>
  );
}

/** Notes about the service, for the team (saved, never deployed). */
function Notes(props: Props & { d: ServiceDetail }) {
  const { d, t } = props;
  const [text, setText] = useState(d.service.notes ?? "");
  const [saved, setSaved] = useState(d.service.notes ?? "");
  const [error, setError] = useState<unknown>(null);
  if (!d.can.update && !saved) return null;
  return (
    <Section title={t("Notes")}>
      <Textarea rows={4} value={text} onChange={(e) => setText(e.target.value)} disabled={!d.can.update} placeholder={t("What the team should know about this service")} />
      <ErrorNote error={error} />
      {d.can.update && text !== saved && (
        <div className="rdp2-row rdp2-end">
          <Button
            size="sm"
            onClick={async () => {
              try {
                await resources.update("services", d.service.id, { notes: text });
                setSaved(text);
                props.toast.success(t("Saved."));
              } catch (err) {
                setError(err);
              }
            }}
          >
            {t("Save")}
          </Button>
        </div>
      )}
    </Section>
  );
}

type Stats = {
  cpu_percent: number;
  mem_used: number;
  missing: number;
  tasks: Array<{ slot: number; container: string | null; cpu_percent: number; mem_used: number; mem_limit: number }>;
};

/** CPU and memory of the service's tasks, every few seconds (EasyPanel's monitor). */
function Monitor(props: Props & { d: ServiceDetail }) {
  const { d, t } = props;
  const stats = useAdminQuery<Stats>(props, `/apps/services/${d.service.id}/stats`, { refetchInterval: 5000 });
  const [cpu, setCpu] = useState<number[]>([]);
  const [mem, setMem] = useState<number[]>([]);
  const last = useRef<Stats | null>(null);
  useEffect(() => {
    if (!stats.data || stats.data === last.current) return;
    last.current = stats.data;
    setCpu((v) => [...v.slice(-59), stats.data!.cpu_percent]);
    setMem((v) => [...v.slice(-59), stats.data!.mem_used]);
  }, [stats.data]);
  const data = stats.data;
  const container = data?.tasks.find((task) => task.container)?.container ?? null;
  if (stats.error)
    return (
      <Section title={t("Monitor")}>
        <ErrorNote error={stats.error} />
      </Section>
    );
  if (!data) return null;
  return (
    <Section
      title={t("Monitor")}
      actions={
        container && (
          <>
            {props.can("docker.view") && props.can("docker.logs") && (
              <a className="rdp2-button-link" href={`${document.baseURI.replace(/\/$/, "")}/p/containers?open=${container}&tab=logs`}>
                {t("Logs")}
              </a>
            )}
            {props.can("docker.view") && props.can("docker.exec") && (
              <a className="rdp2-button-link" href={`${document.baseURI.replace(/\/$/, "")}/p/containers?open=${container}&tab=terminal`}>
                {t("Shell")}
              </a>
            )}
          </>
        )
      }
    >
      <div className="rdp2-tiles">
        <div className="rdp2-tile">
          <span className="rdp2-muted rdp2-small">{t("CPU")}</span>
          <strong>{data.cpu_percent.toFixed(1)}%</strong>
          <Sparkline values={cpu} />
        </div>
        <div className="rdp2-tile">
          <span className="rdp2-muted rdp2-small">{t("Memory")}</span>
          <strong>{bytes(data.mem_used)}</strong>
          <Sparkline values={mem} />
        </div>
      </div>
      {data.missing > 0 && <p className="rdp2-muted rdp2-small">{t("Some tasks gave no figures: the totals leave them out.")}</p>}
      {data.tasks.length > 1 && (
        <p className="rdp2-muted rdp2-small">
          {data.tasks.map((task) => `#${task.slot}: ${task.cpu_percent.toFixed(1)}% · ${bytes(task.mem_used)}`).join("  ·  ")}
        </p>
      )}
    </Section>
  );
}

/** How apps of the project connect, with copy buttons. The passwords are
 * an audited read, only with `services.secrets`. */
function CredentialsTab(props: Props & { d: ServiceDetail }) {
  const { t, d } = props;
  const [secret, setSecret] = useState<{ password: string; root_password: string | null; url: string } | null>(null);
  const [error, setError] = useState<unknown>(null);
  useEffect(() => {
    if (!d.can.secrets) return;
    props.api
      .get<{ password: string; root_password: string | null; url: string }>(`/apps/services/${d.service.id}/credentials`)
      .then(setSecret)
      .catch(setError);
    // Once per service page.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [d.service.id, d.can.secrets]);
  const db = { ...d.database!, ...(secret ?? {}) };
  const row = (label: string, value: string | null | undefined) =>
    value ? (
      [
        label,
        <span className="rdp2-row">
          <code className="rdp2-ellipsis">{value}</code>
          <CopyButton text={value} label={t("Copy")} copied={t("Copied")} />
        </span>,
      ] as [string, ReactNode]
    ) : null;
  const items = [
    row(t("Internal host"), db.host),
    row(t("Port"), String(db.port)),
    row(t("User"), db.user),
    row(t("Password"), db.password),
    row(t("Database"), db.database || null),
    row(t("Root password"), db.root_password ?? null),
    row(t("Internal URL"), db.url),
  ].filter((x): x is [string, ReactNode] => x !== null);
  return (
    <Section
      title={t("Credentials")}
      description={t("Apps of this project connect with these. Changing the password here does not change it in a database already created.")}
    >
      {!d.can.secrets && <Alert variant="warning">{t("You may not read the password (services.secrets).")}</Alert>}
      <ErrorNote error={error} />
      <Facts items={items} />
    </Section>
  );
}

function DeploymentsTab(props: Props & { d: ServiceDetail; onError: (e: unknown) => void; refresh: () => void }) {
  const { d, t } = props;
  const [open, setOpen] = useState<Deployment | null>(null);
  if (!d.can.deployments) return <p className="rdp2-muted">{t("You may not see deployments.")}</p>;
  return (
    <Section title={t("Deployments")}>
      <table className="rdp2-table">
        <thead>
          <tr>
            <th>#</th>
            <th>{t("Action")}</th>
            <th>{t("Status")}</th>
            <th>{t("Started")}</th>
            <th>{t("Image")}</th>
            <th />
          </tr>
        </thead>
        <tbody>
          {d.deployments.map((row) => (
            <tr key={row.id}>
              <td>{row.id}</td>
              <td>{t(row.rebuild ? "force rebuild" : row.trigger)}</td>
              <td>
                <StatusBadge status={row.status} label={t(row.status)} />
              </td>
              <td className="rdp2-small">{when(row.created_at, props.locale)}</td>
              <td className="rdp2-small rdp2-ellipsis rdp2-mono">{row.image ?? ""}</td>
              <td className="rdp2-row rdp2-end">
                <Button size="sm" variant="ghost" onClick={() => setOpen(row)}>
                  {t("Log")}
                </Button>
                {d.can.deploy && row.status === "succeeded" && row.image && !["stop", "destroy"].includes(row.trigger) && (
                  <Button
                    size="sm"
                    variant="outline"
                    disabled={d.service.busy}
                    onClick={async () => {
                      if (!window.confirm(t("Deploy this configuration and image again?"))) return;
                      try {
                        await resources.action("deployments", "rollback", [row.id]);
                        props.refresh();
                      } catch (err) {
                        props.onError(err);
                      }
                    }}
                  >
                    {t("Roll back to this")}
                  </Button>
                )}
              </td>
            </tr>
          ))}
        </tbody>
      </table>
      {open && (
        <Dialog open onClose={() => setOpen(null)} title={`${t("Deployment")} #${open.id}`} className="rdp2-wide">
          {open.error && <Alert variant="destructive">{open.error}</Alert>}
          <DeploymentLog {...props} deployment={open} onFinished={() => setOpen((o) => o && { ...o, finished: true })} />
        </Dialog>
      )}
    </Section>
  );
}

function EnvironmentTab(props: Props & { d: ServiceDetail; onSaved: () => void }) {
  const { d, t } = props;
  const [text, setText] = useState<string | null>(null);
  // The revision read: a save over someone else's change is refused.
  const [revision, setRevision] = useState<string | undefined>(undefined);
  const [error, setError] = useState<unknown>(null);
  const [busy, setBusy] = useState(false);
  // Values in clear only with `services.secrets` (each read is audited).
  useEffect(() => {
    if (!d.can.secrets) {
      setText("");
      return;
    }
    props.api
      .get<{ text: string; revision: string }>(`/apps/services/${d.service.id}/env`)
      .then((r) => {
        setText(r.text);
        setRevision(r.revision);
      })
      .catch((err) => {
        setError(err);
        setText("");
      });
    // Once per service page.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [d.service.id, d.can.secrets]);
  if (text === null) return <Loading />;
  return (
    <Section
      title={t("Environment")}
      description={t("One NAME=value per line; # starts a comment. Values are encrypted; changes apply on the next deploy.")}
    >
      {!d.can.secrets && d.env_names.length > 0 && (
        <Alert variant="warning">
          {t("You may not read the values. Saving replaces every variable; set now:")} {d.env_names.join(", ")}
        </Alert>
      )}
      <Textarea
        className="rdp2-env"
        spellCheck={false}
        rows={14}
        value={text}
        disabled={!d.can.update}
        onChange={(e) => setText(e.target.value)}
        placeholder={"DATABASE_URL=postgres://user:password@shop_db:5432/shop\nNODE_ENV=production"}
      />
      <ErrorNote error={error} />
      {d.can.update && (
        <div className="rdp2-row rdp2-end">
          <Button
            disabled={busy}
            onClick={async () => {
              setBusy(true);
              setError(null);
              try {
                const body = revision === undefined ? { text } : { text, revision };
                await props.api.put(`/apps/services/${d.service.id}/env`, body);
                // The new revision: read again (an audited read).
                if (d.can.secrets) {
                  const r = await props.api.get<{ text: string; revision: string }>(`/apps/services/${d.service.id}/env`);
                  setText(r.text);
                  setRevision(r.revision);
                }
                props.onSaved();
              } catch (err) {
                setError(err);
              } finally {
                setBusy(false);
              }
            }}
          >
            {t("Save")}
          </Button>
        </div>
      )}
    </Section>
  );
}

function DomainsTab(props: { t: T; d: ServiceDetail; onSaved: () => void; onError: (e: unknown) => void }) {
  const { d, t } = props;
  // Adding (with a host to start with), or changing one.
  const [adding, setAdding] = useState<string | null>(null);
  const [editing, setEditing] = useState<Domain | null>(null);
  const resolverName = (id: number | null) => d.cert_resolvers.find((r) => r.id === id)?.name;
  const certificateName = (id: number | null) => d.certificates.find((c) => c.id === id)?.name;
  const mayChange = (domain: Domain) => d.can.system || plainDomain(domain);
  return (
    <Section
      title={t("Domains")}
      description={t("Traefik routes these to the service's port. HTTPS redirects HTTP to it.")}
      actions={
        d.can.domains.create && (
          <>
            {d.auto_domain && !d.domains.some((x) => x.host === d.auto_domain) && (
              <Button variant="outline" onClick={() => setAdding(d.auto_domain ?? "")}>
                + {t("Generated domain")}
              </Button>
            )}
            <Button onClick={() => setAdding("")}>+ {t("Add domain")}</Button>
          </>
        )
      }
    >
      {d.domains.length === 0 && <p className="rdp2-muted">{t("No domain: the service is only reachable inside its project.")}</p>}
      <ul className="rdp2-list">
        {d.domains.map((domain, i) => (
          <li key={domain.id} className="rdp2-list-row">
            <span>
              {i === 0 && <span title={t("Primary domain")}>★ </span>}
              <code>
                {domain.https ? "https" : "http"}://{domain.host}
                {domain.path}
              </code>{" "}
              <span className="rdp2-muted">
                → {domain.compose_service ? `${domain.compose_service}:` : ":"}
                {domain.port}
                {domain.https && domain.cert_resolver_id !== null && ` · 🔒 ${resolverName(domain.cert_resolver_id) ?? t("own resolver")}`}
                {domain.https && domain.certificate_id !== null && ` · 🔒 ${certificateName(domain.certificate_id) ?? t("uploaded certificate")}`}
              </span>
            </span>
            <span className="rdp2-row">
              {d.can.domains.update && mayChange(domain) && (
                <Button size="sm" variant="ghost" onClick={() => setEditing(domain)}>
                  {t("Edit")}
                </Button>
              )}
              {d.can.domains.delete && mayChange(domain) && (
                <Button
                  size="sm"
                  variant="ghost"
                  onClick={async () => {
                    if (!window.confirm(t("Remove this domain?"))) return;
                    try {
                      await resources.remove("service_domains", domain.id);
                      props.onSaved();
                    } catch (err) {
                      props.onError(err);
                    }
                  }}
                >
                  {t("Remove")}
                </Button>
              )}
            </span>
          </li>
        ))}
      </ul>
      {(adding !== null || editing) && (
        <DomainForm
          t={t}
          domain={editing ?? undefined}
          service={{ id: d.service.id, kind: d.service.kind }}
          host={adding ?? undefined}
          cert_resolvers={d.cert_resolvers}
          certificates={d.certificates}
          defaultResolver={d.default_resolver}
          system={d.can.system}
          onClose={() => {
            setAdding(null);
            setEditing(null);
          }}
          onSaved={() => {
            setAdding(null);
            setEditing(null);
            props.onSaved();
          }}
        />
      )}
    </Section>
  );
}

type GitHubToken = { id: number; name: string; account: string | null };
type GitHubRepo = { full_name: string; private: boolean; default_branch: string | null; url: string };

/** Clone with a GitHub token of Settings → Git (given to the project): its
 * repositories and their branches to pick from. */
function GitHubPicker(props: {
  t: T;
  d: ServiceDetail;
  api: Props["api"];
  provider: number | null;
  url: string;
  branch: string;
  error?: string[];
  disabled: boolean;
  onProvider: (id: number | null) => void;
  onRepo: (url: string, defaultBranch: string | null) => void;
}) {
  const { t, d } = props;
  const [tokens, setTokens] = useState<GitHubToken[] | null>(null);
  const [repos, setRepos] = useState<GitHubRepo[] | null>(null);
  const [branches, setBranches] = useState<string[]>([]);
  const [filter, setFilter] = useState("");
  const [error, setError] = useState<unknown>(null);
  const base = `/apps/services/${d.service.id}/github`;
  useEffect(() => {
    if (!d.can.update) return;
    props.api.get<{ providers: GitHubToken[] }>(base).then((r) => setTokens(r.providers), setError);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [base]);
  const repoName = props.url.replace(/^https:\/\/github\.com\//, "").replace(/\.git$/, "");
  useEffect(() => {
    if (!props.provider || !/^[\w.-]+\/[\w.-]+$/.test(repoName)) return setBranches([]);
    props.api
      .get<{ branches: string[] }>(`${base}/${props.provider}/branches`, { query: { repo: repoName } })
      .then((r) => setBranches(r.branches), () => setBranches([]));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [props.provider, repoName]);
  if (!tokens) return <ErrorNote error={error} />;
  if (tokens.length === 0 && !props.provider) {
    return <p className="rdp2-small rdp2-muted">{t("A private GitHub repository: add a GitHub token in Settings → Git and give it this project, or use a deploy key.")}</p>;
  }
  const shown = (repos ?? []).filter((r) => r.full_name.toLowerCase().includes(filter.toLowerCase())).slice(0, 200);
  return (
    <div className="rdp2-form">
      <Field label={t("Clone with")} error={props.error}>
        <NativeSelect
          value={props.provider ?? ""}
          disabled={props.disabled}
          onChange={(e) => {
            props.onProvider(e.target.value ? Number(e.target.value) : null);
            setRepos(null);
          }}
        >
          <option value="">{t("A public URL, or the deploy key")}</option>
          {tokens.map((tok) => (
            <option key={tok.id} value={tok.id}>
              GitHub · {tok.name}
              {tok.account ? ` (${tok.account})` : ""}
            </option>
          ))}
        </NativeSelect>
      </Field>
      {props.provider && !props.disabled && (
        <div className="rdp2-form">
          {repos === null ? (
            <div className="rdp2-row">
              <Button
                type="button"
                variant="outline"
                size="sm"
                onClick={() =>
                  props.api
                    .get<{ repositories: GitHubRepo[] }>(`${base}/${props.provider}/repos`)
                    .then((r) => setRepos(r.repositories), setError)
                }
              >
                {t("Choose a repository")}
              </Button>
            </div>
          ) : (
            <div className="rdp2-grid2">
              <Input value={filter} onChange={(e) => setFilter(e.target.value)} placeholder={t("Search repositories")} />
              <NativeSelect
                value={repos.find((r) => r.url === props.url)?.url ?? ""}
                onChange={(e) => {
                  const repo = repos.find((r) => r.url === e.target.value);
                  if (repo) props.onRepo(repo.url, repo.default_branch);
                }}
              >
                <option value="">{`${t("Repositories")} (${repos.length})`}</option>
                {shown.map((r) => (
                  <option key={r.url} value={r.url}>
                    {r.full_name}
                    {r.private ? " 🔒" : ""}
                  </option>
                ))}
              </NativeSelect>
            </div>
          )}
          {branches.length > 0 && (
            <p className="rdp2-small rdp2-muted">
              {t("Branches")}:{" "}
              {branches.slice(0, 30).map((b, i) => (
                <span key={b}>
                  {i > 0 && ", "}
                  <code className={b === props.branch ? "rdp2-strong" : undefined}>{b}</code>
                </span>
              ))}
            </p>
          )}
          <ErrorNote error={error} />
        </div>
      )}
    </div>
  );
}

/** Saves `data` on the service (the resource's own validation). */
function useServiceSave(id: number, onSaved: () => void) {
  const [error, setError] = useState<unknown>(null);
  const [busy, setBusy] = useState(false);
  const save = async (data: Record<string, unknown>) => {
    setBusy(true);
    setError(null);
    try {
      await resources.update("services", id, data);
      onSaved();
    } catch (err) {
      setError(err);
    } finally {
      setBusy(false);
    }
  };
  const fields = error instanceof ResourceError ? error.fieldErrors : {};
  return { save, error, busy, fields };
}

function SourceTab(props: { t: T; d: ServiceDetail; api: Props["api"]; onSaved: () => void }) {
  const { d, t } = props;
  const s = d.service;
  const [source, setSource] = useState(s.source);
  const [image, setImage] = useState(s.image);
  const [user, setUser] = useState(s.registry_username ?? "");
  const [password, setPassword] = useState("");
  const [gitUrl, setGitUrl] = useState(s.git_url ?? "");
  const [branch, setBranch] = useState(s.git_branch ?? "main");
  const [path, setPath] = useState(s.git_path ?? "/");
  const [provider, setProvider] = useState<number | null>(s.git_provider_id);
  const [builder, setBuilder] = useState(s.builder);
  const [dockerfilePath, setDockerfilePath] = useState(s.dockerfile_path ?? "Dockerfile");
  const [dockerfile, setDockerfile] = useState(s.dockerfile ?? "FROM nginx:1.27-alpine\n");
  const [compose, setCompose] = useState(s.compose ?? "");
  const [composeFile, setComposeFile] = useState(s.compose_file ?? "docker-compose.yml");
  const isCompose = s.kind === "compose";
  const [trigger, setTrigger] = useState<string | null>(null);
  const [webhook, setWebhook] = useState<{ url: string; secret: string } | null>(null);
  const [extra, setExtra] = useState<unknown>(null);
  const { save, error, busy, fields } = useServiceSave(s.id, () => {
    setPassword("");
    props.onSaved();
  });
  const disabled = !d.can.update;
  return (
    <div className="rdp2-stack">
      <Section title={t("Source")} description={t("Where the image comes from: a registry, a Git repository built here, or a Dockerfile written here.")}>
        <form
          className="rdp2-form"
          onSubmit={(e) => {
            e.preventDefault();
            const data: Record<string, unknown> = { source };
            if (isCompose && source === "image") {
              data.compose = compose;
            } else if (isCompose) {
              Object.assign(data, {
                git_url: gitUrl.trim(),
                git_branch: branch.trim() || null,
                git_path: path.trim() || "/",
                git_provider_id: provider === null ? null : String(provider),
                compose_file: composeFile.trim() || "docker-compose.yml",
              });
            } else if (source === "image") {
              Object.assign(data, { image: image.trim(), registry_username: user.trim() || null });
              if (password) data.registry_password = { replace: password };
            } else if (source === "git") {
              Object.assign(data, {
                git_url: gitUrl.trim(),
                git_branch: branch.trim() || null,
                git_path: path.trim() || "/",
                git_provider_id: provider === null ? null : String(provider),
                builder,
                dockerfile_path: dockerfilePath.trim() || "Dockerfile",
              });
            } else {
              Object.assign(data, { dockerfile });
            }
            save(data);
          }}
        >
          <div className="rdp2-kinds">
            {(isCompose
              ? (["image", "git"] as const)
              : props.d.service.kind === "app"
                ? (["image", "git", "dockerfile"] as const)
                : (["image"] as const)
            ).map((id) => (
              <button key={id} type="button" disabled={disabled} className={id === source ? "rdp2-kind rdp2-kind-active" : "rdp2-kind"} onClick={() => setSource(id)}>
                {t(isCompose && id === "image" ? "Compose file" : id === "image" ? "Docker image" : id === "git" ? "Git repository" : "Dockerfile")}
              </button>
            ))}
          </div>
          {isCompose && source === "image" && (
            <Field label={t("Compose file")} help={t("${VAR} reads the Environment tab. Domains lead to one of its services.")} error={fields.compose}>
              <Textarea className="rdp2-env" rows={16} value={compose} onChange={(e) => setCompose(e.target.value)} disabled={disabled} />
            </Field>
          )}
          {isCompose && source === "git" && (
            <>
              <GitHubPicker
                {...props}
                provider={provider}
                url={gitUrl}
                error={fields.git_provider_id}
                disabled={disabled}
                onProvider={setProvider}
                onRepo={(url, defaultBranch) => {
                  setGitUrl(url);
                  if (defaultBranch) setBranch(defaultBranch);
                }}
                branch={branch}
              />
              <Field label={t("Repository URL")} help={provider ? t("https://github.com/<owner>/<repo>.git, cloned with the token") : t("https://github.com/org/app.git, or git@github.com:org/app.git with a deploy key")} error={fields.git_url}>
                <Input value={gitUrl} onChange={(e) => setGitUrl(e.target.value)} disabled={disabled} placeholder="https://github.com/org/app.git" />
              </Field>
              <div className="rdp2-grid2">
                <Field label={t("Branch, tag or commit")} error={fields.git_branch}>
                  <Input value={branch} onChange={(e) => setBranch(e.target.value)} disabled={disabled} />
                </Field>
                <Field label={t("Build path")} help={t("The Compose project's folder")} error={fields.git_path}>
                  <Input value={path} onChange={(e) => setPath(e.target.value)} disabled={disabled} />
                </Field>
                <Field label={t("Compose file")} error={fields.compose_file}>
                  <Input value={composeFile} onChange={(e) => setComposeFile(e.target.value)} disabled={disabled} />
                </Field>
              </div>
            </>
          )}
          {!isCompose && source === "image" && (
            <>
              <Field label={t("Image")} help={t("Pulled and pinned to its digest at each deploy.")} error={fields.image}>
                <Input value={image} onChange={(e) => setImage(e.target.value)} disabled={disabled} />
              </Field>
              <div className="rdp2-grid2">
                <Field label={t("Registry user")} help={t("For a private image")} error={fields.registry_username}>
                  <Input value={user} onChange={(e) => setUser(e.target.value)} disabled={disabled} />
                </Field>
                <Field label={t("Registry password or token")} help={s.registry_password_set ? t("Set: leave empty to keep it") : t("Not set")} error={fields.registry_password}>
                  <Input type="password" value={password} onChange={(e) => setPassword(e.target.value)} disabled={disabled} autoComplete="new-password" />
                </Field>
              </div>
            </>
          )}
          {!isCompose && source === "git" && (
            <>
              <GitHubPicker
                {...props}
                provider={provider}
                url={gitUrl}
                error={fields.git_provider_id}
                disabled={disabled}
                onProvider={setProvider}
                onRepo={(url, defaultBranch) => {
                  setGitUrl(url);
                  if (defaultBranch) setBranch(defaultBranch);
                }}
                branch={branch}
              />
              <Field label={t("Repository URL")} help={provider ? t("https://github.com/<owner>/<repo>.git, cloned with the token") : t("https://github.com/org/app.git, or git@github.com:org/app.git with a deploy key")} error={fields.git_url}>
                <Input value={gitUrl} onChange={(e) => setGitUrl(e.target.value)} disabled={disabled} placeholder="https://github.com/org/app.git" />
              </Field>
              <div className="rdp2-grid2">
                <Field label={t("Branch, tag or commit")} error={fields.git_branch}>
                  <Input value={branch} onChange={(e) => setBranch(e.target.value)} disabled={disabled} />
                </Field>
                <Field label={t("Build path")} help={t("The folder of the repository to build")} error={fields.git_path}>
                  <Input value={path} onChange={(e) => setPath(e.target.value)} disabled={disabled} />
                </Field>
                <Field label={t("Builder")}>
                  <NativeSelect value={builder} onChange={(e) => setBuilder(e.target.value as "dockerfile" | "nixpacks")} disabled={disabled}>
                    <option value="dockerfile">Dockerfile</option>
                    <option value="nixpacks">Nixpacks</option>
                  </NativeSelect>
                </Field>
                {builder === "dockerfile" && (
                  <Field label={t("Dockerfile path")} error={fields.dockerfile_path}>
                    <Input value={dockerfilePath} onChange={(e) => setDockerfilePath(e.target.value)} disabled={disabled} />
                  </Field>
                )}
              </div>
            </>
          )}
          {source === "dockerfile" && (
            <Field label={t("Dockerfile")} error={fields.dockerfile}>
              <Textarea className="rdp2-env" rows={12} value={dockerfile} onChange={(e) => setDockerfile(e.target.value)} disabled={disabled} />
            </Field>
          )}
          {Object.keys(fields).length === 0 && <ErrorNote error={error} />}
          {d.can.update && (
            <div className="rdp2-row rdp2-end">
              {source === "image" && s.registry_password_set && (
                <Button type="button" variant="ghost" onClick={() => save({ registry_password: { clear: true } })}>
                  {t("Remove password")}
                </Button>
              )}
              <Button type="submit" disabled={busy}>
                {t("Save")}
              </Button>
            </div>
          )}
        </form>
      </Section>
      {s.source === "git" && d.can.update && (
        <Section
          title={t("Deploy key")}
          description={t("For a private repository: add this public key to it (read-only), and use its ssh URL.")}
          actions={
            <Button
              variant="outline"
              onClick={async () => {
                if (s.deploy_key_public && !window.confirm(t("Replace the deploy key? The old one stops working."))) return;
                try {
                  await props.api.post(`/apps/services/${s.id}/deploy-key`);
                  props.onSaved();
                } catch (err) {
                  setExtra(err);
                }
              }}
            >
              {s.deploy_key_public ? t("New key") : t("Generate a key")}
            </Button>
          }
        >
          {s.deploy_key_public && (
            <span className="rdp2-row">
              <code className="rdp2-ellipsis">{s.deploy_key_public}</code>
              <CopyButton text={s.deploy_key_public} label={t("Copy")} copied={t("Copied")} />
            </span>
          )}
        </Section>
      )}
      {s.source !== "image" && d.can.deploy && (
        <Section
          title={t("Deployment trigger")}
          description={t("POST to this URL deploys the service: a GitHub webhook (push) or a CI step. Shown once.")}
          actions={
            <Button
              variant="outline"
              onClick={async () => {
                if (s.trigger_set && !window.confirm(t("Make a new URL? The old one stops working."))) return;
                try {
                  const r = await props.api.post<{ url: string }>(`/apps/services/${s.id}/trigger`);
                  setTrigger(r.url);
                  props.onSaved();
                } catch (err) {
                  setExtra(err);
                }
              }}
            >
              {s.trigger_set ? t("New URL") : t("Make a URL")}
            </Button>
          }
        >
          {trigger && (
            <span className="rdp2-row">
              <code className="rdp2-ellipsis">{trigger}</code>
              <CopyButton text={trigger} label={t("Copy")} copied={t("Copied")} />
            </span>
          )}
          {!trigger && s.trigger_set && <p className="rdp2-muted rdp2-small">{t("A URL is set (shown when it was made).")}</p>}
        </Section>
      )}
      {s.source === "git" && d.can.deploy && (
        <Section
          title={t("Auto deploy (GitHub)")}
          description={t("A push to the branch deploys the service. In the repository: Settings → Webhooks → Add webhook, content type application/json, this URL and secret, the push event.")}
          actions={
            <Button
              variant="outline"
              onClick={async () => {
                if (s.webhook_set && !window.confirm(t("Make a new secret? The old one stops working."))) return;
                try {
                  setWebhook(await props.api.post<{ url: string; secret: string }>(`/apps/services/${s.id}/github-webhook`));
                  props.onSaved();
                } catch (err) {
                  setExtra(err);
                }
              }}
            >
              {s.webhook_set ? t("New secret") : t("Enable")}
            </Button>
          }
        >
          {webhook && (
            <div className="rdp2-stack">
              <span className="rdp2-row">
                <code className="rdp2-ellipsis">{webhook.url}</code>
                <CopyButton text={webhook.url} label={t("Copy")} copied={t("Copied")} />
              </span>
              <span className="rdp2-row">
                <code className="rdp2-ellipsis">{webhook.secret}</code>
                <CopyButton text={webhook.secret} label={t("Copy")} copied={t("Copied")} />
              </span>
            </div>
          )}
          {!webhook && s.webhook_set && <p className="rdp2-muted rdp2-small">{t("On (the secret was shown when it was made).")}</p>}
        </Section>
      )}
      <ErrorNote error={extra} />
    </div>
  );
}

function StorageTab(props: { t: T; d: ServiceDetail; onSaved: () => void; onError: (e: unknown) => void }) {
  const { d, t } = props;
  const [adding, setAdding] = useState(false);
  const [kind, setKind] = useState("volume");
  const [name, setName] = useState("");
  const [target, setTarget] = useState("");
  const [readOnly, setReadOnly] = useState(false);
  const [content, setContent] = useState("");
  const [error, setError] = useState<unknown>(null);
  const fields = error instanceof ResourceError ? error.fieldErrors : {};
  return (
    <Section
      title={t("Mounts")}
      description={t("Volumes keep data across deploys. A host directory needs the system permission.")}
      actions={d.can.mounts.create && <Button onClick={() => setAdding(true)}>+ {t("Add mount")}</Button>}
    >
      {d.mounts.length === 0 && <p className="rdp2-muted">{t("No mount: the container's files are lost when it is replaced.")}</p>}
      <ul className="rdp2-list">
        {d.mounts.map((m) => (
          <li key={m.id} className="rdp2-list-row">
            <span>
              <span className="rdp2-muted">{t(m.kind)}</span>{" "}
              {m.kind === "file" ? <code>{m.target}</code> : <><code>{m.kind === "bind" ? m.source : m.name}</code> → <code>{m.target}</code></>}
              {m.read_only && <span className="rdp2-muted"> ({t("read-only")})</span>}
            </span>
            {d.can.mounts.delete && (
              <Button
                size="sm"
                variant="ghost"
                onClick={async () => {
                  if (!window.confirm(t("Remove this mount? A volume's data stays in Docker."))) return;
                  try {
                    await resources.remove("service_mounts", m.id);
                    props.onSaved();
                  } catch (err) {
                    props.onError(err);
                  }
                }}
              >
                {t("Remove")}
              </Button>
            )}
          </li>
        ))}
      </ul>
      {adding && (
        <Dialog open onClose={() => setAdding(false)} title={t("Add mount")}>
          <form
            className="rdp2-form"
            onSubmit={async (e) => {
              e.preventDefault();
              setError(null);
              try {
                await resources.create("service_mounts", {
                  service_id: String(d.service.id),
                  kind,
                  ...(kind === "bind" ? { source: name.trim() } : kind === "file" ? { sealed_content: { replace: content } } : { name: name.trim() }),
                  target: target.trim(),
                  read_only: kind === "file" ? true : readOnly,
                });
                setContent("");
                setAdding(false);
                setName("");
                setTarget("");
                props.onSaved();
              } catch (err) {
                setError(err);
              }
            }}
          >
            <Field label={t("Type")}>
              <NativeSelect value={kind} onChange={(e) => setKind(e.target.value)}>
                <option value="volume">{t("Volume")}</option>
                <option value="bind">{t("Host directory")}</option>
                <option value="file">{t("File")}</option>
              </NativeSelect>
            </Field>
            {kind !== "file" && (
              <Field label={kind === "bind" ? t("Host directory") : t("Volume name")} error={kind === "bind" ? fields.source : fields.name}>
                <Input autoFocus value={name} onChange={(e) => setName(e.target.value)} placeholder={kind === "bind" ? "/srv/data" : "data"} />
              </Field>
            )}
            <Field label={kind === "file" ? t("File path in the container") : t("Path in the container")} error={fields.target}>
              <Input value={target} onChange={(e) => setTarget(e.target.value)} placeholder={kind === "file" ? "/etc/app/config.ini" : "/var/lib/app"} />
            </Field>
            {kind === "file" && (
              <Field label={t("Content")} help={t("A config file, read-only in the container; sealed, never shown again")} error={fields.sealed_content}>
                <Textarea className="rdp2-env" rows={8} value={content} onChange={(e) => setContent(e.target.value)} />
              </Field>
            )}
            {kind !== "file" && (
              <label className="rdp2-row">
                <Checkbox checked={readOnly} onChange={(e) => setReadOnly(e.currentTarget.checked)} /> {t("Read-only")}
              </label>
            )}
            {Object.keys(fields).length === 0 && <ErrorNote error={error} />}
            <div className="rdp2-row rdp2-end">
              <Button type="button" variant="ghost" onClick={() => setAdding(false)}>
                {t("Cancel")}
              </Button>
              <Button type="submit" disabled={(kind !== "file" && !name.trim()) || !target.trim()}>
                {t("Add")}
              </Button>
            </div>
          </form>
        </Dialog>
      )}
    </Section>
  );
}

const num = (v: string) => (v.trim() === "" ? null : Number(v));
const str = (v: number | null | undefined) => (v === null || v === undefined ? "" : String(v));

function AdvancedTab(props: {
  t: T;
  d: ServiceDetail;
  onSaved: () => void;
  onError: (e: unknown) => void;
  onDestroy: () => void;
  onDeleted: () => void;
}) {
  const { d, t } = props;
  const s = d.service;
  const [replicas, setReplicas] = useState(String(s.replicas));
  const [command, setCommand] = useState(s.command ?? "");
  const [zero, setZero] = useState(s.zero_downtime);
  const [check, setCheck] = useState(s.healthcheck_cmd ?? "");
  const [interval, setInterval] = useState(str(s.healthcheck_interval_s));
  const [retries, setRetries] = useState(str(s.healthcheck_retries));
  const [cpu, setCpu] = useState(str(s.cpu_limit_millis));
  const [mem, setMem] = useState(str(s.mem_limit_mb));
  const [cpuR, setCpuR] = useState(str(s.cpu_reservation_millis));
  const [memR, setMemR] = useState(str(s.mem_reservation_mb));
  const [capAdd, setCapAdd] = useState(s.cap_add ?? "");
  const [capDrop, setCapDrop] = useState(s.cap_drop ?? "");
  const [sysctls, setSysctls] = useState(s.sysctls ?? "");
  const [groups, setGroups] = useState(s.groups ?? "");
  const { save, error, busy, fields } = useServiceSave(s.id, props.onSaved);
  const [published, setPublished] = useState("");
  const [targetPort, setTargetPort] = useState("");
  const [protocol, setProtocol] = useState("tcp");
  const [portError, setPortError] = useState<unknown>(null);
  const portFields = portError instanceof ResourceError ? portError.fieldErrors : {};
  const disabled = !d.can.update;

  return (
    <div className="rdp2-stack">
      <Section title={t("Deploy")}>
        <form
          className="rdp2-form"
          onSubmit={(e) => {
            e.preventDefault();
            save({
              replicas: Number(replicas),
              command: command.trim() || null,
              zero_downtime: zero,
              healthcheck_cmd: check.trim() || null,
              healthcheck_interval_s: num(interval),
              healthcheck_retries: num(retries),
              cpu_limit_millis: num(cpu),
              mem_limit_mb: num(mem),
              cpu_reservation_millis: num(cpuR),
              mem_reservation_mb: num(memR),
              ...(capAdd !== (s.cap_add ?? "") ? { cap_add: capAdd.trim() || null } : {}),
              ...(capDrop !== (s.cap_drop ?? "") ? { cap_drop: capDrop.trim() || null } : {}),
              ...(sysctls !== (s.sysctls ?? "") ? { sysctls: sysctls.trim() || null } : {}),
              ...(groups !== (s.groups ?? "") ? { groups: groups.trim() || null } : {}),
            });
          }}
        >
          <div className="rdp2-grid2">
            <Field label={t("Replicas")} error={fields.replicas}>
              <Input type="number" min={0} value={replicas} onChange={(e) => setReplicas(e.target.value)} disabled={disabled} />
            </Field>
            <Field label={t("Command")} help={t("Overrides the image's command (optional)")} error={fields.command}>
              <Input value={command} onChange={(e) => setCommand(e.target.value)} disabled={disabled} />
            </Field>
          </div>
          {s.kind === "app" && (
            <label className="rdp2-row">
              <Switch checked={zero} onCheckedChange={setZero} disabled={disabled} /> {t("Zero downtime")}
            </label>
          )}
          {zero && !d.zero_downtime_possible && (
            <p className="rdp2-muted rdp2-small">
              {t("Zero downtime needs a health check and no host-mode port: updates stop the old task first.")}
            </p>
          )}
          <Field label={t("Health check")} help={t("Run in the container, e.g. wget -qO- http://127.0.0.1/health")} error={fields.healthcheck_cmd}>
            <Input value={check} onChange={(e) => setCheck(e.target.value)} disabled={disabled} />
          </Field>
          <div className="rdp2-grid2">
            <Field label={t("Every (seconds)")} error={fields.healthcheck_interval_s}>
              <Input type="number" value={interval} onChange={(e) => setInterval(e.target.value)} disabled={disabled} placeholder="10" />
            </Field>
            <Field label={t("Retries")} error={fields.healthcheck_retries}>
              <Input type="number" value={retries} onChange={(e) => setRetries(e.target.value)} disabled={disabled} placeholder="3" />
            </Field>
          </div>
          <h4 className="rdp2-subtitle">{t("Resources")}</h4>
          <div className="rdp2-grid2">
            <Field label={t("CPU limit (1000 = one core)")} error={fields.cpu_limit_millis}>
              <Input type="number" value={cpu} onChange={(e) => setCpu(e.target.value)} disabled={disabled} placeholder={t("no limit")} />
            </Field>
            <Field label={t("Memory limit (MB)")} error={fields.mem_limit_mb}>
              <Input type="number" value={mem} onChange={(e) => setMem(e.target.value)} disabled={disabled} placeholder={t("no limit")} />
            </Field>
            <Field label={t("CPU reservation")} error={fields.cpu_reservation_millis}>
              <Input type="number" value={cpuR} onChange={(e) => setCpuR(e.target.value)} disabled={disabled} />
            </Field>
            <Field label={t("Memory reservation (MB)")} error={fields.mem_reservation_mb}>
              <Input type="number" value={memR} onChange={(e) => setMemR(e.target.value)} disabled={disabled} />
            </Field>
          </div>
          {s.kind !== "compose" && (
            <>
              <h4 className="rdp2-subtitle">{t("Linux")}</h4>
              <p className="rdp2-muted rdp2-small">{t("Tini runs as the containers' init. Adding capabilities, groups or sysctls needs system.manage.")}</p>
              <div className="rdp2-grid2">
                <Field label={t("Add capabilities")} help="NET_ADMIN, SYS_TIME" error={fields.cap_add}>
                  <Input className="rdp2-mono" value={capAdd} onChange={(e) => setCapAdd(e.target.value)} disabled={disabled} />
                </Field>
                <Field label={t("Drop capabilities")} help="NET_RAW" error={fields.cap_drop}>
                  <Input className="rdp2-mono" value={capDrop} onChange={(e) => setCapDrop(e.target.value)} disabled={disabled} />
                </Field>
              </div>
              <Field label={t("Supplemental groups")} help="video, 1001" error={fields.groups}>
                <Input className="rdp2-mono" value={groups} onChange={(e) => setGroups(e.target.value)} disabled={disabled} />
              </Field>
              <Field label={t("Sysctls")} help={t("One key=value a line, e.g. net.ipv4.ip_forward=1")} error={fields.sysctls}>
                <Textarea className="rdp2-mono" rows={3} value={sysctls} onChange={(e) => setSysctls(e.target.value)} disabled={disabled} />
              </Field>
            </>
          )}
          {Object.keys(fields).length === 0 && <ErrorNote error={error} />}
          {d.can.update && (
            <div className="rdp2-row rdp2-end">
              <Button type="submit" disabled={busy}>
                {t("Save")}
              </Button>
            </div>
          )}
        </form>
      </Section>

      {d.can.ports.view && <Section title={t("Ports")} description={t("Published on every node of the Swarm. Prefer a domain for web apps.")}>
        <ul className="rdp2-list">
          {d.ports.map((p) => (
            <li key={p.id} className="rdp2-list-row">
              <code>
                {p.published} → {p.target}/{p.protocol}
              </code>
              {d.can.ports.delete && (
                <Button
                  size="sm"
                  variant="ghost"
                  onClick={async () => {
                    try {
                      await resources.remove("service_ports", p.id);
                      props.onSaved();
                    } catch (err) {
                      props.onError(err);
                    }
                  }}
                >
                  {t("Remove")}
                </Button>
              )}
            </li>
          ))}
        </ul>
        {d.can.ports.create && (
          <form
            className="rdp2-row rdp2-wrap"
            onSubmit={async (e) => {
              e.preventDefault();
              setPortError(null);
              try {
                await resources.create("service_ports", {
                  service_id: String(s.id),
                  published: Number(published),
                  target: Number(targetPort),
                  protocol,
                  mode: "ingress",
                });
                setPublished("");
                setTargetPort("");
                props.onSaved();
              } catch (err) {
                setPortError(err);
              }
            }}
          >
            <Input className="rdp2-narrow" type="number" placeholder={t("Published")} value={published} onChange={(e) => setPublished(e.target.value)} />
            <Input className="rdp2-narrow" type="number" placeholder={t("Container")} value={targetPort} onChange={(e) => setTargetPort(e.target.value)} />
            <NativeSelect className="rdp2-narrow" value={protocol} onChange={(e) => setProtocol(e.target.value)}>
              <option value="tcp">TCP</option>
              <option value="udp">UDP</option>
            </NativeSelect>
            <Button type="submit" variant="outline" disabled={!published || !targetPort}>
              + {t("Add port")}
            </Button>
          </form>
        )}
        {Object.entries(portFields).map(([field, list]) => (
          <p key={field} className="rdp2-error rdp2-small">
            {field}: {list.join(", ")}
          </p>
        ))}
        {Object.keys(portFields).length === 0 && <ErrorNote error={portError} />}
      </Section>}

      {(d.can.destroy || d.can.delete) && (
        <Section title={t("Danger zone")}>
          <div className="rdp2-row rdp2-wrap">
            {d.can.destroy && s.status !== "new" && (
              <Button variant="destructive" disabled={s.busy} onClick={props.onDestroy}>
                {t("Remove from Docker")}
              </Button>
            )}
            {d.can.delete && s.status === "new" && (
              <Button
                variant="destructive"
                onClick={async () => {
                  if (!window.confirm(t("Delete this service and its settings?"))) return;
                  try {
                    await resources.remove("services", s.id);
                    props.onDeleted();
                  } catch (err) {
                    props.onError(err);
                  }
                }}
              >
                {t("Delete service")}
              </Button>
            )}
          </div>
          <p className="rdp2-muted rdp2-small">
            {s.status === "new"
              ? t("Not on Docker: deleting removes its settings.")
              : t("Remove it from Docker first (volumes are kept), then it can be deleted.")}
          </p>
        </Section>
      )}
    </div>
  );
}
