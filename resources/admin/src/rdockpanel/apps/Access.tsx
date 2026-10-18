// A service's Redirects and Security tabs, like EasyPanel's: redirects of
// its domains (a regular expression and its replacement) and the users of
// its HTTP basic authentication. Both apply on the next deployment.
import { useState } from "react";
import { type PageProps } from "@rustcms/admin-sdk";
import { Button, Input, Switch } from "@rustcms/admin-sdk/ui";
import { ErrorNote, Field, Section } from "../lib/ui";
import { ResourceError, resources } from "../lib/resources";
import type { ServiceDetail } from "./types";

type Props = PageProps<unknown> & { d: ServiceDetail; onSaved: () => void };

export function RedirectsTab(props: Props) {
  const { t, d } = props;
  const [regex, setRegex] = useState("");
  const [replacement, setReplacement] = useState("");
  const [permanent, setPermanent] = useState(false);
  const [fields, setFields] = useState<Record<string, string[]>>({});
  const [error, setError] = useState<unknown>(null);
  const can = props.can("service_redirects.create");
  const fail = (err: unknown) => {
    if (err instanceof ResourceError && err.status === 422) setFields(err.fieldErrors);
    else setError(err);
  };
  return (
    <Section
      title={t("Redirects")}
      description={t("A regular expression over the request's URL and its replacement (${1} for its first group), on every domain of the service.")}
    >
      <ErrorNote error={error} />
      {d.redirects.length === 0 && <p className="rdp2-muted">{t("No redirect.")}</p>}
      <ul className="rdp2-list">
        {d.redirects.map((r) => (
          <li key={r.id} className="rdp2-row">
            <code className="rdp2-ellipsis">{r.regex}</code>
            <span className="rdp2-muted">→</span>
            <code className="rdp2-ellipsis">{r.replacement}</code>
            <span className="rdp2-muted rdp2-small">{r.permanent ? t("permanent") : t("temporary")}</span>
            {props.can("service_redirects.update") && (
              <Switch
                checked={r.enabled}
                onCheckedChange={async (enabled) => {
                  try {
                    await resources.update("service_redirects", r.id, { enabled });
                    props.onSaved();
                  } catch (err) {
                    setError(err);
                  }
                }}
              />
            )}
            {props.can("service_redirects.delete") && (
              <Button
                size="sm"
                variant="ghost"
                onClick={async () => {
                  try {
                    await resources.remove("service_redirects", r.id);
                    props.onSaved();
                  } catch (err) {
                    setError(err);
                  }
                }}
              >
                {t("Remove")}
              </Button>
            )}
          </li>
        ))}
      </ul>
      {can && (
        <form
          className="rdp2-form"
          onSubmit={async (e) => {
            e.preventDefault();
            setFields({});
            setError(null);
            try {
              await resources.create("service_redirects", {
                service_id: String(d.service.id),
                regex,
                replacement,
                permanent,
                enabled: true,
              });
              setRegex("");
              setReplacement("");
              props.onSaved();
            } catch (err) {
              fail(err);
            }
          }}
        >
          <div className="rdp2-grid2">
            <Field label={t("Regular expression")} error={fields.regex}>
              <Input className="rdp2-mono" value={regex} onChange={(e) => setRegex(e.target.value)} placeholder="^https?://example.com/(.*)" />
            </Field>
            <Field label={t("Replacement")} error={fields.replacement}>
              <Input className="rdp2-mono" value={replacement} onChange={(e) => setReplacement(e.target.value)} placeholder="https://www.example.com/${1}" />
            </Field>
          </div>
          <label className="rdp2-row">
            <Switch checked={permanent} onCheckedChange={setPermanent} /> {t("Permanent (301)")}
          </label>
          <div className="rdp2-row rdp2-end">
            <Button type="submit" disabled={!regex.trim() || !replacement.trim()}>
              {t("Add")}
            </Button>
          </div>
        </form>
      )}
    </Section>
  );
}

export function SecurityTab(props: Props) {
  const { t, d } = props;
  const [username, setUsername] = useState("");
  const [password, setPassword] = useState("");
  const [error, setError] = useState<unknown>(null);
  const fields = error instanceof ResourceError ? error.fieldErrors : {};
  const base = `/apps/services/${d.service.id}/basic-auth`;
  return (
    <Section
      title={t("Basic authentication")}
      description={t("Visitors of the service's domains sign in with one of these users. Passwords are stored hashed.")}
    >
      {d.basic_auth.length === 0 && <p className="rdp2-muted">{t("Open to everyone.")}</p>}
      <ul className="rdp2-list">
        {d.basic_auth.map((u) => (
          <li key={u.id} className="rdp2-row">
            <strong>{u.username}</strong>
            {d.can.update && (
              <Button
                size="sm"
                variant="ghost"
                onClick={async () => {
                  try {
                    await props.api.delete(`${base}/${u.id}`);
                    props.onSaved();
                  } catch (err) {
                    setError(err);
                  }
                }}
              >
                {t("Remove")}
              </Button>
            )}
          </li>
        ))}
      </ul>
      {d.can.update && (
        <form
          className="rdp2-form"
          onSubmit={async (e) => {
            e.preventDefault();
            setError(null);
            try {
              await props.api.post(base, { username: username.trim(), password });
              setUsername("");
              setPassword("");
              props.onSaved();
            } catch (err) {
              setError(err);
            }
          }}
        >
          <div className="rdp2-grid2">
            <Field label={t("User")} error={fields.username}>
              <Input value={username} onChange={(e) => setUsername(e.target.value)} />
            </Field>
            <Field label={t("Password")} help={t("8 characters or more")} error={fields.password}>
              <Input type="password" autoComplete="new-password" value={password} onChange={(e) => setPassword(e.target.value)} />
            </Field>
          </div>
          {Object.keys(fields).length === 0 && <ErrorNote error={error} />}
          <div className="rdp2-row rdp2-end">
            <Button type="submit" disabled={!username.trim() || password.length < 8}>
              {t("Add user")}
            </Button>
          </div>
        </form>
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
