// The Volumes and Networks pages (`/admin/p/volumes`, `/admin/p/networks`):
// Docker's volumes and networks, made and removed here. Docker's own networks
// (bridge, host, none, ingress, docker_gwbridge) cannot be removed.
import { useState, type ReactNode } from "react";
import { fieldErrors, type PageProps, useAdminQuery } from "@rustcms/admin-sdk";
import { Button, Dialog, ErrorState, Input, Loading, NativeSelect } from "@rustcms/admin-sdk/ui";
import { EmptyState, ErrorNote, Field, Page, Pager, RowActions, Section, StatusBadge, TableScroll, useAsk, usePaged } from "../lib/ui";

type Can = { "docker.manage"?: boolean };
type Volume = { name: string; driver: string; mountpoint: string };
type Network = { id: string; name: string; driver: string; scope: string };

const BUILT_IN = new Set(["bridge", "host", "none", "ingress", "docker_gwbridge"]);

/** A dialog making one thing: its fields, then [Cancel] [Create]. */
function CreateDialog(props: {
  t: (s: string) => string;
  title: string;
  onClose: () => void;
  onSubmit: () => Promise<void>;
  disabled: boolean;
  error: unknown;
  children: ReactNode;
}) {
  const [busy, setBusy] = useState(false);
  const fields = props.error ? fieldErrors(props.error) : {};
  return (
    <Dialog open onClose={props.onClose} title={props.title}>
      <form
        className="rdp2-form"
        onSubmit={async (e) => {
          e.preventDefault();
          setBusy(true);
          try {
            await props.onSubmit();
          } finally {
            setBusy(false);
          }
        }}
      >
        {props.children}
        {Object.keys(fields).length === 0 && <ErrorNote error={props.error} />}
        <div className="rdp2-dialog-foot">
          <Button type="button" variant="ghost" onClick={props.onClose}>
            {props.t("Cancel")}
          </Button>
          <Button type="submit" disabled={busy || props.disabled}>
            {props.t("Create")}
          </Button>
        </div>
      </form>
    </Dialog>
  );
}

export function Volumes(props: PageProps<{ can?: Can }>) {
  const t = props.t;
  const can = props.data?.can ?? {};
  const list = useAdminQuery<{ volumes: Volume[] }>(props, "/docker/volumes", { refetchInterval: 10000 });
  const [creating, setCreating] = useState(false);
  const [name, setName] = useState("");
  const [error, setError] = useState<unknown>(null);
  const [formError, setFormError] = useState<unknown>(null);
  const ask = useAsk();
  const volumes = [...(list.data?.volumes ?? [])].sort((a, b) => a.name.localeCompare(b.name));
  const paged = usePaged(volumes);
  const header = { title: props.title, eyebrow: "Docker", heading: t("Volumes"), description: t("Docker volumes keep data across containers and deployments.") };
  if (list.isLoading) return <Loading />;
  if (list.error || !list.data) return <ErrorState error={list.error} />;
  const remove = async (v: Volume) => {
    const ok = await ask({
      title: t("Remove the volume {name}?").replace("{name}", v.name),
      body: t("Its data is deleted for good."),
      action: t("Remove volume"),
      destructive: true,
      typed: v.name,
    });
    if (!ok) return;
    setError(null);
    try {
      await props.api.delete(`/docker/volumes/${encodeURIComponent(v.name)}`);
      props.toast.success(t("Removed"));
      list.refetch();
    } catch (err) {
      setError(err);
    }
  };
  return (
    <Page {...header} actions={can["docker.manage"] && <Button onClick={() => setCreating(true)}>{t("New volume")}</Button>}>
      <ErrorNote error={error} />
      <Section
        title={t("All volumes")}
        flush
        actions={
          <Button size="sm" variant="outline" onClick={() => list.refetch()}>
            {t("Refresh")}
          </Button>
        }
      >
        {volumes.length === 0 ? (
          <EmptyState title={t("No volumes.")} />
        ) : (
          <>
            <TableScroll>
              <table className="rdp2-table">
                <thead>
                  <tr>
                    <th>{t("Name")}</th>
                    <th>{t("Driver")}</th>
                    <th className="rdp2-hide-sm">{t("Mountpoint")}</th>
                    <th className="rdp2-right">
                      <span className="rdp2-sr">{t("Actions")}</span>
                    </th>
                  </tr>
                </thead>
                <tbody>
                  {paged.page.map((v) => (
                    <tr key={v.name}>
                      <td>
                        <span className="rdp2-mono rdp2-ellipsis rdp2-cell-clip" title={v.name}>
                          {v.name}
                        </span>
                      </td>
                      <td>{v.driver}</td>
                      <td className="rdp2-hide-sm">
                        <span className="rdp2-mono rdp2-muted rdp2-ellipsis rdp2-cell-clip" title={v.mountpoint}>
                          {v.mountpoint}
                        </span>
                      </td>
                      <td>
                        <RowActions
                          label={t("Actions for {host}").replace("{host}", v.name)}
                          menu={can["docker.manage"] ? [{ label: t("Remove…"), destructive: true, onSelect: () => void remove(v) }] : []}
                        />
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </TableScroll>
            <Pager total={volumes.length} at={paged.at} onPage={paged.set} t={t} />
          </>
        )}
      </Section>
      {creating && (
        <CreateDialog
          t={t}
          title={t("New volume")}
          disabled={!name.trim()}
          error={formError}
          onClose={() => setCreating(false)}
          onSubmit={async () => {
            setFormError(null);
            try {
              await props.api.post("/docker/volumes", { name: name.trim() });
              props.toast.success(t("Created"));
              setName("");
              setCreating(false);
              list.refetch();
            } catch (err) {
              setFormError(err);
            }
          }}
        >
          <Field label={t("Name")} error={fieldErrors(formError).name}>
            <Input autoFocus className="rdp2-mono" value={name} onChange={(e) => setName(e.target.value)} placeholder="shared-data" />
          </Field>
        </CreateDialog>
      )}
    </Page>
  );
}

export function Networks(props: PageProps<{ can?: Can }>) {
  const t = props.t;
  const can = props.data?.can ?? {};
  const list = useAdminQuery<{ networks: Network[] }>(props, "/docker/networks", { refetchInterval: 10000 });
  const [creating, setCreating] = useState(false);
  const [name, setName] = useState("");
  const [driver, setDriver] = useState("overlay");
  const [error, setError] = useState<unknown>(null);
  const [formError, setFormError] = useState<unknown>(null);
  const ask = useAsk();
  const networks = [...(list.data?.networks ?? [])].sort((a, b) => a.name.localeCompare(b.name));
  const paged = usePaged(networks);
  const header = { title: props.title, eyebrow: "Docker", heading: t("Networks"), description: t("How containers reach each other: one network per project, Traefik on each.") };
  if (list.isLoading) return <Loading />;
  if (list.error || !list.data) return <ErrorState error={list.error} />;
  const remove = async (n: Network) => {
    const ok = await ask({ title: t("Remove the network {name}?").replace("{name}", n.name), action: t("Remove network"), destructive: true });
    if (!ok) return;
    setError(null);
    try {
      await props.api.delete(`/docker/networks/${encodeURIComponent(n.id)}`);
      props.toast.success(t("Removed"));
      list.refetch();
    } catch (err) {
      setError(err);
    }
  };
  return (
    <Page {...header} actions={can["docker.manage"] && <Button onClick={() => setCreating(true)}>{t("New network")}</Button>}>
      <ErrorNote error={error} />
      <Section
        title={t("All networks")}
        flush
        actions={
          <Button size="sm" variant="outline" onClick={() => list.refetch()}>
            {t("Refresh")}
          </Button>
        }
      >
        {networks.length === 0 ? (
          <EmptyState title={t("No networks.")} />
        ) : (
          <>
            <TableScroll>
              <table className="rdp2-table">
                <thead>
                  <tr>
                    <th>{t("Name")}</th>
                    <th className="rdp2-hide-sm">{t("ID")}</th>
                    <th>{t("Driver")}</th>
                    <th>{t("Scope")}</th>
                    <th className="rdp2-right">
                      <span className="rdp2-sr">{t("Actions")}</span>
                    </th>
                  </tr>
                </thead>
                <tbody>
                  {paged.page.map((n) => (
                    <tr key={n.id}>
                      <td className="rdp2-mono">{n.name}</td>
                      <td className="rdp2-hide-sm rdp2-mono rdp2-muted" title={n.id}>
                        {n.id.slice(0, 12)}
                      </td>
                      <td>{n.driver}</td>
                      <td>
                        <StatusBadge tone={n.scope === "swarm" ? "ok" : "off"} label={n.scope} />
                      </td>
                      <td>
                        <RowActions
                          label={t("Actions for {host}").replace("{host}", n.name)}
                          menu={
                            can["docker.manage"]
                              ? [
                                  {
                                    label: BUILT_IN.has(n.name) ? t("Docker's own: kept") : t("Remove…"),
                                    destructive: true,
                                    disabled: BUILT_IN.has(n.name),
                                    onSelect: () => void remove(n),
                                  },
                                ]
                              : []
                          }
                        />
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </TableScroll>
            <Pager total={networks.length} at={paged.at} onPage={paged.set} t={t} />
          </>
        )}
      </Section>
      {creating && (
        <CreateDialog
          t={t}
          title={t("New network")}
          disabled={!name.trim()}
          error={formError}
          onClose={() => setCreating(false)}
          onSubmit={async () => {
            setFormError(null);
            try {
              await props.api.post("/docker/networks", { name: name.trim(), driver });
              props.toast.success(t("Created"));
              setName("");
              setCreating(false);
              list.refetch();
            } catch (err) {
              setFormError(err);
            }
          }}
        >
          <Field label={t("Name")} error={fieldErrors(formError).name}>
            <Input autoFocus className="rdp2-mono" value={name} onChange={(e) => setName(e.target.value)} placeholder="shared" />
          </Field>
          <Field label={t("Driver")}>
            <NativeSelect value={driver} onChange={(e) => setDriver(e.target.value)}>
              <option value="overlay">{t("overlay (the swarm)")}</option>
              <option value="bridge">{t("bridge (this server)")}</option>
            </NativeSelect>
          </Field>
        </CreateDialog>
      )}
    </Page>
  );
}
