// Settings → Cloudflare Tunnel: the panel runs one cloudflared per tunnel
// (one Cloudflare token each), so domains are reached through Cloudflare
// without opening ports 80 and 443. Each public hostname of a tunnel (set in
// Cloudflare) goes to the origin shown in its guide; every domain of the
// panel is served there too.
import { useEffect, useRef, useState } from "react";
import { fieldErrors, type PageProps } from "@rustcms/admin-sdk";
import { useAdminQuery } from "../lib/query";
import { Alert, Button, CodeBlock, Dialog, ErrorState, Input, Loading } from "@rustcms/admin-sdk/ui";
import { EmptyState, ErrorNote, Field, RowActions, Section, Status, StatusBadge, TableScroll, useAsk } from "../lib/ui";

type Tunnel = {
  name: string;
  enabled: boolean;
  token_set: boolean;
  service: boolean;
  service_name: string;
  running: boolean;
  connected: boolean;
  error: string | null;
  image: string;
  origin: string;
};

/** Its status in the health words (docs/ui-guidelines.md §4). */
function state(tunnel: Tunnel): string {
  if (!tunnel.enabled) return "off";
  return tunnel.connected ? "connected" : tunnel.error ? "not-connected" : "starting";
}

export function Tunnel(props: PageProps<unknown>) {
  const ask = useAsk();
  const t = props.t;
  const list = useAdminQuery<{ tunnels: Tunnel[] }>(props, "/apps/tunnels", { refetchInterval: 10000 });
  // The tunnel being changed (its row's actions wait), and how.
  const [busy, setBusy] = useState<{ name: string; label: string } | null>(null);
  const [error, setError] = useState<unknown>(null);
  // The dialog open: adding (no name), a new token for `name`, or its guide.
  const [editing, setEditing] = useState<{ name?: string } | null>(null);
  const [guide, setGuide] = useState<Tunnel | null>(null);
  if (list.isLoading) return <Loading />;
  if (list.error || !list.data) return <ErrorState error={list.error} />;
  const tunnels = list.data.tunnels;
  const act = async (name: string, label: string, call: () => Promise<unknown>, done: string) => {
    setBusy({ name, label });
    setError(null);
    try {
      await call();
      props.toast.success(t(done));
      list.refetch();
    } catch (err) {
      setError(err);
    } finally {
      setBusy(null);
    }
  };
  const path = (name: string) => `/apps/tunnels/${encodeURIComponent(name)}`;
  const add = <Button onClick={() => setEditing({})}>{t("Add tunnel")}</Button>;
  const failing = tunnels.filter((x) => x.enabled && x.error);
  return (
    <Section
      flush
      title={t("Cloudflare Tunnel")}
      description={t("Reach your domains through Cloudflare, without opening ports 80 and 443 (cloudflared runs beside Traefik).")}
      actions={tunnels.length > 0 ? add : undefined}
    >
      <div className="rdp2-flush-note rdp2-form">
        <ErrorNote error={error} />
        {failing.map((x) => (
          <Alert key={x.name} variant="warning">
            {x.name}: {x.error}
          </Alert>
        ))}
      </div>
      {tunnels.length === 0 ? (
        <EmptyState title={t("No tunnel yet.")} hint={t("In Cloudflare: Zero Trust → Networks → Tunnels → Create a tunnel (cloudflared), copy its token.")} action={add} />
      ) : (
        <TableScroll>
          <table className="rdp2-table">
            <thead>
              <tr>
                <th>{t("Name")}</th>
                <th>{t("Status")}</th>
                <th className="rdp2-hide-sm">{t("Service")}</th>
                <th className="rdp2-right">
                  <span className="rdp2-sr">{t("Actions")}</span>
                </th>
              </tr>
            </thead>
            <tbody>
              {tunnels.map((x) => {
                const mine = busy?.name === x.name;
                return (
                  <tr key={x.name}>
                    <td className="rdp2-strong-cell">{x.name}</td>
                    <td>
                      {mine ? (
                        <StatusBadge tone="busy" label={t(busy.label)} />
                      ) : (
                        <Status kind="health" status={state(x)} t={t} detail={x.error ?? undefined} />
                      )}
                    </td>
                    <td className="rdp2-hide-sm rdp2-mono">{x.service_name}</td>
                    <td>
                      <RowActions
                        label={t("Actions for {host}").replace("{host}", x.name)}
                        menu={[
                          { label: t("Hostnames…"), onSelect: () => setGuide(x) },
                          { label: t("Replace token…"), disabled: busy != null, onSelect: () => setEditing({ name: x.name }) },
                          "separator",
                          {
                            label: t("Remove…"),
                            destructive: true,
                            disabled: busy != null,
                            onSelect: async () => {
                              const ok = await ask({
                                title: t("Remove the tunnel {name}?").replace("{name}", x.name),
                                body: t("Its cloudflared stops and its token is forgotten: domains it served are then reached only another way."),
                                action: t("Remove tunnel"),
                                destructive: true,
                              });
                              if (ok) void act(x.name, "Removing…", () => props.api.delete(path(x.name)), "Tunnel removed");
                            },
                          },
                        ]}
                      >
                        {x.enabled ? (
                          <Button
                            size="sm"
                            variant="ghost"
                            disabled={busy != null}
                            onClick={async () => {
                              const ok = await ask({
                                title: t("Turn the tunnel {name} off?").replace("{name}", x.name),
                                body: t("Domains it served are then reached only through ports 80 and 443."),
                                action: t("Turn off"),
                                destructive: true,
                              });
                              if (ok) void act(x.name, "Turning off…", () => props.api.post(`${path(x.name)}/disable`, {}), "Tunnel off");
                            }}
                          >
                            {mine ? t(busy.label) : t("Turn off")}
                          </Button>
                        ) : (
                          <Button
                            size="sm"
                            variant="ghost"
                            disabled={busy != null}
                            onClick={() => void act(x.name, "Turning on…", () => props.api.post(`${path(x.name)}/enable`, {}), "Tunnel on: connecting…")}
                          >
                            {mine ? t(busy.label) : t("Turn on")}
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
        <TunnelDialog
          {...props}
          name={editing.name}
          onClose={() => setEditing(null)}
          onDone={() => {
            setEditing(null);
            list.refetch();
          }}
        />
      )}
      {guide && (
        <Dialog open onClose={() => setGuide(null)} title={t("Hostnames of {name}").replace("{name}", guide.name)}>
          <div className="rdp2-form">
            <p className="rdp2-small">{t("For each domain, add a public hostname to the tunnel with this service:")}</p>
            <CodeBlock code={guide.origin} label={t("Service (origin)")} />
            <Alert>
              {t("Cloudflare serves HTTPS (domains' certificates are not used through the tunnel); a visitor over plain HTTP is redirected. The visitor's address reaches apps as Cloudflare's Cf-Connecting-Ip header (the panel sees cloudflared's).")}
            </Alert>
            <p className="rdp2-small rdp2-muted">{guide.image}</p>
            <div className="rdp2-dialog-foot">
              <Button type="button" variant="ghost" autoFocus onClick={() => setGuide(null)}>
                {t("Close")}
              </Button>
            </div>
          </div>
        </Dialog>
      )}
    </Section>
  );
}

/** Adds a tunnel (a name and its token), or gives the tunnel `name` a new
 * token (write-only: the one kept is never shown). Either way it runs. */
function TunnelDialog(props: PageProps<unknown> & { name?: string; onClose: () => void; onDone: () => void }) {
  const t = props.t;
  const adding = props.name == null;
  const [name, setName] = useState("");
  const [token, setToken] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<unknown>(null);
  const form = useRef<HTMLFormElement>(null);
  const fields = error ? fieldErrors(error) : {};
  // A refusal: the first field in error takes the focus.
  useEffect(() => {
    const first = ["name", "token"].find((f) => fields[f]);
    if (first) form.current?.querySelector<HTMLInputElement>(`input[name="${first}"]`)?.focus();
  }, [error]); // eslint-disable-line react-hooks/exhaustive-deps
  return (
    <Dialog open onClose={props.onClose} title={adding ? t("Add tunnel") : t("Replace the token of {name}").replace("{name}", props.name!)}>
      <form
        ref={form}
        className="rdp2-form"
        onSubmit={async (e) => {
          e.preventDefault();
          setBusy(true);
          setError(null);
          try {
            if (adding) await props.api.post("/apps/tunnels", { name: name.trim(), token: token.trim() || null });
            else await props.api.post(`/apps/tunnels/${encodeURIComponent(props.name!)}/enable`, { token: token.trim() });
            props.toast.success(t(adding ? "Tunnel added: connecting…" : "Token replaced: connecting…"));
            props.onDone();
          } catch (err) {
            setError(err);
          } finally {
            setBusy(false);
          }
        }}
      >
        {adding && (
          <Field label={t("Name")} help={t("Lowercase letters, digits and dashes, starting with a letter.")} error={fields.name}>
            <Input name="name" autoFocus autoComplete="off" className="rdp2-mono" value={name} onChange={(e) => setName(e.target.value)} placeholder="cloudflare-team" />
          </Field>
        )}
        <Field label={t("Tunnel token")} help={t("In Cloudflare: Zero Trust → Networks → Tunnels → Create a tunnel (cloudflared), copy its token.")} error={fields.token}>
          <Input
            name="token"
            type="password"
            autoFocus={!adding}
            autoComplete="off"
            value={token}
            onChange={(e) => setToken(e.target.value)}
            placeholder={adding ? "eyJ…" : t("Kept (paste a new one to replace it)")}
          />
        </Field>
        {Object.keys(fields).length === 0 && <ErrorNote error={error} />}
        <div className="rdp2-dialog-foot">
          <Button type="button" variant="ghost" onClick={props.onClose}>
            {t("Cancel")}
          </Button>
          <Button type="submit" disabled={busy || !token.trim() || (adding && !name.trim())}>
            {busy ? (adding ? t("Adding…") : t("Saving…")) : adding ? t("Add tunnel") : t("Replace token")}
          </Button>
        </div>
      </form>
    </Dialog>
  );
}
