// A service's Redirects, Security, Maintenance and Scripts tabs, like
// EasyPanel's: redirects of its domains (a regular expression and its
// replacement), its rate limit and the users of its HTTP basic
// authentication, its maintenance page, and named commands run in its
// container. They apply when saved.
import { type ReactNode, useState } from "react";
import { type PageProps } from "@rustcms/admin-sdk";
import { Alert, Button, Dialog, Input, NativeSelect, Switch, Textarea } from "@rustcms/admin-sdk/ui";
import { EmptyState, ErrorNote, Field, FilePicker, RowActions, Section, Stack, TableScroll, useAsk } from "../lib/ui";
import { HtmlEditor, Preview, fromLocalInput, localInput } from "./MaintenancePage";
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

type Period = "1s" | "1m" | "1h";
/** The limits offered: requests per second on average, and at once. */
const PRESETS: Record<"normal" | "strict", { average: number; period: Period; burst: number }> = {
  normal: { average: 50, period: "1s", burst: 200 },
  strict: { average: 10, period: "1s", burst: 30 },
};

/** Which choice a saved limit is (a preset's numbers: that preset). */
function presetOf(s: ServiceDetail["service"]): "off" | "normal" | "strict" | "custom" {
  if (s.rate_limit_average == null) return "off";
  for (const name of ["normal", "strict"] as const) {
    const p = PRESETS[name];
    if (p.average === s.rate_limit_average && p.period === (s.rate_limit_period ?? "1s") && p.burst === s.rate_limit_burst) return name;
  }
  return "custom";
}

/** The rate limit: requests by visitor (their IP), on every domain. */
function RateLimitSection(props: Props) {
  const { t, d } = props;
  const s = d.service;
  const [choice, setChoice] = useState(presetOf(s));
  const [average, setAverage] = useState(String(s.rate_limit_average ?? 50));
  const [period, setPeriod] = useState<Period>(s.rate_limit_period ?? "1s");
  const [burst, setBurst] = useState(String(s.rate_limit_burst ?? 200));
  const [exempt, setExempt] = useState(s.rate_limit_exempt ?? "");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<unknown>(null);
  const fields = error instanceof ResourceError ? error.fieldErrors : {};
  const per: Record<Period, string> = { "1s": t("second"), "1m": t("minute"), "1h": t("hour") };
  const shown = choice === "normal" || choice === "strict" ? PRESETS[choice] : null;
  return (
    <Section
      title={t("Rate limit")}
      description={t("Limits how often one visitor (their IP address) sends requests to the service's domains. Over the limit, they get 429 Too Many Requests. Applied when saved.")}
    >
      <form
        className="rdp2-form"
        onSubmit={async (e) => {
          e.preventDefault();
          setError(null);
          const limit =
            choice === "off"
              ? { rate_limit_average: null, rate_limit_period: null, rate_limit_burst: null }
              : shown
                ? { rate_limit_average: shown.average, rate_limit_period: shown.period, rate_limit_burst: shown.burst }
                : { rate_limit_average: Number(average), rate_limit_period: period, rate_limit_burst: Number(burst) };
          setBusy(true);
          try {
            await resources.update("services", s.id, { ...limit, rate_limit_exempt: exempt.trim() || null });
            props.onSaved();
          } catch (err) {
            setError(err);
          } finally {
            setBusy(false);
          }
        }}
      >
        {d.domains.length === 0 && <Alert>{t("The service has no domain yet: the limit applies once it has one.")}</Alert>}
        <Field
          label={t("Limit")}
          help={
            choice === "off"
              ? undefined
              : [
                  shown && t("{average} requests a second on average, bursts of {burst}").replace("{average}", String(shown.average)).replace("{burst}", String(shown.burst)) + ".",
                  t("Counted per visitor and per domain. Visitors through Cloudflare (a proxied record or the tunnel) are counted by their own IP; IPv6 visitors by their /64."),
                ]
                  .filter(Boolean)
                  .join(" ")
          }
          error={choice === "custom" ? undefined : fields.rate_limit_average}
        >
          <NativeSelect value={choice} onChange={(e) => setChoice(e.target.value as typeof choice)} disabled={!d.can.update}>
            <option value="off">{t("Off")}</option>
            <option value="normal">{t("Normal (websites)")}</option>
            <option value="strict">{t("Strict (APIs)")}</option>
            <option value="custom">{t("Custom")}</option>
          </NativeSelect>
        </Field>
        {choice === "custom" && (
          <div className="rdp2-grid2">
            <Field label={t("Requests on average")} help={t("1 to 100000")} error={fields.rate_limit_average}>
              <div className="rdp2-row">
                <Input type="number" min={1} max={100000} value={average} onChange={(e) => setAverage(e.target.value)} disabled={!d.can.update} />
                <span className="rdp2-muted">{t("per")}</span>
                <NativeSelect aria-label={t("Period")} value={period} onChange={(e) => setPeriod(e.target.value as Period)} disabled={!d.can.update}>
                  {(["1s", "1m", "1h"] as const).map((p) => <option key={p} value={p}>{per[p]}</option>)}
                </NativeSelect>
              </div>
            </Field>
            <Field label={t("Burst")} help={t("Requests a visitor may send at once (a page with many images and scripts needs more)")} error={fields.rate_limit_burst}>
              <Input type="number" min={1} max={100000} value={burst} onChange={(e) => setBurst(e.target.value)} disabled={!d.can.update} />
            </Field>
          </div>
        )}
        {choice !== "off" && (
          <Field
            label={t("Exempt addresses")}
            help={t("IPs or networks never limited, one a line (a VPN, a monitor). They match visitors reaching the server directly, not through Cloudflare.")}
            error={fields.rate_limit_exempt}
          >
            <Textarea className="rdp2-mono" rows={3} value={exempt} placeholder={"203.0.113.7\n10.8.0.0/24"} onChange={(e) => setExempt(e.target.value)} disabled={!d.can.update} />
          </Field>
        )}
        {Object.keys(fields).length === 0 && <ErrorNote error={error} />}
        {d.can.update && (
          <div className="rdp2-row rdp2-end">
            <Button type="submit" disabled={busy}>
              {busy ? t("Saving…") : t("Save")}
            </Button>
          </div>
        )}
      </form>
    </Section>
  );
}

export function SecurityTab(props: Props) {
  return (
    <Stack>
      <RateLimitSection {...props} />
      <BasicAuthSection {...props} />
    </Stack>
  );
}

function BasicAuthSection(props: Props) {
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
                                  const ok = await ask({ title: t("Remove the user {name}?").replace("{name}", u.username), body: t("They can no longer sign in, at once."), action: t("Remove user"), destructive: true });
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

/** File bytes as base64 (without the data URL's prefix). */
function base64Of(file: File): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => resolve(String(reader.result).split(",", 2)[1] ?? "");
    reader.onerror = () => reject(reader.error);
    reader.readAsDataURL(file);
  });
}

/** Maintenance mode: the domains show a page (503) instead of the service:
 * the default one (the panel's shared page, else the built-in one) or the
 * service's own HTML; the addresses let through still reach the service. */
export function MaintenanceTab(props: Props) {
  const { t, d } = props;
  const s = d.service;
  const [on, setOn] = useState(s.maintenance);
  const [kind, setKind] = useState<"default" | "custom">(s.maintenance_page ?? "default");
  const [title, setTitle] = useState(s.maintenance_title ?? "");
  const [subtitle, setSubtitle] = useState(s.maintenance_subtitle ?? "");
  const [eta, setEta] = useState(localInput(s.maintenance_eta));
  const [contact, setContact] = useState(s.maintenance_contact ?? "");
  const [html, setHtml] = useState(s.maintenance_html ?? "");
  const [bypass, setBypass] = useState(s.maintenance_bypass ?? "");
  const [logo, setLogo] = useState<string | null>(d.maintenance.logo);
  // Bumped when the logo changes: the preview is made again.
  const [logoRevision, setLogoRevision] = useState(0);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<unknown>(null);
  const ask = useAsk();
  const fields = error instanceof ResourceError ? error.fieldErrors : {};
  const when = fromLocalInput(eta);
  const can = d.can.update;
  const hosts = d.domains.map((x) => x.host).filter((h) => !h.startsWith("*."));
  const uploadLogo = async (file: File) => {
    setError(null);
    try {
      const made = await props.api.put<{ logo: string }>(`/apps/services/${s.id}/maintenance-logo`, { data: await base64Of(file) });
      setLogo(made.logo);
      setLogoRevision((n) => n + 1);
    } catch (err) {
      setError(err);
    }
  };
  const removeLogo = async () => {
    setError(null);
    try {
      await props.api.delete(`/apps/services/${s.id}/maintenance-logo`);
      setLogo(null);
      setLogoRevision((n) => n + 1);
    } catch (err) {
      setError(err);
    }
  };
  return (
    <Section
      title={t("Maintenance")}
      description={t("While on, the service's domains show this page (503) instead of the service. Applied when saved.")}
    >
      <form
        className="rdp2-form"
        onSubmit={async (e) => {
          e.preventDefault();
          setError(null);
          // Turned on: the site goes down for its visitors.
          if (on && !s.maintenance && d.domains.length > 0) {
            const ok = await ask({
              title: t("Turn maintenance on?"),
              body: t("Visitors of the service's domains get the maintenance page (503) until it is turned off; the addresses let through still reach it."),
              action: t("Turn maintenance on"),
              destructive: true,
            });
            if (!ok) return;
          }
          setBusy(true);
          try {
            await resources.update("services", s.id, {
              maintenance: on,
              maintenance_page: kind,
              maintenance_title: title.trim() || null,
              maintenance_subtitle: subtitle.trim() || null,
              maintenance_eta: when.eta,
              maintenance_eta_offset: when.eta ? when.offset : null,
              maintenance_contact: contact.trim() || null,
              maintenance_html: html.trim() ? html : null,
              maintenance_bypass: bypass.trim() || null,
            });
            props.onSaved();
          } catch (err) {
            setError(err);
          } finally {
            setBusy(false);
          }
        }}
      >
        <label className="rdp2-row">
          <Switch checked={on} onCheckedChange={setOn} disabled={!can} /> {t("Maintenance mode")}
        </label>
        {s.maintenance && hosts.length > 0 && (
          <p className="rdp2-muted">
            {t("Visitors of these domains see the page:")}{" "}
            {hosts.map((h, i) => (
              <span key={h}>
                {i > 0 && ", "}
                <a href={`https://${h}`} target="_blank" rel="noreferrer">{h}</a>
              </span>
            ))}
          </p>
        )}
        <div className="rdp2-maint">
          <div className="rdp2-form">
            <Field
              label={t("Page")}
              help={kind === "default" ? (d.maintenance.shared ? t("The panel's shared page (Settings → Maintenance page).") : t("The built-in page.")) : undefined}
              error={fields.maintenance_page}
            >
              <NativeSelect value={kind} onChange={(e) => setKind(e.target.value as "default" | "custom")} disabled={!can}>
                <option value="default">{t("Default")}</option>
                <option value="custom">{t("Its own HTML")}</option>
              </NativeSelect>
            </Field>
            <Field label={t("Title")} error={fields.maintenance_title}>
              <Input value={title} onChange={(e) => setTitle(e.target.value)} placeholder={t("Under maintenance")} disabled={!can} />
            </Field>
            <Field label={t("Subtitle")} error={fields.maintenance_subtitle}>
              <Input value={subtitle} onChange={(e) => setSubtitle(e.target.value)} placeholder={t("We will be back soon.")} disabled={!can} />
            </Field>
            <div className="rdp2-grid2">
              <Field label={t("Back at")} help={t("In your time; empty: not shown")} error={fields.maintenance_eta}>
                <Input type="datetime-local" value={eta} onChange={(e) => setEta(e.target.value)} disabled={!can} />
              </Field>
              <Field label={t("Contact")} help={t("An email or an https link")} error={fields.maintenance_contact}>
                <Input value={contact} onChange={(e) => setContact(e.target.value)} placeholder="support@example.com" disabled={!can} />
              </Field>
            </div>
            <Field label={t("Logo")} help={t("PNG, JPEG or WebP up to 256 KB. None: the panel's logo (Branding).")} error={fields.data}>
              <div className="rdp2-row rdp2-wrap">
                {logo && <img src={logo} alt="" className="rdp2-brand-preview" />}
                {can && <FilePicker label={logo ? t("Replace") : t("Upload")} accept="image/png,image/jpeg,image/webp" onFile={(file) => void uploadLogo(file)} />}
                {can && logo && (
                  <Button type="button" variant="ghost" size="sm" onClick={() => void removeLogo()}>
                    {t("Use the panel's logo")}
                  </Button>
                )}
              </div>
            </Field>
            {kind === "custom" && (
              <HtmlEditor t={t} value={html} onChange={setHtml} starter={d.maintenance.starter} placeholders={d.maintenance.placeholders} disabled={!can} error={fields.maintenance_html} />
            )}
            <Field
              label={t("Addresses that still reach the service")}
              help={t("IPs or networks, one a line (you, your team, a VPN): they see the service as usual. Through Cloudflare, single addresses only.")}
              error={fields.maintenance_bypass}
            >
              <Textarea className="rdp2-mono" rows={3} value={bypass} placeholder={"203.0.113.7\n10.8.0.0/24"} onChange={(e) => setBypass(e.target.value)} disabled={!can} />
            </Field>
          </div>
          <Preview
            {...props}
            label={t("Preview")}
            revision={String(logoRevision)}
            body={{
              service_id: s.id,
              title,
              subtitle,
              eta: when.eta,
              eta_offset: when.offset,
              contact,
              page: kind,
              html: kind === "custom" ? html : undefined,
            }}
          />
        </div>
        {Object.keys(fields).length === 0 && <ErrorNote error={error} />}
        {can && (
          <div className="rdp2-row rdp2-end">
            <Button type="submit" disabled={busy}>
              {busy ? t("Saving…") : t("Save")}
            </Button>
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
