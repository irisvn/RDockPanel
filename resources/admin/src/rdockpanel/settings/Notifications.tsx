// Settings → Notifications, like EasyPanel's: channels (Discord, Slack,
// Telegram, email, webhook), the events each gets, and "Send test
// notification". The URL, bot token or SMTP password is never shown again.
import { useState } from "react";
import { type PageProps, useAdminQuery } from "@rustcms/admin-sdk";
import { Button, Dialog, ErrorState, Input, Loading, NativeSelect, Switch } from "@rustcms/admin-sdk/ui";
import { ErrorNote, Field, Section } from "../lib/ui";
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
  chat_id: string | null;
  smtp_host: string | null;
  smtp_port: number | null;
  smtp_username: string | null;
  smtp_from: string | null;
  email_to: string | null;
  secret_set: boolean;
};

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
  const [onDeploy, setOnDeploy] = useState(c?.on_deploy ?? true);
  const [onBackup, setOnBackup] = useState(c?.on_backup ?? true);
  const [onDisk, setOnDisk] = useState(c?.on_disk ?? true);
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
            on_deploy: onDeploy,
            on_backup: onBackup,
            on_disk: onDisk,
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
          <label className="rdp2-row">
            <Switch checked={onDeploy} onCheckedChange={setOnDeploy} /> {t("App deployment (and every failure)")}
          </label>
          <label className="rdp2-row">
            <Switch checked={onBackup} onCheckedChange={setOnBackup} /> {t("Database backup and restore")}
          </label>
          <label className="rdp2-row">
            <Switch checked={onDisk} onCheckedChange={setOnDisk} /> {t("Disk usage at or over")}
            <Input className="rdp2-narrow" type="number" min={1} max={100} value={threshold} onChange={(e) => setThreshold(e.target.value)} /> %
          </label>
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
  if (list.isLoading) return <Loading />;
  if (list.error || !list.data) return <ErrorState error={list.error} />;
  return (
    <Section
      level={2}
      title={t("Notifications")}
      description={t("Where the panel says what happened: deployments, backups, a full disk.")}
      actions={props.can("notification_channels.create") && <Button onClick={() => setEditing("new")}>{t("Add channel")}</Button>}
    >
      <ErrorNote error={error} />
      {list.data.channels.length === 0 && <p className="rdp2-muted">{t("No channel yet.")}</p>}
      {list.data.channels.length > 0 && (
        <table className="rdp2-table">
          <thead>
            <tr>
              <th>{t("Name")}</th>
              <th>{t("Type")}</th>
              <th>{t("Events")}</th>
              <th />
            </tr>
          </thead>
          <tbody>
            {list.data.channels.map((c) => {
              const result = tested[c.id];
              const events = [c.on_deploy && t("deployments"), c.on_backup && t("backups"), c.on_disk && `${t("disk")} ≥ ${c.disk_threshold}%`]
                .filter(Boolean)
                .join(", ");
              return (
                <tr key={c.id}>
                  <td>
                    {c.name}
                    {!c.enabled && <span className="rdp2-muted"> · {t("off")}</span>}
                  </td>
                  <td>{KINDS.find(([v]) => v === c.kind)?.[1] ?? c.kind}</td>
                  <td className="rdp2-small">{events}</td>
                  <td className="rdp2-row rdp2-end">
                    {result && result !== "busy" && (
                      <span className={result.ok ? "rdp2-small" : "rdp2-error rdp2-small"}>{result.ok ? t("Sent") : result.error}</span>
                    )}
                    {props.can("notification_channels.update") && (
                      <Button
                        size="sm"
                        variant="outline"
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
                        {t("Send test notification")}
                      </Button>
                    )}
                    {props.can("notification_channels.update") && (
                      <Button size="sm" variant="ghost" onClick={() => setEditing(c)}>
                        {t("Edit")}
                      </Button>
                    )}
                    {props.can("notification_channels.delete") && (
                      <Button
                        size="sm"
                        variant="ghost"
                        onClick={async () => {
                          if (!window.confirm(t("Delete this channel?"))) return;
                          setError(null);
                          try {
                            await resources.remove("notification_channels", c.id);
                            list.refetch();
                          } catch (err) {
                            setError(err);
                          }
                        }}
                      >
                        {t("Delete")}
                      </Button>
                    )}
                  </td>
                </tr>
              );
            })}
          </tbody>
        </table>
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
