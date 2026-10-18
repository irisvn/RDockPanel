// The apps interface (`/admin/p/dashboard`), like EasyPanel's: the
// dashboard (server and projects), then a project, then a service with its
// tabs, by sub-path: `<project>`, `<project>/<service>`, `<project>/<service>/<tab>`.
// A React page on the panel's React, React Query and UI kit (host API v2);
// `npm run build` in resources/admin, then commit the built files.
import { type PageProps, registerPage, useAdminQuery } from "@rustcms/admin-sdk";
import { Button, ErrorState, Loading } from "@rustcms/admin-sdk/ui";
import { useState } from "react";
import { Dashboard, NewService } from "./apps/Dashboard";
import { ServicePage } from "./apps/ServicePage";
import { Templates } from "./apps/Templates";
import { Settings } from "./settings/Settings";
import type { Overview } from "./apps/types";
import { StatusDot } from "./lib/ui";
import { bindSession } from "./lib/resources";
import "./lib/i18n";
import "./dashboard.css";

function ProjectPage(props: PageProps<unknown> & { project: string }) {
  const t = props.t;
  const overview = useAdminQuery<Overview>(props, "/apps/overview", { refetchInterval: 5000, refetchOnMount: "always" });
  const [adding, setAdding] = useState(false);
  const [templating, setTemplating] = useState(false);
  if (overview.isLoading) return <Loading />;
  if (overview.error) return <ErrorState error={overview.error} />;
  const project = overview.data?.projects.find((p) => p.name === props.project);
  if (!project) return <p className="rdp2-muted">{t("This project does not exist.")}</p>;
  return (
    <div className="rdp2-page">
      <nav className="rdp2-crumbs">
        <button type="button" className="rdp2-link" onClick={() => props.navigate("/p/dashboard")}>
          {t("Dashboard")}
        </button>
        <span>/</span>
        <strong>{project.name}</strong>
      </nav>
      <div className="rdp2-head">
        <h2 className="rdp2-title">{project.name}</h2>
        {props.can("services.create") && (
          <div className="rdp2-row">
            <Button variant="outline" onClick={() => setTemplating(true)}>
              {t("Templates")}
            </Button>
            <Button onClick={() => setAdding(true)}>+ {t("Service")}</Button>
          </div>
        )}
      </div>
      {project.description && <p className="rdp2-muted">{project.description}</p>}
      <div className="rdp2-cards">
        {project.services.map((s) => (
          <button key={s.id} type="button" className="rdp2-card" onClick={() => props.navigate(`/p/dashboard/${project.name}/${s.name}`)}>
            <span className="rdp2-row">
              <StatusDot status={s.status} busy={s.busy} />
              <strong>{s.name}</strong>
            </span>
            <span className="rdp2-muted rdp2-small">{t(s.kind)}</span>
            <span className="rdp2-muted rdp2-small rdp2-ellipsis">{s.domain ?? s.image}</span>
          </button>
        ))}
      </div>
      {project.services.length === 0 && <p className="rdp2-muted">{t("No service: add one with + Service.")}</p>}
      {templating && (
        <Templates
          {...props}
          project={project}
          onClose={() => setTemplating(false)}
          onDone={(made) => {
            setTemplating(false);
            if (made.warnings.length) window.alert(made.warnings.join("\n"));
            overview.refetch();
          }}
        />
      )}
      {adding && (
        <NewService
          t={t}
          api={props.api}
          project={project}
          onClose={() => setAdding(false)}
          onDone={(name) => {
            setAdding(false);
            props.navigate(`/p/dashboard/${project.name}/${name}`);
          }}
        />
      )}
    </div>
  );
}

function ServiceRoute(props: PageProps<unknown> & { project: string; service: string; tab: string }) {
  // Fresh: the service may have just been created.
  const overview = useAdminQuery<Overview>(props, "/apps/overview", { refetchOnMount: "always" });
  if (overview.isLoading) return <Loading />;
  if (overview.error) return <ErrorState error={overview.error} />;
  const service = overview.data?.projects
    .find((p) => p.name === props.project)
    ?.services.find((s) => s.name === props.service);
  if (!service && overview.isFetching) return <Loading />;
  if (!service) return <p className="rdp2-muted">{props.t("This service does not exist.")}</p>;
  // Keyed: another service gets fresh forms.
  return <ServicePage key={service.id} {...props} serviceId={service.id} />;
}

function Apps(props: PageProps<{ user?: string | number }>) {
  bindSession(String(props.data.user ?? ""));
  const [project, service, tab] = props.path ? props.path.split("/") : [];
  if (project && service) return <ServiceRoute {...props} project={project} service={service} tab={tab ?? "overview"} />;
  if (project) return <ProjectPage {...props} project={project} />;
  return <Dashboard {...props} />;
}

registerPage("rdockpanel/dashboard", Apps as (p: PageProps<unknown>) => ReturnType<typeof Apps>);

function SettingsRoute(props: PageProps<{ user?: string | number }>) {
  bindSession(String(props.data.user ?? ""));
  return <Settings {...props} />;
}

registerPage("rdockpanel/settings", SettingsRoute as (p: PageProps<unknown>) => ReturnType<typeof SettingsRoute>);
