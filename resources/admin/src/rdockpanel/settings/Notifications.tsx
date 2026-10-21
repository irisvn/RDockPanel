// Settings → Notifications, like EasyPanel's: channels (Discord, Slack,
// Telegram, email, webhook), the events each gets, and "Send test
// notification". The URL, bot token or SMTP password is never shown again.
import { useState } from "react";
import { type PageProps } from "@rustcms/admin-sdk";
import { useAdminQuery } from "../lib/query";
import { Button, Dialog, ErrorState, Input, Loading, NativeSelect, Switch } from "@rustcms/admin-sdk/ui";
import { EmptyState, ErrorNote, Field, type MenuItem, RowActions, Section, TableScroll, useAsk } from "../lib/ui";
import { ResourceError, resources } from "../lib/resources";

type Channel = {
  id: number;
  name: string;
  kind: string;
  enabled: boolean;
  on_deploy: boolean;
  on_backup: boolean;
  on_disk: boolean;
  disk_threshold: number;
  /** The events it hears (names of EVENTS). */
  events: string[];
  chat_id: string | null;
  smtp_host: string | null;
  smtp_port: number | null;
  smtp_username: string | null;
  smtp_from: string | null;
  email_to: string | null;
  secret_set: boolean;
};

/** What a channel may hear, and whether a new one does. */
const EVENTS: Array<{ name: string; label: string; short: string; on: boolean }> = [
  { name: "deploy", label: "App deployment (and every failure)", short: "deployments", on: true },
  { name: "backup", label: "Database backup and restore", short: "backups", on: true },
  { name: "disk", label: "Disk usage at or over", short: "disk", on: true },
  { name: "service", label: "A running service fails, and when it is back", short: "services", on: true },
  { name: "traefik", label: "Traefik stops, and when it is back", short: "Traefik", on: true },
  { name: "certificate", label: "An uploaded certificate expires soon", short: "certificates", on: true },
  { name: "update", label: "A newer RDockPanel is out", short: "updates", on: true },
  { name: "prune", label: "Docker pruned", short: "prunes", on: false },
  { name: "login", label: "A sign-in from a new address", short: "sign-ins", on: false },
];

const KINDS: Array<[string, string]> = [
  ["discord", "Discord"],
  ["slack", "Slack"],
  ["telegram", "Telegram"],
  ["email", "Email (SMTP)"],
  ["webhook", "Webhook"],
];

function ChannelForm(props: PageProps<unknown> & { channel: Channel | null; onClose: () => void; onSaved: () => void }) {
  const t = props.t;
  const c = props.channel;
  const [name, setName] = useState(c?.name ?? "");
  const [kind, setKind] = useState(c?.kind ?? "discord");
  const [secret, setSecret] = useState("");
  const [chatId, setChatId] = useState(c?.chat_id ?? "");
  const [host, setHost] = useState(c?.smtp_host ?? "");
  const [port, setPort] = useState(String(c?.smtp_port ?? 587));
  const [user, setUser] = useState(c?.smtp_username ?? "");
  const [from, setFrom] = useState(c?.smtp_from ?? "");
  const [to, setTo] = useState(c?.email_to ?? "");
  const [enabled, setEnabled] = useState(c?.enabled ?? true);
  const [events, setEvents] = useState<string[]>(c?.events ?? EVENTS.filter((e) => e.on).map((e) => e.name));
  const hears = (name: string) => events.includes(name);
  const toggle = (name: string) => (on: boolean) => setEvents((old) => (on ? [...old.filter((e) => e !== name), name] : old.filter((e) => e !== name)));
  const [threshold, setThreshold] = useState(String(c?.disk_threshold ?? 90));
  const [fields, setFields] = useState<Record<string, string[]>>({});
  const [error, setError] = useState<unknown>(null);
  const secretLabel = kind === "telegram" ? t("Bot access token") : kind === "email" ? t("SMTP password") : t("Webhook URL");
  return (
    <Dialog open onClose={props.onClose} title={c ? t("Edit channel") : t("Add channel")}>
      <form
        className="rdp2-form"
        onSubmit={async (e) => {
          e.preventDefault();
          setFields({});
          setError(null);
          const data: Record<string, unknown> = {
            name: name.trim(),
            kind,
            enabled,
            events: JSON.stringify(EVENTS.map((e) => e.name).filter(hears)),
            // As before `events` (kept in step).
            on_deploy: hears("deploy"),
            on_backup: hears("backup"),
            on_disk: hears("disk"),
            disk_threshold: Number(threshold),
          };
          if (kind === "telegram") data.chat_id = chatId.trim();
          if (kind === "email")
            Object.assign(data, {
              smtp_host: host.trim(),
              smtp_port: Number(port),
              smtp_username: user.trim() || null,
              smtp_from: from.trim(),
              email_to: to.trim(),
            });
          if (secret) data.secret = { replace: secret.trim() };
          try {
            if (c) await resources.update("notification_channels", c.id, data);
            else await resources.create("notification_channels", data);
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
          <NativeSelect value={kind} onChange={(e) => setKind(e.target.value)} disabled={!!c}>
            {KINDS.map(([v, label]) => (
              <option key={v} value={v}>
                {label}
              </option>
            ))}
          </NativeSelect>
        </Field>
        <Field label={secretLabel} help={c?.secret_set ? t("Set: leave empty to keep it") : undefined} error={fields.secret}>
          <Input type="password" autoComplete="new-password" value={secret} onChange={(e) => setSecret(e.target.value)} />
        </Field>
        {kind === "telegram" && (
          <Field label={t("Chat ID")} help={t("The chat's id (a group's starts with -100)")} error={fields.chat_id}>
            <Input value={chatId} onChange={(e) => setChatId(e.target.value)} />
          </Field>
        )}
        {kind === "email" && (
          <>
            <div className="rdp2-row">
              <Field label={t("SMTP server")} error={fields.smtp_host}>
                <Input value={host} onChange={(e) => setHost(e.target.value)} placeholder="smtp.example.com" />
              </Field>
              <Field label={t("Port")} error={fields.smtp_port}>
                <Input type="number" value={port} onChange={(e) => setPort(e.target.value)} />
              </Field>
            </div>
            <Field label={t("SMTP user")} error={fields.smtp_username}>
              <Input value={user} onChange={(e) => setUser(e.target.value)} />
            </Field>
            <Field label={t("From")} error={fields.smtp_from}>
              <Input value={from} onChange={(e) => setFrom(e.target.value)} placeholder="panel@example.com" />
            </Field>
            <Field label={t("To")} help={t("Recipients, comma-separated")} error={fields.email_to}>
              <Input value={to} onChange={(e) => setTo(e.target.value)} />
            </Field>
          </>
        )}
        <fieldset className="rdp2-form">
          <legend className="rdp2-field-label">{t("Events")}</legend>
          {EVENTS.map((e) => (
            <label key={e.name} className="rdp2-row">
              <Switch checked={hears(e.name)} onCheckedChange={toggle(e.name)} /> {t(e.label)}
              {e.name === "disk" && (
                <>
                  <Input className="rdp2-narrow" type="number" min={1} max={100} value={threshold} onChange={(ev) => setThreshold(ev.target.value)} /> %
                </>
              )}
            </label>
          ))}
          {fields.events && <p className="rdp2-small rdp2-error">{fields.events.join(" ")}</p>}
        </fieldset>
        <label className="rdp2-row">
          <Switch checked={enabled} onCheckedChange={setEnabled} /> {t("Enabled")}
        </label>
        <ErrorNote error={error} />
        <div className="rdp2-row rdp2-end">
          <Button type="submit">{c ? t("Save") : t("Create")}</Button>
        </div>
      </form>
    </Dialog>
  );
}

export function Notifications(props: PageProps<unknown>) {
  const t = props.t;
  const list = useAdminQuery<{ channels: Channel[] }>(props, "/apps/notification-channels");
  const [editing, setEditing] = useState<Channel | "new" | null>(null);
  const [tested, setTested] = useState<Record<number, { ok: boolean; error?: string } | "busy">>({});
  const [error, setError] = useState<unknown>(null);
  const ask = useAsk();
  if (list.isLoading) return <Loading />;
  if (list.error || !list.data) return <ErrorState error={list.error} />;
  return (
    <Section
      flush
      title={t("Notifications")}
      description={t("Where the panel says what happened: deployments, backups, failing services, Traefik, a full disk, updates.")}
      actions={props.can("notification_channels.create") && <Button onClick={() => setEditing("new")}>{t("Add channel")}</Button>}
    >
      {error != null && (
        <div className="rdp2-flush-note">
          <ErrorNote error={error} />
        </div>
      )}
      {list.data.channels.length === 0 ? (
        <EmptyState title={t("No channel yet.")} hint={t("Add Discord, Slack, Telegram, email or a webhook to hear what happens.")} />
      ) : (
        <TableScroll>
          <table className="rdp2-table">
            <thead>
              <tr>
                <th>{t("Name")}</th>
                <th>{t("Type")}</th>
                <th className="rdp2-hide-sm">{t("Events")}</th>
                <th className="rdp2-right">
                  <span className="rdp2-sr">{t("Actions")}</span>
                </th>
              </tr>
            </thead>
            <tbody>
              {list.data.channels.map((c) => {
                const result = tested[c.id];
                const events = EVENTS.filter((e) => c.events.includes(e.name))
                  .map((e) => (e.name === "disk" ? `${t("disk")} ≥ ${c.disk_threshold}%` : t(e.short)))
                  .join(", ");
                const menu: MenuItem[] = [];
                if (props.can("notification_channels.update")) menu.push({ label: t("Edit"), onSelect: () => setEditing(c) });
                if (props.can("notification_channels.delete"))
                  menu.push("separator", {
                    label: t("Delete…"),
                    destructive: true,
                    onSelect: async () => {
                      const ok = await ask({ title: t("Delete the channel {name}?").replace("{name}", c.name), action: t("Delete channel"), destructive: true });
                      if (!ok) return;
                      setError(null);
                      try {
                        await resources.remove("notification_channels", c.id);
                        list.refetch();
                      } catch (err) {
                        setError(err);
                      }
                    },
                  });
                return (
                  <tr key={c.id}>
                    <td>
                      <div className="rdp2-strong-cell">{c.name}</div>
                      {!c.enabled && <div className="rdp2-muted rdp2-small">{t("off")}</div>}
                    </td>
                    <td>{KINDS.find(([v]) => v === c.kind)?.[1] ?? c.kind}</td>
                    <td className="rdp2-hide-sm rdp2-small">{events}</td>
                    <td>
                      <RowActions label={t("Actions for {host}").replace("{host}", c.name)} menu={menu}>
                        {result && result !== "busy" && (
                          <span className={result.ok ? "rdp2-small" : "rdp2-error rdp2-small"}>{result.ok ? t("Sent") : result.error}</span>
                        )}
                        {props.can("notification_channels.update") && (
                          <Button
                            size="sm"
                            variant="ghost"
                            disabled={result === "busy"}
                            onClick={async () => {
                              setTested((s) => ({ ...s, [c.id]: "busy" }));
                              try {
                                const out = await props.api.post<{ ok: boolean; error?: string }>(`/apps/notification-channels/${c.id}/test`, {});
                                setTested((s) => ({ ...s, [c.id]: out }));
                              } catch (err) {
                                setTested((s) => ({ ...s, [c.id]: { ok: false, error: err instanceof Error ? err.message : String(err) } }));
                              }
                            }}
                          >
                            {result === "busy" ? t("Sending…") : t("Send test")}
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
        <ChannelForm
          {...props}
          channel={editing === "new" ? null : editing}
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
