// The dashboard: the server at a glance, then every project with its
// services as cards (status, kind, domain). Creating a project or a service
// asks for a name (and an image), nothing else.
import { type ReactNode, useState } from "react";
import { type AdminApi, type PageProps, useAdminQuery, useEvents } from "@rustcms/admin-sdk";
import { Alert, Button, Dialog, Input, Loading, ErrorState, Textarea } from "@rustcms/admin-sdk/ui";
import { EmptyState, ErrorNote, Field, Metric, NavLink, Page, RowActions, Section, Status, useAsk, useConfirm } from "../lib/ui";
import { percent, size } from "../lib/format";
import { ResourceError, resources } from "../lib/resources";
import { Templates } from "./Templates";
import type { Overview, Project } from "./types";

type Sample = {
  cpu_percent: number;
  mem_used: number;
  mem_total: number;
  net_rx_rate: number;
  net_tx_rate: number;
  disk_used: number;
  disk_total: number;
};

function ServerTiles(props: PageProps<unknown>) {
  return props.can("server.view") ? <LiveTiles {...props} /> : null;
}

function LiveTiles(props: PageProps<unknown>) {
  const [samples, setSamples] = useState<Sample[]>([]);
  useEvents(props, "/server/live", {
    onEvent(name, data) {
      if (name === "history") setSamples(((data as { samples: Sample[] }).samples ?? []).slice(-60));
      else if (name === "sample") setSamples((s) => [...s.slice(-59), data as Sample]);
    },
  });
  const last = samples[samples.length - 1];
  const t = props.t;
  const locale = props.locale;
  if (!last) return null;
  const share = (used: number, total: number) => percent((used / Math.max(1, total)) * 100, locale);
  return (
    <div className="rdp2-metrics">
      <Metric label={t("CPU")} value={percent(last.cpu_percent, locale)} values={samples.map((s) => s.cpu_percent)} max={100} />
      <Metric
        label={t("Memory")}
        value={share(last.mem_used, last.mem_total)}
        sub={`${size(last.mem_used, locale)} / ${size(last.mem_total, locale)}`}
        values={samples.map((s) => s.mem_used)}
        max={last.mem_total}
      />
      <Metric
        label={t("Disk")}
        value={share(last.disk_used, last.disk_total)}
        sub={`${size(last.disk_used, locale)} / ${size(last.disk_total, locale)}`}
        values={samples.map((s) => s.disk_used)}
        max={last.disk_total}
      />
      <Metric label={t("Network in")} value={`${size(last.net_rx_rate, locale)}/s`} values={samples.map((s) => s.net_rx_rate)} />
      <Metric label={t("Network out")} value={`${size(last.net_tx_rate, locale)}/s`} values={samples.map((s) => s.net_tx_rate)} />
    </div>
  );
}

/** A project's services as cards (the Dashboard's one use of cards): name and
 * status, kind, domain (or image); each opens its service. */
export function ServiceCards(props: Pick<PageProps<unknown>, "t" | "navigate"> & { project: Project }) {
  const t = props.t;
  if (props.project.services.length === 0) {
    return <EmptyState title={t("No service yet")} hint={t("Add one, or start from a template.")} />;
  }
  return (
    <div className="rdp2-cards">
      {props.project.services.map((s) => (
        <button key={s.id} type="button" className="rdp2-card" onClick={() => props.navigate(`/p/dashboard/${props.project.name}/${s.name}`)}>
          <span className="rdp2-card-top">
            <strong className="rdp2-ellipsis">{s.name}</strong>
            <Status kind="service" status={s.status} busy={s.busy} t={t} />
          </span>
          <span className="rdp2-muted rdp2-small">{t(s.kind)}</span>
          <span className="rdp2-mono rdp2-muted rdp2-ellipsis">{s.domain ?? s.image}</span>
        </button>
      ))}
    </div>
  );
}

/** A template's warnings once it made its services, in full (a dialog: a
 * toast would cut long ones). */
export function templateWarnings(
  props: Pick<PageProps<unknown>, "toast" | "t">,
  warnings: string[],
  ask: (o: { title: string; body: ReactNode; action: string; notice: boolean }) => Promise<boolean>,
) {
  if (!warnings.length) return props.toast.success(props.t("Created"));
  void ask({
    title: props.t("Created, with warnings"),
    body: (
      <ul className="rdp2-plain-list">
        {warnings.map((w) => (
          <li key={w}>{w}</li>
        ))}
      </ul>
    ),
    action: props.t("OK"),
    notice: true,
  });
}

function NewProject(props: { t: (s: string) => string; onDone: (name: string) => void; onClose: () => void }) {
  const [name, setName] = useState("");
  const [error, setError] = useState<unknown>(null);
  const [busy, setBusy] = useState(false);
  const fields = error instanceof ResourceError ? error.fieldErrors : {};
  return (
    <Dialog open onClose={props.onClose} title={props.t("Create project")}>
      <form
        className="rdp2-form"
        onSubmit={async (e) => {
          e.preventDefault();
          setBusy(true);
          try {
            await resources.create("projects", { name: name.trim() });
            props.onDone(name.trim());
          } catch (err) {
            setError(err);
          } finally {
            setBusy(false);
          }
        }}
      >
        <Field label={props.t("Name")} help={props.t("Lowercase letters, digits and dashes")} error={fields.name}>
          <Input autoFocus value={name} onChange={(e) => setName(e.target.value)} placeholder="shop" />
        </Field>
        {!fields.name && <ErrorNote error={error} />}
        <div className="rdp2-row rdp2-end">
          <Button type="button" variant="ghost" onClick={props.onClose}>
            {props.t("Cancel")}
          </Button>
          <Button type="submit" disabled={busy || !name.trim()}>
            {props.t("Create")}
          </Button>
        </div>
      </form>
    </Dialog>
  );
}

const KINDS: Array<[string, string]> = [
  ["app", "App (Docker image)"],
  ["postgres", "PostgreSQL"],
  ["mysql", "MySQL"],
  ["mariadb", "MariaDB"],
  ["mongo", "MongoDB"],
  ["redis", "Redis"],
  ["compose", "Compose"],
];

const COMPOSE_EXAMPLE = "services:\n  web:\n    image: nginx:1.27-alpine\n";

/** A new service: a name, and an image for an app; a database needs only
 * its name (its credentials are generated). */
export function NewService(props: {
  t: (s: string) => string;
  api: AdminApi;
  /** Compose services: the panel's administrators only (`system.manage`). */
  canCompose?: boolean;
  project: Project;
  onDone: (name: string) => void;
  onClose: () => void;
}) {
  const t = props.t;
  const [kind, setKind] = useState("app");
  const [name, setName] = useState("");
  const [image, setImage] = useState("");
  const [compose, setCompose] = useState(COMPOSE_EXAMPLE);
  const [error, setError] = useState<unknown>(null);
  const [busy, setBusy] = useState(false);
  const fields = (error as { fieldErrors?: Record<string, string[]> } | null)?.fieldErrors ?? {};
  const app = kind === "app";
  const isCompose = kind === "compose";
  return (
    <Dialog open onClose={props.onClose} title={`${t("Create service in")} ${props.project.name}`}>
      <form
        className="rdp2-form"
        onSubmit={async (e) => {
          e.preventDefault();
          setBusy(true);
          setError(null);
          try {
            await props.api.post(`/apps/projects/${props.project.id}/services`, {
              name: name.trim(),
              kind,
              image: isCompose ? undefined : image.trim() || undefined,
              compose: isCompose ? compose : undefined,
            });
            props.onDone(name.trim());
          } catch (err) {
            setError(err);
          } finally {
            setBusy(false);
          }
        }}
      >
        <Field label={t("Type")}>
          <div className="rdp2-kinds">
            {KINDS.filter(([id]) => id !== "compose" || props.canCompose).map(([id, label]) => (
              <button
                key={id}
                type="button"
                className={id === kind ? "rdp2-kind rdp2-kind-active" : "rdp2-kind"}
                onClick={() => {
                  // Another kind: the image typed for the last one does not fit.
                  if (id !== kind) setImage("");
                  setKind(id);
                }}
              >
                {t(label)}
              </button>
            ))}
          </div>
        </Field>
        <Field label={t("Service name")} help={t("Lowercase letters, digits and dashes")} error={fields.name}>
          <Input autoFocus value={name} onChange={(e) => setName(e.target.value)} placeholder={app ? "web" : "db"} />
        </Field>
        {isCompose ? (
          <Field label={t("Compose file")} help={t("docker-compose.yml; a repository can be set later on its Source tab")} error={fields.compose}>
            <Textarea className="rdp2-env" rows={10} value={compose} onChange={(e) => setCompose(e.target.value)} />
          </Field>
        ) : (
          <Field
            label={app ? t("Docker image") : t("Image (optional)")}
            help={app ? t("e.g. nginx:1.27 or ghcr.io/org/app:v2") : t("The engine's official image by default; credentials are generated.")}
            error={fields.image}
          >
            <Input value={image} onChange={(e) => setImage(e.target.value)} placeholder={app ? "nginx:1.27-alpine" : ""} />
          </Field>
        )}
        {!fields.name && !fields.image && !fields.compose && <ErrorNote error={error} />}
        <div className="rdp2-row rdp2-end">
          <Button type="button" variant="ghost" onClick={props.onClose}>
            {t("Cancel")}
          </Button>
          <Button type="submit" disabled={busy || !name.trim() || (app && !image.trim())}>
            {t("Create")}
          </Button>
        </div>
      </form>
    </Dialog>
  );
}

/** A newer release, for the panel's administrators: its link, and a way to
 * hide it until the next one (each their own). */
function UpdateNotice(props: PageProps<unknown>) {
  const t = props.t;
  const notice = useAdminQuery<{ notice: { version: string } | null }>(props, "/apps/system/version/notice");
  const [hidden, setHidden] = useState(false);
  const version = notice.data?.notice?.version;
  if (!version || hidden) return null;
  return (
    <Alert variant="warning">
      <span className="rdp2-row rdp2-wrap">
        <span>{t("RDockPanel {v} is out.").replace("{v}", version)}</span>
        <NavLink to="/p/settings/version" navigate={props.navigate} className="rdp2-link rdp2-strong">
          {t("See what changed")}
        </NavLink>
        <Button
          size="sm"
          variant="ghost"
          onClick={async () => {
            setHidden(true);
            try {
              await props.api.post("/apps/system/version/dismiss", { version });
            } catch {
              // Shown again next time: nothing lost.
            }
          }}
        >
          {t("Hide")}
        </Button>
      </span>
    </Alert>
  );
}

/** The server's metrics, then the projects (with their permission). */
export function Dashboard(props: PageProps<unknown>) {
  const t = props.t;
  const [creating, setCreating] = useState(false);
  const canCreateProject = props.can("projects.create");
  return (
    <Page
      title={props.title}
      eyebrow={t("Workspace")}
      heading={t("Dashboard")}
      description={t("Your server and the projects running on it.")}
      actions={canCreateProject && <Button onClick={() => setCreating(true)}>{t("New project")}</Button>}
    >
      {props.can("system.manage") && <UpdateNotice {...props} />}
      <ServerTiles {...props} />
      <Projects {...props} creating={creating} setCreating={setCreating} />
      <Versions {...props} />
    </Page>
  );
}

type VersionInfo = { rdockpanel: string; commit: string | null; rustcms: string };

/** What runs: RDockPanel's version (and commit), RustCMS Core's. */
function Versions(props: PageProps<unknown>) {
  const v = (props.data as { versions?: VersionInfo } | null)?.versions;
  if (!v) return null;
  return (
    <p className="rdp2-versions">
      RDockPanel {v.rdockpanel}
      {v.commit && (
        <>
          {" "}
          (
          <a className="rdp2-link" href={`https://github.com/irisvn/rdockpanel/commit/${v.commit}`} target="_blank" rel="noreferrer">
            {v.commit.slice(0, 7)}
          </a>
          )
        </>
      )}{" "}
      · RustCMS Core {v.rustcms}
    </p>
  );
}

function Projects(props: PageProps<unknown> & { creating: boolean; setCreating: (on: boolean) => void }) {
  const t = props.t;
  const overview = useAdminQuery<Overview>(props, "/apps/overview", { refetchInterval: 5000 });
  const [adding, setAdding] = useState<Project | null>(null);
  const [templating, setTemplating] = useState<Project | null>(null);
  const [error, setError] = useState<unknown>(null);
  const [confirm, confirmDialog] = useConfirm(t);
  const ask = useAsk();
  const canCreateProject = props.can("projects.create");
  const remove = async (project: Project) => {
    const ok = await confirm({
      title: t("Delete the project {name}?").replace("{name}", project.name),
      body: t("It has no service; its members are removed with it."),
      action: t("Delete project"),
      destructive: true,
      typed: project.name,
    });
    if (!ok) return;
    try {
      await resources.remove("projects", project.id);
      props.toast.success(t("Project deleted"));
      overview.refetch();
    } catch (err) {
      setError(err);
    }
  };

  return (
    <>
      <ErrorNote error={error} />
      {overview.isLoading && <Loading />}
      {overview.error && <ErrorState error={overview.error} />}
      {overview.data?.projects.length === 0 && (
        <Section title={t("Projects")}>
          <EmptyState
            title={t("No project yet")}
            hint={t("A project groups the services of one app: its web, its database, …")}
            action={canCreateProject && <Button onClick={() => props.setCreating(true)}>{t("New project")}</Button>}
          />
        </Section>
      )}
      {overview.data?.projects.map((project) => (
        <Section
          key={project.id}
          title={
            <NavLink to={`/p/dashboard/${project.name}`} navigate={props.navigate}>
              {project.name}
            </NavLink>
          }
          description={
            <>
              {t("{n} services").replace("{n}", String(project.services.length))}
              {project.role && <> · {t(project.role)}</>}
            </>
          }
          actions={
            <>
              {project.can.create_service && (
                <>
                  <Button size="sm" variant="outline" onClick={() => setTemplating(project)}>
                    {t("Templates")}
                  </Button>
                  <Button size="sm" variant="outline" onClick={() => setAdding(project)}>
                    {t("Add service")}
                  </Button>
                </>
              )}
              {project.services.length === 0 && project.can.delete && (
                <RowActions
                  label={t("Actions for {host}").replace("{host}", project.name)}
                  menu={[{ label: t("Delete project…"), destructive: true, onSelect: () => void remove(project) }]}
                />
              )}
            </>
          }
        >
          <ServiceCards t={t} navigate={props.navigate} project={project} />
        </Section>
      ))}
      {props.creating && (
        <NewProject
          t={t}
          onClose={() => props.setCreating(false)}
          onDone={() => {
            props.setCreating(false);
            props.toast.success(t("Project created"));
            overview.refetch();
          }}
        />
      )}
      {templating && (
        <Templates
          {...props}
          project={templating}
          onClose={() => setTemplating(null)}
          onDone={(made) => {
            setTemplating(null);
            templateWarnings(props, made.warnings, ask);
            props.navigate(`/p/dashboard/${made.project}`);
          }}
        />
      )}
      {adding && (
        <NewService
          t={t}
          api={props.api}
          canCompose={props.can("system.manage")}
          project={adding}
          onClose={() => setAdding(null)}
          onDone={(name) => {
            const project = adding.name;
            setAdding(null);
            props.navigate(`/p/dashboard/${project}/${name}`);
          }}
        />
      )}
      {confirmDialog}
    </>
  );
}
