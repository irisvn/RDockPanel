// The apps interface (`/admin/p/dashboard`), like EasyPanel's: the
// dashboard (server and projects), then a project, then a service with its
// tabs, by sub-path: `<project>`, `<project>/<service>`, `<project>/<service>/<tab>`.
// A React page on the panel's React, React Query and UI kit (host API v2);
// `npm run build` in resources/admin, then commit the built files.
import { type PageProps, registerPage, registerSlot } from "@rustcms/admin-sdk";
import { useAdminQuery } from "./lib/query";
import { Button, ErrorState, Loading } from "@rustcms/admin-sdk/ui";
import { useEffect, useState } from "react";
import { Dashboard, NewService, ServiceCards, templateWarnings } from "./apps/Dashboard";
import { ServicePage } from "./apps/ServicePage";
import { Templates } from "./apps/Templates";
import { Settings } from "./settings/Settings";
import { Actions } from "./actions/Actions";
import { Domains } from "./domains/Domains";
import { Containers } from "./docker/Containers";
import { Images } from "./docker/Images";
import { Networks, Volumes } from "./docker/VolumesNetworks";
import { ServiceLive } from "./docker/ServiceLive";
import { Palette } from "./lib/Palette";
import { Members } from "./apps/Members";
import type { Overview } from "./apps/types";
import { ConfirmProvider, EmptyState, Page, Section, useAsk } from "./lib/ui";
import { bindResources } from "./lib/resources";
import "./lib/i18n";
import "./dashboard.css";

function ProjectPage(props: PageProps<unknown> & { project: string }) {
  const t = props.t;
  const overview = useAdminQuery<Overview>(props, "/apps/overview", { refetchInterval: 5000, refetchOnMount: "always" });
  const [adding, setAdding] = useState(false);
  const [templating, setTemplating] = useState(false);
  const [members, setMembers] = useState(false);
  const ask = useAsk();
  if (overview.isLoading) return <Loading />;
  if (overview.error) return <ErrorState error={overview.error} />;
  const project = overview.data?.projects.find((p) => p.name === props.project);
  if (!project) return <EmptyState title={t("This project does not exist.")} />;
  return (
    <Page
      title={props.title}
      eyebrow={t("Project")}
      heading={project.name}
      description={project.description || undefined}
      actions={
        <>
          {project.can.members && (
            <Button variant="outline" onClick={() => setMembers(true)}>
              {t("Members")}
            </Button>
          )}
          {project.can.create_service && (
            <>
              <Button variant="outline" onClick={() => setTemplating(true)}>
                {t("Templates")}
              </Button>
              <Button onClick={() => setAdding(true)}>{t("Add service")}</Button>
            </>
          )}
        </>
      }
    >
      <Section title={t("Services")} description={project.role ? t(project.role) : undefined}>
        <ServiceCards t={t} navigate={props.navigate} project={project} />
      </Section>
      {members && <Members {...props} project={project} onClose={() => setMembers(false)} />}
      {templating && (
        <Templates
          {...props}
          project={project}
          onClose={() => setTemplating(false)}
          onDone={(made) => {
            setTemplating(false);
            templateWarnings(props, made.warnings, ask);
            overview.refetch();
          }}
        />
      )}
      {adding && (
        <NewService
          t={t}
          api={props.api}
          canCompose={props.can("system.manage")}
          project={project}
          onClose={() => setAdding(false)}
          onDone={(name) => {
            setAdding(false);
            props.navigate(`/p/dashboard/${project.name}/${name}`);
          }}
        />
      )}
    </Page>
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
  if (!service) return <EmptyState title={props.t("This service does not exist.")} />;
  // Keyed: another service gets fresh forms.
  return <ServicePage key={service.id} {...props} serviceId={service.id} />;
}

function Apps(props: PageProps<{ user?: string | number }>) {
  bindResources(props.resources);
  const [project, service, tab] = props.path ? props.path.split("/") : [];
  // A project or a service has its own header (status, actions):
  // no "Dashboard" heading above it.
  const { title } = props;
  useEffect(() => {
    if (project) title(null);
  }, [title, project, service, tab]);
  const page =
    project && service ? (
      <ServiceRoute {...props} project={project} service={service} tab={tab ?? "overview"} />
    ) : project ? (
      <ProjectPage {...props} project={project} />
    ) : (
      <Dashboard {...props} />
    );
  return (
    <ConfirmProvider t={props.t} path={props.path}>
      {page}
      <Palette {...props} />
    </ConfirmProvider>
  );
}

registerPage("rdockpanel/dashboard", Apps as (p: PageProps<unknown>) => ReturnType<typeof Apps>);

function SettingsRoute(props: PageProps<{ user?: string | number }>) {
  bindResources(props.resources);
  return (
    <ConfirmProvider t={props.t} path={props.path}>
      <Settings {...props} />
      <Palette {...props} />
    </ConfirmProvider>
  );
}

registerPage("rdockpanel/settings", SettingsRoute as (p: PageProps<unknown>) => ReturnType<typeof SettingsRoute>);

function ActionsRoute(props: PageProps<{ user?: string | number }>) {
  bindResources(props.resources);
  return (
    <ConfirmProvider t={props.t} path={props.path}>
      <Actions {...props} />
      <Palette {...props} />
    </ConfirmProvider>
  );
}

registerPage("rdockpanel/actions", ActionsRoute as (p: PageProps<unknown>) => ReturnType<typeof ActionsRoute>);

function DomainsRoute(props: PageProps<{ user?: string | number }>) {
  bindResources(props.resources);
  return (
    <ConfirmProvider t={props.t} path={props.path}>
      <Domains {...props} />
      <Palette {...props} />
    </ConfirmProvider>
  );
}

registerPage("rdockpanel/domains", DomainsRoute as (p: PageProps<unknown>) => ReturnType<typeof DomainsRoute>);

/** The Docker pages, on the same frame as the rest (docs/ui-redesign.md R5). */
function dockerRoute(Page: (p: PageProps<never>) => ReturnType<typeof Containers>) {
  return function DockerRoute(props: PageProps<{ user?: string | number }>) {
    bindResources(props.resources);
    return (
      <ConfirmProvider t={props.t} path={props.path}>
        <Page {...(props as PageProps<never>)} />
        <Palette {...props} />
      </ConfirmProvider>
    );
  };
}

registerPage("rdockpanel/containers", dockerRoute(Containers as never) as (p: PageProps<unknown>) => ReturnType<typeof Containers>);
registerPage("rdockpanel/images", dockerRoute(Images as never) as (p: PageProps<unknown>) => ReturnType<typeof Containers>);
registerPage("rdockpanel/volumes", dockerRoute(Volumes as never) as (p: PageProps<unknown>) => ReturnType<typeof Containers>);
registerPage("rdockpanel/networks", dockerRoute(Networks as never) as (p: PageProps<unknown>) => ReturnType<typeof Containers>);
registerSlot("rdockpanel/service-live", ServiceLive as never);
