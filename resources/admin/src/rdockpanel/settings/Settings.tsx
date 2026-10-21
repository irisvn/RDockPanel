// The panel's Settings (`/admin/p/settings`), like EasyPanel's: storage
// providers for backups (a local directory or an S3-compatible bucket),
// each with a "Test" button, and notification channels. Written through the `storage_providers`
// resource (its secret key sealed, never shown again).
import { useState } from "react";
import { type PageProps } from "@rustcms/admin-sdk";
import { useAdminQuery } from "../lib/query";
import { Alert, Button, Dialog, ErrorState, Input, Loading, NativeSelect, Switch } from "@rustcms/admin-sdk/ui";
import { EmptyState, ErrorNote, Field, type MenuItem, Page, RowActions, Section, TableScroll, Tabs, useAsk } from "../lib/ui";
import { ResourceError, resources } from "../lib/resources";
import { Notifications } from "./Notifications";
import { Certificates } from "./Certificates";
import { UploadedCertificates } from "./Uploaded";
import { Traefik } from "./Traefik";
import { Panel } from "./Panel";
import { Version } from "./Version";
import { Git } from "./Git";
import { Tokens } from "./Tokens";
import { Tunnel } from "./Tunnel";
import { BrandingSettings } from "./Branding";

type Provider = {
  id: number;
  name: string;
  kind: string;
  local_path: string | null;
  endpoint: string | null;
  region: string | null;
  bucket: string | null;
  access_key: string | null;
  path_style: boolean;
  secret_set: boolean;
};

function ProviderForm(props: PageProps<unknown> & { provider: Provider | null; onClose: () => void; onSaved: () => void }) {
  const t = props.t;
  const p = props.provider;
  const [name, setName] = useState(p?.name ?? "");
  const [kind, setKind] = useState(p?.kind ?? "local");
  const [localPath, setLocalPath] = useState(p?.local_path ?? "/var/lib/rdockpanel/backups");
  const [endpoint, setEndpoint] = useState(p?.endpoint ?? "");
  const [region, setRegion] = useState(p?.region ?? "");
  const [bucket, setBucket] = useState(p?.bucket ?? "");
  const [accessKey, setAccessKey] = useState(p?.access_key ?? "");
  const [secret, setSecret] = useState("");
  const [pathStyle, setPathStyle] = useState(p?.path_style ?? false);
  const [fields, setFields] = useState<Record<string, string[]>>({});
  const [error, setError] = useState<unknown>(null);
  return (
    <Dialog open onClose={props.onClose} title={p ? t("Edit storage provider") : t("Add storage provider")}>
      <form
        className="rdp2-form"
        onSubmit={async (e) => {
          e.preventDefault();
          setFields({});
          setError(null);
          const data: Record<string, unknown> =
            kind === "local"
              ? { name: name.trim(), kind, local_path: localPath.trim() }
              : {
                  name: name.trim(),
                  kind,
                  endpoint: endpoint.trim(),
                  region: region.trim() || null,
                  bucket: bucket.trim(),
                  access_key: accessKey.trim(),
                  path_style: pathStyle,
                };
          if (kind === "s3" && secret) data.secret_key = { replace: secret };
          try {
            if (p) await resources.update("storage_providers", p.id, data);
            else await resources.create("storage_providers", data);
            props.onSaved();
          } catch (err) {
            if (err instanceof ResourceError && err.status === 422) setFields(err.fieldErrors);
            else setError(err);
          }
        }}
      >
        <Field label={t("Name")} error={fields.name}>
          <Input autoFocus value={name} onChange={(e) => setName(e.target.value)} />
        </Field>
        <Field label={t("Type")} error={fields.kind}>
          <NativeSelect value={kind} onChange={(e) => setKind(e.target.value)} disabled={!!p}>
            <option value="local">{t("Local directory")}</option>
            <option value="s3">{t("S3-compatible (AWS, Cloudflare R2, MinIO, …)")}</option>
          </NativeSelect>
        </Field>
        {kind === "local" ? (
          <Field
            label={t("Directory")}
            help={t("An absolute directory of the server. In the installed panel, keep it under /var/lib/rdockpanel (a host directory)." )}
            error={fields.local_path}
          >
            <Input className="rdp2-mono" value={localPath} onChange={(e) => setLocalPath(e.target.value)} />
          </Field>
        ) : (
          <>
            <Field label={t("Endpoint")} help="https://s3.eu-west-1.amazonaws.com" error={fields.endpoint}>
              <Input value={endpoint} onChange={(e) => setEndpoint(e.target.value)} />
            </Field>
            <div className="rdp2-row">
              <Field label={t("Bucket")} error={fields.bucket}>
                <Input value={bucket} onChange={(e) => setBucket(e.target.value)} />
              </Field>
              <Field label={t("Region")} error={fields.region}>
                <Input placeholder="us-east-1" value={region} onChange={(e) => setRegion(e.target.value)} />
              </Field>
            </div>
            <Field label={t("Access key")} error={fields.access_key}>
              <Input value={accessKey} onChange={(e) => setAccessKey(e.target.value)} />
            </Field>
            <Field
              label={t("Secret key")}
              help={p?.secret_set ? t("Set: leave empty to keep it") : undefined}
              error={fields.secret_key}
            >
              <Input type="password" autoComplete="new-password" value={secret} onChange={(e) => setSecret(e.target.value)} />
            </Field>
            <label className="rdp2-row">
              <Switch checked={pathStyle} onCheckedChange={setPathStyle} /> {t("Path-style URLs (MinIO and most S3-compatible services)")}
            </label>
          </>
        )}
        <ErrorNote error={error} />
        <div className="rdp2-row rdp2-end">
          <Button type="submit">{p ? t("Save") : t("Create")}</Button>
        </div>
      </form>
    </Dialog>
  );
}

function StorageProviders(props: PageProps<unknown>) {
  const t = props.t;
  const list = useAdminQuery<{ providers: Provider[] }>(props, "/apps/storage-providers");
  const [editing, setEditing] = useState<Provider | "new" | null>(null);
  const [tested, setTested] = useState<Record<number, { ok: boolean; error?: string } | "busy">>({});
  const [error, setError] = useState<unknown>(null);
  const ask = useAsk();
  if (list.isLoading) return <Loading />;
  if (list.error || !list.data) return <ErrorState error={list.error} />;
  return (
    <Section
      flush
      title={t("Storage providers")}
      description={t("Where database backups are written.")}
      actions={props.can("storage_providers.create") && <Button onClick={() => setEditing("new")}>{t("Add storage provider")}</Button>}
    >
      {error != null && (
        <div className="rdp2-flush-note">
          <ErrorNote error={error} />
        </div>
      )}
      {list.data.providers.length === 0 ? (
        <EmptyState title={t("No storage provider yet.")} hint={t("Backups need one: a directory on the server or an S3 bucket.")} />
      ) : (
        <TableScroll>
          <table className="rdp2-table">
            <thead>
              <tr>
                <th>{t("Name")}</th>
                <th>{t("Type")}</th>
                <th className="rdp2-hide-sm">{t("Where")}</th>
                <th className="rdp2-right">
                  <span className="rdp2-sr">{t("Actions")}</span>
                </th>
              </tr>
            </thead>
            <tbody>
              {list.data.providers.map((p) => {
                const result = tested[p.id];
                const menu: MenuItem[] = [];
                if (props.can("storage_providers.update")) menu.push({ label: t("Edit"), onSelect: () => setEditing(p) });
                if (props.can("storage_providers.delete"))
                  menu.push("separator", {
                    label: t("Delete…"),
                    destructive: true,
                    onSelect: async () => {
                      const ok = await ask({
                        title: t("Delete the storage provider {name}?").replace("{name}", p.name),
                        body: t("Its files stay where they are."),
                        action: t("Delete provider"),
                        destructive: true,
                      });
                      if (!ok) return;
                      setError(null);
                      try {
                        await resources.remove("storage_providers", p.id);
                        list.refetch();
                      } catch (err) {
                        setError(
                          err instanceof ResourceError && err.status === 403 ? new Error(t("A backup uses this provider: change or delete it first.")) : err,
                        );
                      }
                    },
                  });
                return (
                  <tr key={p.id}>
                    <td className="rdp2-strong-cell">{p.name}</td>
                    <td>{p.kind === "s3" ? "S3" : t("Local directory")}</td>
                    <td className="rdp2-hide-sm">
                      <span className="rdp2-mono rdp2-ellipsis rdp2-cell-clip">{p.kind === "s3" ? `${p.bucket} · ${p.endpoint}` : p.local_path}</span>
                    </td>
                    <td>
                      <RowActions label={t("Actions for {host}").replace("{host}", p.name)} menu={menu}>
                        {result && result !== "busy" && (
                          <span className={result.ok ? "rdp2-small" : "rdp2-error rdp2-small"}>{result.ok ? t("Works") : result.error}</span>
                        )}
                        {props.can("storage_providers.update") && (
                          <Button
                            size="sm"
                            variant="ghost"
                            disabled={result === "busy"}
                            onClick={async () => {
                              setTested((s) => ({ ...s, [p.id]: "busy" }));
                              try {
                                const out = await props.api.post<{ ok: boolean; error?: string }>(`/apps/storage-providers/${p.id}/check`, {});
                                setTested((s) => ({ ...s, [p.id]: out }));
                              } catch (err) {
                                setTested((s) => ({ ...s, [p.id]: { ok: false, error: err instanceof Error ? err.message : String(err) } }));
                              }
                            }}
                          >
                            {result === "busy" ? t("Testing…") : t("Test")}
                          </Button>
                        )}
                      </RowActions>
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        </TableScroll>
      )}
      {editing && (
        <ProviderForm
          {...props}
          provider={editing === "new" ? null : editing}
          onClose={() => setEditing(null)}
          onSaved={() => {
            setEditing(null);
            list.refetch();
          }}
        />
      )}
    </Section>
  );
}

/** Its sub-pages (`/p/settings/<tab>`), each shown with its own permission. */
const TABS: Array<{ id: string; label: string; permission: string | null }> = [
  { id: "panel", label: "Panel", permission: "system.manage" },
  { id: "version", label: "Version", permission: "system.manage" },
  { id: "traefik", label: "Traefik", permission: "system.manage" },
  { id: "certificates", label: "Certificates", permission: "system.manage" },
  { id: "tunnel", label: "Cloudflare Tunnel", permission: "system.manage" },
  { id: "branding", label: "Branding", permission: "system.manage" },
  { id: "git", label: "Git", permission: "system.manage" },
  { id: "backups", label: "Backups", permission: "storage_providers.view_any" },
  { id: "notifications", label: "Notifications", permission: "notification_channels.view_any" },
  // Everyone's own.
  { id: "tokens", label: "API tokens", permission: null },
];

export function Settings(props: PageProps<unknown>) {
  const t = props.t;
  // The page opens with any of these permissions (`permission_any`): each
  // tab with its own.
  const tabs = TABS.filter((tab) => tab.permission === null || props.can(tab.permission));
  const wanted = (props.path ?? "").split("/")[0];
  const tab = tabs.find((x) => x.id === wanted)?.id ?? tabs[0]?.id;
  const header = {
    title: props.title,
    eyebrow: t("Workspace"),
    heading: t("Settings"),
    description: t("The panel, its proxy and certificates, storage, notifications and your API tokens."),
  };
  if (!tab)
    return (
      <Page {...header}>
        <EmptyState title={t("Nothing to set here with your permissions.")} />
      </Page>
    );
  return (
    <Page {...header}>
      <Tabs
        label={t("Settings")}
        value={tab}
        onSelect={(id) => props.navigate(`/p/settings/${id}`)}
        tabs={tabs.map((x) => ({ id: x.id, label: t(x.label) }))}
      >
        {tab === "panel" && <Panel {...props} />}
        {tab === "version" && <Version {...props} />}
        {tab === "traefik" && <Traefik {...props} />}
        {tab === "certificates" && (
          <>
            <Certificates {...props} />
            <UploadedCertificates {...props} />
          </>
        )}
        {tab === "tunnel" && <Tunnel {...props} />}
        {tab === "branding" && <BrandingSettings {...props} />}
        {tab === "git" && <Git {...props} />}
        {tab === "backups" && <StorageProviders {...props} />}
        {tab === "notifications" && <Notifications {...props} />}
        {tab === "tokens" && <Tokens {...props} />}
      </Tabs>
    </Page>
  );
}
