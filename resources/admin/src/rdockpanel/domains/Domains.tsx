// The Domains page (`/admin/p/domains`), like EasyPanel's: every service's
// domains, `https://host/path → http://<project>_<service>:<port>/`, each
// with whether Traefik serves it as configured (a change waits for the
// service's next deployment), opened, changed or removed here.
import { useState } from "react";
import { type PageProps, useAdminQuery } from "@rustcms/admin-sdk";
import { Button, ErrorState, Loading, NativeSelect } from "@rustcms/admin-sdk/ui";
import { ErrorNote, StatusDot } from "../lib/ui";
import { resources } from "../lib/resources";
import { DomainForm, type ServiceChoice, plainDomain } from "../apps/DomainForm";
import type { Domain } from "../apps/types";

type Row = Omit<Domain, "id"> & {
  /** None: removed, served until its service's next deployment. */
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

const STATUS: Record<Row["status"], { tone: string; label: string }> = {
  live: { tone: "running", label: "Live" },
  pending: { tone: "deploying", label: "Waiting for Deploy" },
  removed: { tone: "failed", label: "Removed, served until Deploy" },
  stopped: { tone: "stopped", label: "Service not running" },
};

export function Domains(props: PageProps<unknown>) {
  const t = props.t;
  const list = useAdminQuery<List>(props, "/apps/domains", { refetchInterval: 5000, refetchOnMount: "always" });
  const [project, setProject] = useState("");
  const [adding, setAdding] = useState(false);
  const [editing, setEditing] = useState<Row | null>(null);
  const [error, setError] = useState<unknown>(null);
  if (list.isLoading) return <Loading />;
  if (list.error || !list.data) return <ErrorState error={list.error} />;
  const data = list.data;
  const projects = [...new Set(data.services.map((s) => s.project))].sort();
  const rows = data.domains.filter((d) => !project || d.project === project);
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
  return (
    <div className="rdp2-page">
      <div className="rdp2-head">
        <p className="rdp2-muted">{t("Every service's domains and where Traefik sends them. A change is applied by the service's next deployment.")}</p>
        <div className="rdp2-row">
          {projects.length > 1 && (
            <NativeSelect className="rdp2-narrow" value={project} onChange={(e) => setProject(e.target.value)} aria-label={t("Project")}>
              <option value="">{t("All projects")}</option>
              {projects.map((p) => (
                <option key={p} value={p}>
                  {p}
                </option>
              ))}
            </NativeSelect>
          )}
          {addable && <Button onClick={() => setAdding(true)}>+ {t("Add domain")}</Button>}
        </div>
      </div>
      <ErrorNote error={error} />
      <div className="rdp2-domains">
        {data.panel && !project && (
          <div className="rdp2-domain">
            <span className="rdp2-domain-box rdp2-mono">{data.panel.url}</span>
            <span className="rdp2-muted" aria-hidden>
              →
            </span>
            <span className="rdp2-domain-box rdp2-muted">{t("RDockPanel (Settings → Panel)")}</span>
            <span className="rdp2-domain-tools">
              <a className="rdp2-icon-button" href={data.panel.url} target="_blank" rel="noreferrer" title={t("Open")} aria-label={t("Open")}>
                ↗
              </a>
            </span>
          </div>
        )}
        {rows.map((d) => (
          <div key={`${d.id ?? "gone"}-${d.host}${d.path}`} className={d.status === "removed" ? "rdp2-domain rdp2-domain-gone" : "rdp2-domain"}>
            <span className="rdp2-domain-box rdp2-mono" title={t(STATUS[d.status].label)}>
              <StatusDot status={STATUS[d.status].tone} title={t(STATUS[d.status].label)} />
              <span className="rdp2-ellipsis">
                {d.primary && <span title={t("Primary domain")}>★ </span>}
                {url(d)}
              </span>
            </span>
            <span className="rdp2-muted" aria-hidden>
              →
            </span>
            <span className="rdp2-domain-box">
              <span className="rdp2-mono rdp2-ellipsis">{d.maintenance ? t("Maintenance page") : d.target}</span>
              {canOpen ? (
                <button type="button" className="rdp2-link rdp2-muted rdp2-domain-service" onClick={() => props.navigate(`/p/dashboard/${d.project}/${d.service}/domains`)}>
                  {d.project} / {d.service}
                </button>
              ) : (
                <span className="rdp2-muted rdp2-domain-service">
                  {d.project} / {d.service}
                </span>
              )}
            </span>
            <span className="rdp2-domain-tools">
              <a className="rdp2-icon-button" href={url(d)} target="_blank" rel="noreferrer" title={t("Open")} aria-label={t("Open")}>
                ↗
              </a>
              {data.can.update && mayChange(d) && (
                <button type="button" className="rdp2-icon-button" title={t("Edit")} aria-label={t("Edit")} onClick={() => setEditing(d)}>
                  ✎
                </button>
              )}
              {data.can.delete && mayChange(d) && (
                <button
                  type="button"
                  className="rdp2-icon-button"
                  title={t("Remove")}
                  aria-label={t("Remove")}
                  onClick={async () => {
                    if (!window.confirm(t("Remove this domain?"))) return;
                    setError(null);
                    try {
                      await resources.remove("service_domains", d.id as number);
                      list.refetch();
                    } catch (err) {
                      setError(err);
                    }
                  }}
                >
                  🗑
                </button>
              )}
            </span>
          </div>
        ))}
        {rows.length === 0 && !data.panel && <p className="rdp2-muted">{t("No domain yet: add one to a service.")}</p>}
        {addable && (
          <Button variant="outline" className="rdp2-domain-add" onClick={() => setAdding(true)}>
            {t("Add domain")}
          </Button>
        )}
      </div>
      {(adding || editing) && (
        <DomainForm
          t={t}
          domain={editing ? ({ ...editing, id: editing.id as number } as Domain) : undefined}
          service={editing ? { id: editing.service_id, kind: editing.kind } : undefined}
          services={data.services.filter((s) => !project || s.project === project)}
          cert_resolvers={data.cert_resolvers}
          certificates={data.certificates}
          defaultResolver={data.default_resolver}
          system={data.can.system}
          onClose={() => {
            setAdding(false);
            setEditing(null);
          }}
          onSaved={done}
        />
      )}
    </div>
  );
}
