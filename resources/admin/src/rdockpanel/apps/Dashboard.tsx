// The dashboard: the server at a glance, then every project with its
// services as cards (status, kind, domain). Creating a project or a service
// asks for a name (and an image), nothing else.
import { useState } from "react";
import { type AdminApi, type PageProps, useAdminQuery, useEvents } from "@rustcms/admin-sdk";
import { Button, Dialog, Input, Loading, ErrorState, Textarea } from "@rustcms/admin-sdk/ui";
import { ErrorNote, Field, Sparkline, StatusDot, bytes } from "../lib/ui";
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
  const tiles: Array<[string, string, string, number[], number | undefined]> = last
    ? [
        [t("CPU"), `${last.cpu_percent.toFixed(1)}%`, "", samples.map((s) => s.cpu_percent), 100],
        [
          t("Memory"),
          `${((last.mem_used / Math.max(1, last.mem_total)) * 100).toFixed(1)}%`,
          `${bytes(last.mem_used)} / ${bytes(last.mem_total)}`,
          samples.map((s) => s.mem_used),
          last.mem_total,
        ],
        [
          t("Disk"),
          `${((last.disk_used / Math.max(1, last.disk_total)) * 100).toFixed(1)}%`,
          `${bytes(last.disk_used)} / ${bytes(last.disk_total)}`,
          samples.map((s) => s.disk_used),
          last.disk_total,
        ],
        [t("Network in"), `${bytes(last.net_rx_rate)}/s`, "", samples.map((s) => s.net_rx_rate), undefined],
        [t("Network out"), `${bytes(last.net_tx_rate)}/s`, "", samples.map((s) => s.net_tx_rate), undefined],
      ]
    : [];
  return (
    <div className="rdp2-tiles">
      {tiles.map(([label, value, sub, series, max]) => (
        <div key={label} className="rdp2-tile">
          <span className="rdp2-muted rdp2-small">{label}</span>
          <strong className="rdp2-tile-value">{value}</strong>
          <span className="rdp2-muted rdp2-small">{sub || " "}</span>
          <Sparkline values={series} max={max} />
        </div>
      ))}
    </div>
  );
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

/** The server's metrics, then the projects (with their permission). */
export function Dashboard(props: PageProps<unknown>) {
  if (!props.can("projects.view_any")) {
    return (
      <div className="rdp2-page">
        <ServerTiles {...props} />
        <Versions {...props} />
      </div>
    );
  }
  return (
    <>
      <Projects {...props} />
      <Versions {...props} />
    </>
  );
}

type VersionInfo = { rdockpanel: string; commit: string | null; rustcms: string };

/** What runs: RDockPanel's version (and commit), RustCMS Core's. */
function Versions(props: PageProps<unknown>) {
  const v = (props.data as { versions?: VersionInfo } | null)?.versions;
  if (!v) return null;
  return (
    <p className="rdp2-small rdp2-muted rdp2-versions">
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

function Projects(props: PageProps<unknown>) {
  const t = props.t;
  const overview = useAdminQuery<Overview>(props, "/apps/overview", { refetchInterval: 5000 });
  const [creating, setCreating] = useState(false);
  const [adding, setAdding] = useState<Project | null>(null);
  const [templating, setTemplating] = useState<Project | null>(null);
  const [error, setError] = useState<unknown>(null);
  const canCreateProject = props.can("projects.create");
  const canCreateService = props.can("services.create");

  return (
    <div className="rdp2-page">
      <ServerTiles {...props} />
      <div className="rdp2-head">
        <h2 className="rdp2-title">{t("Projects")}</h2>
        {canCreateProject && <Button onClick={() => setCreating(true)}>+ {t("New project")}</Button>}
      </div>
      <ErrorNote error={error} />
      {overview.isLoading && <Loading />}
      {overview.error && <ErrorState error={overview.error} />}
      {overview.data?.projects.length === 0 && (
        <div className="rdp2-empty">
          <p>{t("No project yet. A project groups the services of one app: its web, its database, …")}</p>
          {canCreateProject && <Button onClick={() => setCreating(true)}>+ {t("Create your first project")}</Button>}
        </div>
      )}
      {overview.data?.projects.map((project) => (
        <section key={project.id} className="rdp2-project">
          <header className="rdp2-project-head">
            <button type="button" className="rdp2-link rdp2-project-name" onClick={() => props.navigate(`/p/dashboard/${project.name}`)}>
              {project.name}
            </button>
            <div className="rdp2-row">
              {canCreateService && (
                <>
                  <Button size="sm" variant="outline" onClick={() => setTemplating(project)}>
                    {t("Templates")}
                  </Button>
                  <Button size="sm" variant="outline" onClick={() => setAdding(project)}>
                    + {t("Service")}
                  </Button>
                </>
              )}
              {project.services.length === 0 && props.can("projects.delete") && (
                <Button
                  size="sm"
                  variant="ghost"
                  onClick={async () => {
                    if (!window.confirm(t("Delete this empty project?"))) return;
                    try {
                      await resources.remove("projects", project.id);
                      overview.refetch();
                    } catch (err) {
                      setError(err);
                    }
                  }}
                >
                  {t("Delete")}
                </Button>
              )}
            </div>
          </header>
          <div className="rdp2-cards">
            {project.services.map((s) => (
              <button
                key={s.id}
                type="button"
                className="rdp2-card"
                onClick={() => props.navigate(`/p/dashboard/${project.name}/${s.name}`)}
              >
                <span className="rdp2-row">
                  <StatusDot status={s.status} busy={s.busy} />
                  <strong>{s.name}</strong>
                </span>
                <span className="rdp2-muted rdp2-small">{t(s.kind)}</span>
                <span className="rdp2-muted rdp2-small rdp2-ellipsis">{s.domain ?? s.image}</span>
              </button>
            ))}
            {project.services.length === 0 && (
              <p className="rdp2-muted rdp2-small">{t("No service: add one with + Service.")}</p>
            )}
          </div>
        </section>
      ))}
      {creating && (
        <NewProject
          t={t}
          onClose={() => setCreating(false)}
          onDone={() => {
            setCreating(false);
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
            if (made.warnings.length) window.alert(made.warnings.join("\n"));
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
    </div>
  );
}
