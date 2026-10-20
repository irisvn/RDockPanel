// A service's Redirects, Security, Maintenance and Scripts tabs, like
// EasyPanel's: redirects of its domains (a regular expression and its
// replacement), the users of its HTTP basic authentication, its maintenance
// page, and named commands run in its container. They apply on the next
// deployment (scripts at once).
import { type ReactNode, useState } from "react";
import { type PageProps } from "@rustcms/admin-sdk";
import { Button, Dialog, Input, Switch } from "@rustcms/admin-sdk/ui";
import { EmptyState, ErrorNote, Field, RowActions, Section, TableScroll, useAsk } from "../lib/ui";
import { ResourceError, resources } from "../lib/resources";
import type { ServiceDetail } from "./types";

type Props = PageProps<unknown> & { d: ServiceDetail; onSaved: () => void };

/** A dialog adding one row: its fields, then [Cancel] [Add]. */
function AddDialog(props: {
  title: string;
  t: (s: string) => string;
  submit: string;
  disabled: boolean;
  error: unknown;
  hasFieldErrors: boolean;
  onClose: () => void;
  onSubmit: () => Promise<void>;
  children: ReactNode;
}) {
  const [busy, setBusy] = useState(false);
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
        {!props.hasFieldErrors && <ErrorNote error={props.error} />}
        <div className="rdp2-dialog-foot">
          <Button type="button" variant="ghost" onClick={props.onClose}>
            {props.t("Cancel")}
          </Button>
          <Button type="submit" disabled={busy || props.disabled}>
            {props.submit}
          </Button>
        </div>
      </form>
    </Dialog>
  );
}

export function RedirectsTab(props: Props) {
  const { t, d } = props;
  const [adding, setAdding] = useState(false);
  const [regex, setRegex] = useState("");
  const [replacement, setReplacement] = useState("");
  const [permanent, setPermanent] = useState(false);
  const [fields, setFields] = useState<Record<string, string[]>>({});
  const [error, setError] = useState<unknown>(null);
  const ask = useAsk();
  const can = d.permissions.includes("service_redirects.create");
  const fail = (err: unknown) => {
    if (err instanceof ResourceError && err.status === 422) setFields(err.fieldErrors);
    else setError(err);
  };
  return (
    <Section
      title={t("Redirects")}
      description={t("A regular expression over the request's URL and its replacement (${1} for its first group), on every domain of the service.")}
      flush
      actions={
        can && (
          <Button size="sm" onClick={() => setAdding(true)}>
            {t("Add redirect")}
          </Button>
        )
      }
    >
      {!adding && <div className="rdp2-flush-note"><ErrorNote error={error} /></div>}
      {d.redirects.length === 0 ? (
        <EmptyState title={t("No redirect yet")} hint={t("Send old addresses to new ones, or every visitor to www.")} />
      ) : (
        <TableScroll>
          <table className="rdp2-table">
            <thead>
              <tr>
                <th>{t("From (regular expression)")}</th>
                <th>{t("To")}</th>
                <th className="rdp2-hide-sm">{t("Kind")}</th>
                <th>{t("On")}</th>
                <th className="rdp2-right">
                  <span className="rdp2-sr">{t("Actions")}</span>
                </th>
              </tr>
            </thead>
            <tbody>
              {d.redirects.map((r) => (
                <tr key={r.id}>
                  <td>
                    <span className="rdp2-mono rdp2-ellipsis rdp2-cell-clip" title={r.regex}>
                      {r.regex}
                    </span>
                  </td>
                  <td>
                    <span className="rdp2-mono rdp2-ellipsis rdp2-cell-clip" title={r.replacement}>
                      {r.replacement}
                    </span>
                  </td>
                  <td className="rdp2-hide-sm">{r.permanent ? t("permanent") : t("temporary")}</td>
                  <td>
                    <Switch
                      checked={r.enabled}
                      disabled={!d.permissions.includes("service_redirects.update")}
                      aria-label={t("On")}
                      onCheckedChange={async (enabled) => {
                        try {
                          await resources.update("service_redirects", r.id, { enabled });
                          props.onSaved();
                        } catch (err) {
                          setError(err);
                        }
                      }}
                    />
                  </td>
                  <td>
                    <RowActions
                      label={t("Actions for {host}").replace("{host}", r.regex)}
                      menu={
                        d.permissions.includes("service_redirects.delete")
                          ? [
                              {
                                label: t("Remove…"),
                                destructive: true,
                                onSelect: async () => {
                                  const ok = await ask({ title: t("Remove this redirect?"), body: <code>{r.regex}</code>, action: t("Remove redirect"), destructive: true });
                                  if (!ok) return;
                                  try {
                                    await resources.remove("service_redirects", r.id);
                                    props.onSaved();
                                  } catch (err) {
                                    setError(err);
                                  }
                                },
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
      )}
      {adding && (
        <AddDialog
          t={t}
          title={t("Add redirect")}
          submit={t("Add redirect")}
          disabled={!regex.trim() || !replacement.trim()}
          error={error}
          hasFieldErrors={Object.keys(fields).length > 0}
          onClose={() => setAdding(false)}
          onSubmit={async () => {
            setFields({});
            setError(null);
            try {
              await resources.create("service_redirects", { service_id: String(d.service.id), regex, replacement, permanent, enabled: true });
              setRegex("");
              setReplacement("");
              setAdding(false);
              props.onSaved();
            } catch (err) {
              fail(err);
            }
          }}
        >
          <Field label={t("Regular expression")} error={fields.regex}>
            <Input autoFocus className="rdp2-mono" value={regex} onChange={(e) => setRegex(e.target.value)} placeholder="^https?://example.com/(.*)" />
          </Field>
          <Field label={t("Replacement")} error={fields.replacement}>
            <Input className="rdp2-mono" value={replacement} onChange={(e) => setReplacement(e.target.value)} placeholder="https://www.example.com/${1}" />
          </Field>
          <label className="rdp2-row">
            <Switch checked={permanent} onCheckedChange={setPermanent} /> {t("Permanent (301)")}
          </label>
        </AddDialog>
      )}
    </Section>
  );
}

export function SecurityTab(props: Props) {
  const { t, d } = props;
  const [adding, setAdding] = useState(false);
  const [username, setUsername] = useState("");
  const [password, setPassword] = useState("");
  const [error, setError] = useState<unknown>(null);
  const ask = useAsk();
  const fields = error instanceof ResourceError ? error.fieldErrors : {};
  const base = `/apps/services/${d.service.id}/basic-auth`;
  return (
    <Section
      title={t("Basic authentication")}
      description={t("Visitors of the service's domains sign in with one of these users. Passwords are stored hashed.")}
      flush
      actions={
        d.can.update && (
          <Button size="sm" onClick={() => setAdding(true)}>
            {t("Add user")}
          </Button>
        )
      }
    >
      {!adding && <div className="rdp2-flush-note"><ErrorNote error={error} /></div>}
      {d.basic_auth.length === 0 ? (
        <EmptyState title={t("Open to everyone")} hint={t("Add a user to ask visitors for a name and a password.")} />
      ) : (
        <TableScroll>
          <table className="rdp2-table">
            <thead>
              <tr>
                <th>{t("User")}</th>
                <th className="rdp2-right">
                  <span className="rdp2-sr">{t("Actions")}</span>
                </th>
              </tr>
            </thead>
            <tbody>
              {d.basic_auth.map((u) => (
                <tr key={u.id}>
                  <td className="rdp2-strong-cell">{u.username}</td>
                  <td>
                    <RowActions
                      label={t("Actions for {host}").replace("{host}", u.username)}
                      menu={
                        d.can.update
                          ? [
                              {
                                label: t("Remove…"),
                                destructive: true,
                                onSelect: async () => {
                                  const ok = await ask({ title: t("Remove the user {name}?").replace("{name}", u.username), body: t("They can no longer sign in at the next deployment."), action: t("Remove user"), destructive: true });
                                  if (!ok) return;
                                  try {
                                    await props.api.delete(`${base}/${u.id}`);
                                    props.onSaved();
                                  } catch (err) {
                                    setError(err);
                                  }
                                },
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
      )}
      {adding && (
        <AddDialog
          t={t}
          title={t("Add user")}
          submit={t("Add user")}
          disabled={!username.trim() || password.length < 8}
          error={error}
          hasFieldErrors={Object.keys(fields).length > 0}
          onClose={() => setAdding(false)}
          onSubmit={async () => {
            setError(null);
            try {
              await props.api.post(base, { username: username.trim(), password });
              setUsername("");
              setPassword("");
              setAdding(false);
              props.onSaved();
            } catch (err) {
              setError(err);
            }
          }}
        >
          <Field label={t("User")} error={fields.username}>
            <Input autoFocus value={username} onChange={(e) => setUsername(e.target.value)} />
          </Field>
          <Field label={t("Password")} help={t("8 characters or more")} error={fields.password}>
            <Input type="password" autoComplete="new-password" value={password} onChange={(e) => setPassword(e.target.value)} />
          </Field>
        </AddDialog>
      )}
    </Section>
  );
}

/** Maintenance mode: the domains show the panel's page (title, subtitle). */
export function MaintenanceTab(props: Props) {
  const { t, d } = props;
  const s = d.service;
  const [on, setOn] = useState(s.maintenance);
  const [title, setTitle] = useState(s.maintenance_title ?? "");
  const [subtitle, setSubtitle] = useState(s.maintenance_subtitle ?? "");
  const [error, setError] = useState<unknown>(null);
  const fields = error instanceof ResourceError ? error.fieldErrors : {};
  return (
    <Section
      title={t("Maintenance")}
      description={t("While on, the service's domains show this page (503) instead of the service. Deploy to apply.")}
    >
      <form
        className="rdp2-form"
        onSubmit={async (e) => {
          e.preventDefault();
          setError(null);
          try {
            await resources.update("services", s.id, {
              maintenance: on,
              maintenance_title: title.trim() || null,
              maintenance_subtitle: subtitle.trim() || null,
            });
            props.onSaved();
          } catch (err) {
            setError(err);
          }
        }}
      >
        <label className="rdp2-row">
          <Switch checked={on} onCheckedChange={setOn} disabled={!d.can.update} /> {t("Maintenance mode")}
        </label>
        <Field label={t("Title")} error={fields.maintenance_title}>
          <Input value={title} onChange={(e) => setTitle(e.target.value)} placeholder={t("Under maintenance")} disabled={!d.can.update} />
        </Field>
        <Field label={t("Subtitle")} error={fields.maintenance_subtitle}>
          <Input value={subtitle} onChange={(e) => setSubtitle(e.target.value)} placeholder={t("We will be back soon.")} disabled={!d.can.update} />
        </Field>
        {Object.keys(fields).length === 0 && <ErrorNote error={error} />}
        {d.can.update && (
          <div className="rdp2-row rdp2-end">
            <Button type="submit">{t("Save")}</Button>
          </div>
        )}
      </form>
    </Section>
  );
}

/** Scripts, like EasyPanel's: named shell commands run in a running
 * container of the service; their output shown. */
export function ScriptsTab(props: Props) {
  const { t, d } = props;
  const [adding, setAdding] = useState(false);
  const [name, setName] = useState("");
  const [command, setCommand] = useState("");
  const [running, setRunning] = useState<number | null>(null);
  const [result, setResult] = useState<{ name: string; exit_code: number | null; output: string } | null>(null);
  const [error, setError] = useState<unknown>(null);
  const ask = useAsk();
  const fields = error instanceof ResourceError ? error.fieldErrors : {};
  const canRun = d.permissions.includes("services.exec") || d.permissions.includes("docker.exec");
  const run = async (s: ServiceDetail["scripts"][number]) => {
    setRunning(s.id);
    setError(null);
    try {
      const out = await props.api.post<{ exit_code: number | null; output: string }>(`/apps/scripts/${s.id}/run`, { command: s.command });
      setResult({ name: s.name, ...out });
    } catch (err) {
      setError(err);
    } finally {
      setRunning(null);
    }
  };
  return (
    <>
      <Section
        title={t("Scripts")}
        description={t("Shell commands run with sh -c in a running container of the service (at most 25 seconds).")}
        flush
        actions={
          d.permissions.includes("service_scripts.create") && (
            <Button size="sm" onClick={() => setAdding(true)}>
              {t("Add script")}
            </Button>
          )
        }
      >
        {!adding && Object.keys(fields).length === 0 && <div className="rdp2-flush-note"><ErrorNote error={error} /></div>}
        {d.scripts.length === 0 ? (
          <EmptyState title={t("No script yet")} hint={t("Save a command you run often, such as a migration.")} />
        ) : (
          <TableScroll>
            <table className="rdp2-table">
              <thead>
                <tr>
                  <th>{t("Name")}</th>
                  <th>{t("Command")}</th>
                  <th className="rdp2-right">
                    <span className="rdp2-sr">{t("Actions")}</span>
                  </th>
                </tr>
              </thead>
              <tbody>
                {d.scripts.map((s) => (
                  <tr key={s.id}>
                    <td className="rdp2-strong-cell">{s.name}</td>
                    <td>
                      <span className="rdp2-mono rdp2-ellipsis rdp2-cell-clip" title={s.command}>
                        {s.command}
                      </span>
                    </td>
                    <td>
                      <RowActions
                        label={t("Actions for {host}").replace("{host}", s.name)}
                        menu={
                          d.permissions.includes("service_scripts.delete")
                            ? [
                                {
                                  label: t("Remove…"),
                                  destructive: true,
                                  onSelect: async () => {
                                    const ok = await ask({ title: t("Remove the script {name}?").replace("{name}", s.name), action: t("Remove script"), destructive: true });
                                    if (!ok) return;
                                    try {
                                      await resources.remove("service_scripts", s.id);
                                      props.onSaved();
                                    } catch (err) {
                                      setError(err);
                                    }
                                  },
                                },
                              ]
                            : []
                        }
                      >
                        {canRun && (
                          <Button size="sm" variant="ghost" disabled={running !== null || d.service.status !== "running"} onClick={() => run(s)}>
                            {running === s.id ? t("Running…") : t("Run")}
                          </Button>
                        )}
                      </RowActions>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </TableScroll>
        )}
      </Section>
      {result && (
        <Section title={`${result.name}`} description={`${t("exit code")} ${result.exit_code ?? "?"}`}>
          <pre className="rdp2-log">{result.output}</pre>
        </Section>
      )}
      {adding && (
        <AddDialog
          t={t}
          title={t("Add script")}
          submit={t("Add script")}
          disabled={!name.trim() || !command.trim()}
          error={error}
          hasFieldErrors={Object.keys(fields).length > 0}
          onClose={() => setAdding(false)}
          onSubmit={async () => {
            setError(null);
            try {
              await resources.create("service_scripts", { service_id: String(d.service.id), name: name.trim(), command });
              setName("");
              setCommand("");
              setAdding(false);
              props.onSaved();
            } catch (err) {
              setError(err);
            }
          }}
        >
          <Field label={t("Name")} error={fields.name}>
            <Input autoFocus value={name} onChange={(e) => setName(e.target.value)} placeholder="Migrate" />
          </Field>
          <Field label={t("Command")} error={fields.command}>
            <Input className="rdp2-mono" value={command} onChange={(e) => setCommand(e.target.value)} placeholder="php artisan migrate --force" />
          </Field>
        </AddDialog>
      )}
    </>
  );
}
