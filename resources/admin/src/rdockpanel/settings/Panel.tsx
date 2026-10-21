// Settings → Panel: the panel's domain, its Let's Encrypt email and the
// apps' domain, as the installer set them; new ones are checked here (DNS, a
// taken name) and applied by the installer command shown (run on the
// server: the panel never changes its own service, Traefik's resolver and
// its route itself — no way back if it did not come up). The panel's
// administrators' (`system.manage`).
import { useState } from "react";
import { type PageProps } from "@rustcms/admin-sdk";
import { useAdminQuery } from "../lib/query";
import { Alert, Button, CodeBlock, ErrorState, Input, Loading } from "@rustcms/admin-sdk/ui";
import { ErrorNote, Facts, Field, Section } from "../lib/ui";
import { Controls, SystemFooter } from "./Controls";

type Current = {
  url: string;
  installed: boolean;
  domain: string | null;
  email: string | null;
  apps_domain: string | null;
  public_ip: string | null;
  https: boolean;
};

type Checked = {
  errors: Record<string, string>;
  warnings: string[];
  dns: Array<{ name: string; addresses: string[]; ok: boolean }>;
  command: string | null;
};

export function Panel(props: PageProps<unknown>) {
  const t = props.t;
  const current = useAdminQuery<Current>(props, "/apps/panel");
  const [domain, setDomain] = useState<string | null>(null);
  const [email, setEmail] = useState<string | null>(null);
  const [apps, setApps] = useState<string | null>(null);
  const [checked, setChecked] = useState<Checked | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<unknown>(null);
  if (current.isLoading) return <Loading />;
  if (current.error || !current.data) return <ErrorState error={current.error} />;
  const c = current.data;
  const values = {
    domain: domain ?? c.domain ?? "",
    email: email ?? c.email ?? "",
    // Development's `localhost` is no domain to give the installer.
    apps_domain: apps ?? (c.apps_domain === "localhost" ? "" : (c.apps_domain ?? "")),
  };
  const edit = (set: (v: string) => void) => (e: React.ChangeEvent<HTMLInputElement>) => {
    set(e.target.value);
    setChecked(null);
  };
  return (
    <>
    <Section
      title={t("Panel")}
      description={t("The panel's domain, its Let's Encrypt email and the apps' domain. Changed by the installer: check new ones here, then run the command shown on the server.")}
    >
      {!c.installed && (
        <Alert>{t("This panel runs from source (development), not from install.sh: its address comes from APP_URL. The command below is for a server installed with install.sh.")}</Alert>
      )}
      <Facts
        items={[
          [t("Panel address"), c.url],
          [t("Let's Encrypt email"), c.email ?? t("none")],
          [t("Apps' domain"), c.apps_domain ? `<project>-<service>.${c.apps_domain}` : t("none (no generated domains)")],
          [t("Server address"), c.public_ip ?? t("unknown")],
        ]}
      />
      <form
        className="rdp2-form"
        onSubmit={async (e) => {
          e.preventDefault();
          setBusy(true);
          setError(null);
          setChecked(null);
          try {
            setChecked(await props.api.post<Checked>("/apps/panel/check", values));
          } catch (err) {
            setError(err);
          } finally {
            setBusy(false);
          }
        }}
      >
        <div className="rdp2-grid2">
          <Field
            label={t("Panel domain")}
            help={t("Empty: the installed panel answers at http://<server-ip>:3000. Point the domain's DNS (A record) to the server first.")}
            error={checked?.errors.domain ? [checked.errors.domain] : undefined}
          >
            <Input value={values.domain} placeholder="panel.example.com" onChange={edit(setDomain)} disabled={busy} />
          </Field>
          <Field
            label={t("Let's Encrypt email")}
            help={t("Its account: the panel's and the apps' certificates")}
            error={checked?.errors.email ? [checked.errors.email] : undefined}
          >
            <Input type="email" value={values.email} placeholder="ops@example.com" onChange={edit(setEmail)} disabled={busy} />
          </Field>
        </div>
        <Field
          label={t("Apps' domain")}
          help={t("Generated domains <project>-<service>.<this>: a wildcard DNS record *.<this> to this server. Existing domains stay as they are.")}
          error={checked?.errors.apps_domain ? [checked.errors.apps_domain] : undefined}
        >
          <Input value={values.apps_domain} placeholder="apps.example.com" onChange={edit(setApps)} disabled={busy} />
        </Field>
        <ErrorNote error={error} />
        <div className="rdp2-row rdp2-end">
          <Button type="submit" disabled={busy}>
            {t("Check")}
          </Button>
        </div>
      </form>
      {checked && (
        <div className="rdp2-form">
          {checked.dns.map((d) => (
            <p key={d.name} className={d.ok ? "rdp2-small" : "rdp2-small rdp2-error"}>
              {d.ok ? "✓" : "⚠"} <span className="rdp2-mono">{d.name}</span> →{" "}
              {d.addresses.length ? d.addresses.join(", ") : t("does not resolve")}
            </p>
          ))}
          {checked.warnings.map((w) => (
            <Alert key={w} variant="warning">
              {w}
            </Alert>
          ))}
          {checked.command && (
            <>
              <p className="rdp2-small">
                {t("Run this on the server (as root). The panel restarts at its new address; the installer keeps everything else.")}
              </p>
              <CodeBlock code={checked.command} label={t("Installer command")} />
            </>
          )}
        </div>
      )}
    </Section>
    <Controls {...props} />
    <SystemFooter {...props} />
    </>
  );
}
