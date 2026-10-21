// The Domains page (`/admin/p/domains`), like EasyPanel's: every service's
// domains, `https://host/path → http://<project>_<service>:<port>/`, each
// with whether Traefik serves it (a change is applied when saved, while the
// service runs), opened, changed or removed here.
import { useState } from "react";
import { type PageProps } from "@rustcms/admin-sdk";
import { useAdminQuery } from "../lib/query";
import { Button, ErrorState, Loading, NativeSelect } from "@rustcms/admin-sdk/ui";
import {
  EmptyState,
  ErrorNote,
  IconButton,
  type MenuItem,
  NavLink,
  Page,
  Pager,
  RowActions,
  Section,
  StatusBadge,
  TableScroll,
  useConfirm,
  usePaged,
} from "../lib/ui";
import type { Tone } from "../lib/status";
import { resources } from "../lib/resources";
import { DomainForm, type ServiceChoice, plainDomain } from "../apps/DomainForm";
import type { Domain } from "../apps/types";

type Row = Omit<Domain, "id"> & {
  /** None: a Compose service's, removed, served until it deploys. */
  id: number | null;
  project: string;
  service: string;
  kind: string;
  primary: boolean;
  target: string;
  /** In maintenance: Traefik sends it to the panel's page. */
  maintenance: boolean;
  /** Changed by this user (a Compose or privileged service's: administrators'). */
  writable: boolean;
  status: "live" | "pending" | "removed" | "stopped";
  /** Where its name leads (`cloudflare`: proxied or a tunnel's). */
  via?: "server" | "cloudflare" | "elsewhere" | "none";
};

type List = {
  domains: Row[];
  panel: { host: string; url: string } | null;
  services: ServiceChoice[];
  default_resolver: string | null;
  cert_resolvers: Array<{ id: number; name: string; kind: string }>;
  certificates: Array<{ id: number; name: string; names: string[] }>;
  can: { create: boolean; update: boolean; delete: boolean; system: boolean };
};

const STATUS: Record<Row["status"], { tone: Tone; label: string; hint: string }> = {
  live: { tone: "ok", label: "Live", hint: "Traefik serves it as configured." },
  pending: { tone: "busy", label: "Waiting for deploy", hint: "A Compose service's domain: its next deployment applies it." },
  removed: { tone: "bad", label: "Removed", hint: "A Compose service's domain: served until its next deployment." },
  stopped: { tone: "off", label: "Not running", hint: "The service is stopped." },
};

export function Domains(props: PageProps<unknown>) {
  const t = props.t;
  const list = useAdminQuery<List>(props, "/apps/domains", { refetchInterval: 5000, refetchOnMount: "always" });
  const [project, setProject] = useState("");
  const [adding, setAdding] = useState(false);
  const [editing, setEditing] = useState<Row | null>(null);
  const [error, setError] = useState<unknown>(null);
  const [confirm, confirmDialog] = useConfirm(t);
  const rows = (list.data?.domains ?? []).filter((d) => !project || d.project === project);
  const paged = usePaged(rows, project);
  const header = {
    title: props.title,
    eyebrow: t("Workspace"),
    heading: t("Domains"),
    description: t("Every service's domains and where Traefik sends them. A change is applied when saved."),
  };
  if (list.isLoading) return <Loading />;
  if (list.error || !list.data) return <ErrorState error={list.error} />;
  const data = list.data;
  const projects = [...new Set(data.services.map((s) => s.project))].sort();
  const url = (d: { https: boolean; host: string; path?: string }) => `${d.https ? "https" : "http"}://${d.host}${d.path ?? "/"}`;
  const mayChange = (d: Row) => d.id !== null && d.writable && (data.can.system || plainDomain(d));
  // A service's page reads the dashboard's overview.
  const canOpen = props.can("projects.view_any");
  const addable = data.can.create && data.services.some((s) => s.writable);
  const done = () => {
    setAdding(false);
    setEditing(null);
    list.refetch();
  };
  const remove = async (d: Row) => {
    const ok = await confirm({
      title: t("Remove {host}?").replace("{host}", d.host + (d.path !== "/" ? d.path : "")),
      body: t("It stops being served at once."),
      action: t("Remove domain"),
      destructive: true,
    });
    if (!ok) return;
    setError(null);
    try {
      await resources.remove("service_domains", d.id as number);
      props.toast.success(t("Domain removed"));
      list.refetch();
    } catch (err) {
      setError(err);
    }
  };
  const empty = rows.length === 0 && !(data.panel && !project);
  return (
    <Page {...header} actions={addable && <Button onClick={() => setAdding(true)}>{t("Add domain")}</Button>}>
      <ErrorNote error={error} />
      <Section
        title={t("All domains")}
        flush
        actions={
          projects.length > 1 && (
            <NativeSelect value={project} onChange={(e) => setProject(e.target.value)} aria-label={t("Project")}>
              <option value="">{t("All projects")}</option>
              {projects.map((p) => (
                <option key={p} value={p}>
                  {p}
                </option>
              ))}
            </NativeSelect>
          )
        }
      >
        {empty ? (
          <EmptyState
            title={project ? t("No domain in this project") : t("No domain yet")}
            hint={t("Add one to reach a service from the internet.")}
            action={project && <Button size="sm" variant="outline" onClick={() => setProject("")}>{t("Show all projects")}</Button>}
          />
        ) : (
          <>
            <TableScroll>
              <table className="rdp2-table">
                <thead>
                  <tr>
                    <th>{t("Domain")}</th>
                    <th>{t("Goes to")}</th>
                    <th className="rdp2-hide-sm">{t("Service")}</th>
                    <th>{t("State")}</th>
                    <th className="rdp2-right">
                      <span className="rdp2-sr">{t("Actions")}</span>
                    </th>
                  </tr>
                </thead>
                <tbody>
                  {data.panel && !project && paged.at === 0 && (
                    <tr>
                      <td>
                        <span className="rdp2-mono">{data.panel.url}</span>
                      </td>
                      <td className="rdp2-muted">{t("RDockPanel (Settings → Panel)")}</td>
                      <td className="rdp2-hide-sm rdp2-muted">–</td>
                      <td>
                        <StatusBadge tone="ok" label={t("Live")} />
                      </td>
                      <td>
                        <RowActions label={t("Actions for {host}").replace("{host}", data.panel.host)}>
                          <IconButton icon="externalLink" label={t("Open {url}").replace("{url}", data.panel.url)} href={data.panel.url} />
                        </RowActions>
                      </td>
                    </tr>
                  )}
                  {paged.page.map((d) => {
                    const state = STATUS[d.status];
                    const menu: MenuItem[] = [];
                    if (data.can.update && mayChange(d)) menu.push({ label: t("Edit"), onSelect: () => setEditing(d) });
                    if (canOpen) menu.push({ label: t("Open the service"), onSelect: () => props.navigate(`/p/dashboard/${d.project}/${d.service}/domains`) });
                    if (data.can.delete && mayChange(d)) menu.push("separator", { label: t("Remove…"), destructive: true, onSelect: () => void remove(d) });
                    return (
                      <tr key={`${d.id ?? "gone"}-${d.host}${d.path}`}>
                        <td>
                          <div className="rdp2-row">
                            <span className={d.status === "removed" ? "rdp2-mono rdp2-ellipsis rdp2-gone" : "rdp2-mono rdp2-ellipsis"}>{url(d)}</span>
                            {d.via === "cloudflare" && (
                              <StatusBadge
                                tone="off"
                                label={t("via Cloudflare")}
                                title={t("Its name leads to Cloudflare: through the tunnel, Cloudflare gives it HTTPS (HTTPS off here); proxied to this server, it needs a DNS resolver or an uploaded certificate.")}
                              />
                            )}
                            {d.primary && <span className="rdp2-badge">{t("Primary")}</span>}
                          </div>
                        </td>
                        <td>
                          <span className="rdp2-mono rdp2-ellipsis">{d.maintenance ? t("Maintenance page") : d.target}</span>
                        </td>
                        <td className="rdp2-hide-sm">
                          {canOpen ? (
                            <NavLink to={`/p/dashboard/${d.project}/${d.service}/domains`} navigate={props.navigate}>
                              {d.project} / {d.service}
                            </NavLink>
                          ) : (
                            <span>
                              {d.project} / {d.service}
                            </span>
                          )}
                        </td>
                        <td>
                          <StatusBadge tone={state.tone} label={t(state.label)} title={t(state.hint)} />
                        </td>
                        <td>
                          <RowActions label={t("Actions for {host}").replace("{host}", d.host)} menu={menu}>
                            <IconButton icon="externalLink" label={t("Open {url}").replace("{url}", url(d))} href={url(d)} />
                          </RowActions>
                        </td>
                      </tr>
                    );
                  })}
                </tbody>
              </table>
            </TableScroll>
            <Pager total={rows.length} at={paged.at} onPage={paged.set} t={t} />
          </>
        )}
      </Section>
      {(adding || editing) && (
        <DomainForm
          t={t}
          domain={editing ? ({ ...editing, id: editing.id as number } as Domain) : undefined}
          service={editing ? { id: editing.service_id, kind: editing.kind } : undefined}
          services={data.services.filter((s) => !project || s.project === project)}
          cert_resolvers={data.cert_resolvers}
          certificates={data.certificates}
          lookup={(host) => props.api.get<{ via: string; tunnel: boolean }>("/apps/domains/dns", { query: { host } })}
          defaultResolver={data.default_resolver}
          system={data.can.system}
          onClose={() => {
            setAdding(false);
            setEditing(null);
          }}
          onSaved={done}
        />
      )}
      {confirmDialog}
    </Page>
  );
}
