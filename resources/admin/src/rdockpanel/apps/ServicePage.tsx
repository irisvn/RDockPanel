// A service's page: its header (status, Deploy and the other actions, always
// visible) and its tabs, in the URL (`/p/dashboard/<project>/<service>/<tab>`).
// Saving never deploys: the page says when a change waits for Deploy.
import { type ReactNode, useEffect, useRef, useState } from "react";
import { type PageProps, useAdminQuery, useEvents } from "@rustcms/admin-sdk";
import { Alert, Button, Checkbox, Dialog, ErrorState, Input, Loading, NativeSelect, Switch, Textarea } from "@rustcms/admin-sdk/ui";
import { CopyButton, ErrorNote, Facts, Field, Section, StatusBadge, Tabs, when } from "../lib/ui";
import { ResourceError, resources } from "../lib/resources";
import type { Deployment, ServiceDetail } from "./types";

type Props = PageProps<unknown> & { project: string; serviceId: number; tab: string };
type T = (s: string) => string;

const TABS = ["overview", "credentials", "deployments", "environment", "domains", "source", "storage", "advanced"] as const;

export function ServicePage(props: Props) {
  const t = props.t;
  const detail = useAdminQuery<ServiceDetail>(props, `/apps/services/${props.serviceId}`, { refetchInterval: 3000 });
  const [error, setError] = useState<unknown>(null);
  const [notice, setNotice] = useState<string | null>(null);
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
    setNotice(t("Saved. Deploy to apply it."));
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
          </>
        )}
        {primary && running && (
          <a className="rdp2-button-link" href={`${primary.https ? "https" : "http"}://${primary.host}${primary.path}`} target="_blank" rel="noreferrer">
            {t("Open")} ↗
          </a>
        )}
      </div>
      {d.pending && s.status !== "new" && <Alert variant="warning">{t("The configuration changed since the last deployment: deploy to apply it.")}</Alert>}
      {notice && <Alert>{notice}</Alert>}
      <ErrorNote error={error} />
      <Tabs
        value={tab}
        onSelect={(id) => {
          setNotice(null);
          props.navigate(`${base}/${id}`);
        }}
        tabs={[
          { id: "overview", label: t("Overview") },
          ...(d.database ? [{ id: "credentials", label: t("Credentials") }] : []),
          { id: "deployments", label: t("Deployments") },
          { id: "environment", label: t("Environment") },
          { id: "domains", label: t("Domains") },
          { id: "source", label: t("Source") },
          { id: "storage", label: t("Storage") },
          { id: "advanced", label: t("Advanced") },
        ]}
      />
      <div className="rdp2-tab-body">
        {tab === "overview" && <OverviewTab {...props} d={d} />}
        {tab === "credentials" && d.database && <CredentialsTab {...props} d={d} />}
        {tab === "deployments" && <DeploymentsTab {...props} d={d} onError={setError} refresh={refresh} />}
        {tab === "environment" && <EnvironmentTab {...props} d={d} onSaved={saved} />}
        {tab === "domains" && !d.can.domains.view && <p className="rdp2-muted">{t("You may not see domains.")}</p>}
        {tab === "domains" && d.can.domains.view && <DomainsTab t={t} d={d} onSaved={saved} onError={setError} />}
        {tab === "source" && <SourceTab t={t} d={d} onSaved={saved} />}
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
      </div>
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
      {d.latest && (
        <Section
          title={`${t("Deployment")} #${d.latest.id} · ${t(d.latest.trigger)}`}
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
              <td>{t(row.trigger)}</td>
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
  const [adding, setAdding] = useState(false);
  const [host, setHost] = useState("");
  const [path, setPath] = useState("/");
  const [port, setPort] = useState("80");
  const [https, setHttps] = useState(true);
  const [error, setError] = useState<unknown>(null);
  const fields = error instanceof ResourceError ? error.fieldErrors : {};
  return (
    <Section
      title={t("Domains")}
      description={t("Traefik routes these to the service's port. HTTPS redirects HTTP to it.")}
      actions={
        d.can.domains.create && (
          <>
            {d.auto_domain && !d.domains.some((x) => x.host === d.auto_domain) && (
              <Button
                variant="outline"
                onClick={() => {
                  setHost(d.auto_domain ?? "");
                  setHttps(!(d.auto_domain ?? "").endsWith(".localhost"));
                  setAdding(true);
                }}
              >
                + {t("Generated domain")}
              </Button>
            )}
            <Button onClick={() => setAdding(true)}>+ {t("Add domain")}</Button>
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
              <span className="rdp2-muted">→ :{domain.port}</span>
            </span>
            {d.can.domains.delete && (
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
          </li>
        ))}
      </ul>
      {adding && (
        <Dialog open onClose={() => setAdding(false)} title={t("Add domain")}>
          <form
            className="rdp2-form"
            onSubmit={async (e) => {
              e.preventDefault();
              setError(null);
              try {
                await resources.create("service_domains", {
                  service_id: String(d.service.id),
                  host: host.trim().toLowerCase(),
                  path: path.trim() || "/",
                  port: Number(port),
                  https,
                });
                setAdding(false);
                setHost("");
                props.onSaved();
              } catch (err) {
                setError(err);
              }
            }}
          >
            <Field label={t("Host")} help={t("e.g. app.example.com (DNS pointing to this server)")} error={fields.host}>
              <Input autoFocus value={host} onChange={(e) => setHost(e.target.value)} placeholder="app.example.com" />
            </Field>
            <div className="rdp2-grid2">
              <Field label={t("Path")} error={fields.path}>
                <Input value={path} onChange={(e) => setPath(e.target.value)} />
              </Field>
              <Field label={t("Container port")} error={fields.port}>
                <Input type="number" value={port} onChange={(e) => setPort(e.target.value)} />
              </Field>
            </div>
            <label className="rdp2-row">
              <Switch checked={https} onCheckedChange={setHttps} /> {t("HTTPS")}
            </label>
            {Object.keys(fields).length === 0 && <ErrorNote error={error} />}
            <div className="rdp2-row rdp2-end">
              <Button type="button" variant="ghost" onClick={() => setAdding(false)}>
                {t("Cancel")}
              </Button>
              <Button type="submit" disabled={!host.trim()}>
                {t("Add")}
              </Button>
            </div>
          </form>
        </Dialog>
      )}
    </Section>
  );
}

/** Saves `data` on the service (the resource's own validation). */
function useServiceSave(id: number, onSaved: () => void) {
  const [error, setError] = useState<unknown>(null);
  const [busy, setBusy] = useState(false);
  const save = async (data: object) => {
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

function SourceTab(props: { t: T; d: ServiceDetail; onSaved: () => void }) {
  const { d, t } = props;
  const s = d.service;
  const [image, setImage] = useState(s.image);
  const [user, setUser] = useState(s.registry_username ?? "");
  const [password, setPassword] = useState("");
  const { save, error, busy, fields } = useServiceSave(s.id, () => {
    setPassword("");
    props.onSaved();
  });
  return (
    <Section title={t("Docker image")} description={t("Pulled and pinned to its digest at each deploy.")}>
      <form
        className="rdp2-form"
        onSubmit={(e) => {
          e.preventDefault();
          const data: Record<string, unknown> = { image: image.trim(), registry_username: user.trim() || null };
          if (password) data.registry_password = { replace: password };
          save(data);
        }}
      >
        <Field label={t("Image")} error={fields.image}>
          <Input value={image} onChange={(e) => setImage(e.target.value)} disabled={!d.can.update} />
        </Field>
        <div className="rdp2-grid2">
          <Field label={t("Registry user")} help={t("For a private image")} error={fields.registry_username}>
            <Input value={user} onChange={(e) => setUser(e.target.value)} disabled={!d.can.update} />
          </Field>
          <Field
            label={t("Registry password or token")}
            help={s.registry_password_set ? t("Set: leave empty to keep it") : t("Not set")}
            error={fields.registry_password}
          >
            <Input type="password" value={password} onChange={(e) => setPassword(e.target.value)} disabled={!d.can.update} autoComplete="new-password" />
          </Field>
        </div>
        {Object.keys(fields).length === 0 && <ErrorNote error={error} />}
        {d.can.update && (
          <div className="rdp2-row rdp2-end">
            {s.registry_password_set && (
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
