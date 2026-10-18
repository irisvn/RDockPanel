// The dashboard: the server at a glance, then every project with its
// services as cards (status, kind, domain). Creating a project or a service
// asks for a name (and an image), nothing else.
import { useState } from "react";
import { type PageProps, useAdminQuery, useEvents } from "@rustcms/admin-sdk";
import { Button, Dialog, Input, Loading, ErrorState } from "@rustcms/admin-sdk/ui";
import { ErrorNote, Field, Sparkline, StatusDot, bytes } from "../lib/ui";
import { ResourceError, resources } from "../lib/resources";
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

/** A new app service: a name and an image (the rest has defaults). */
export function NewService(props: {
  t: (s: string) => string;
  project: Project;
  onDone: (name: string) => void;
  onClose: () => void;
}) {
  const t = props.t;
  const [name, setName] = useState("");
  const [image, setImage] = useState("");
  const [error, setError] = useState<unknown>(null);
  const [busy, setBusy] = useState(false);
  const fields = error instanceof ResourceError ? error.fieldErrors : {};
  return (
    <Dialog open onClose={props.onClose} title={`${t("Create service in")} ${props.project.name}`}>
      <form
        className="rdp2-form"
        onSubmit={async (e) => {
          e.preventDefault();
          setBusy(true);
          try {
            await resources.create("services", {
              project_id: String(props.project.id),
              name: name.trim(),
              image: image.trim(),
              replicas: 1,
              zero_downtime: true,
            });
            props.onDone(name.trim());
          } catch (err) {
            setError(err);
          } finally {
            setBusy(false);
          }
        }}
      >
        <Field label={t("Service name")} help={t("Lowercase letters, digits and dashes")} error={fields.name}>
          <Input autoFocus value={name} onChange={(e) => setName(e.target.value)} placeholder="web" />
        </Field>
        <Field label={t("Docker image")} help={t("e.g. nginx:1.27 or ghcr.io/org/app:v2")} error={fields.image}>
          <Input value={image} onChange={(e) => setImage(e.target.value)} placeholder="nginx:1.27-alpine" />
        </Field>
        {!fields.name && !fields.image && <ErrorNote error={error} />}
        <div className="rdp2-row rdp2-end">
          <Button type="button" variant="ghost" onClick={props.onClose}>
            {t("Cancel")}
          </Button>
          <Button type="submit" disabled={busy || !name.trim() || !image.trim()}>
            {t("Create")}
          </Button>
        </div>
      </form>
    </Dialog>
  );
}

export function Dashboard(props: PageProps<unknown>) {
  const t = props.t;
  const overview = useAdminQuery<Overview>(props, "/apps/overview", { refetchInterval: 5000 });
  const [creating, setCreating] = useState(false);
  const [adding, setAdding] = useState<Project | null>(null);
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
                <Button size="sm" variant="outline" onClick={() => setAdding(project)}>
                  + {t("Service")}
                </Button>
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
      {adding && (
        <NewService
          t={t}
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
