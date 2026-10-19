// Settings → Certificates: Traefik's certificate resolvers (Let's Encrypt
// over HTTP, or over DNS with a provider's token: the only way to a wildcard
// certificate), and "Apply" (Traefik restarts with them). The panel's
// administrators' (`system.manage`). Written through the `cert_resolvers`
// resource (the token sealed, never shown again); a domain chooses one in
// its dialog.
import { useState } from "react";
import { type PageProps, useAdminQuery } from "@rustcms/admin-sdk";
import { Alert, Button, Dialog, ErrorState, Input, Loading, NativeSelect, Switch } from "@rustcms/admin-sdk/ui";
import { ErrorNote, Field, Section } from "../lib/ui";
import { ResourceError, resources } from "../lib/resources";

type Resolver = {
  id: number;
  name: string;
  kind: string;
  provider: string | null;
  email: string;
  staging: boolean;
  has_token: boolean;
  in_use: boolean;
};

type Status = {
  resolvers: Resolver[];
  providers: Array<{ name: string; label: string }>;
  default: string;
  applied: boolean | null;
};

function ResolverForm(
  props: PageProps<unknown> & { resolver: Resolver | null; providers: Status["providers"]; onClose: () => void; onSaved: () => void },
) {
  const t = props.t;
  const r = props.resolver;
  const [name, setName] = useState(r?.name ?? "");
  const [kind, setKind] = useState(r?.kind ?? "dns");
  const [provider, setProvider] = useState(r?.provider ?? props.providers[0]?.name ?? "");
  const [email, setEmail] = useState(r?.email ?? "");
  const [token, setToken] = useState("");
  const [staging, setStaging] = useState(r?.staging ?? false);
  const [fields, setFields] = useState<Record<string, string[]>>({});
  const [error, setError] = useState<unknown>(null);
  return (
    <Dialog open onClose={props.onClose} title={r ? t("Edit certificate resolver") : t("Add certificate resolver")}>
      <form
        className="rdp2-form"
        onSubmit={async (e) => {
          e.preventDefault();
          setFields({});
          setError(null);
          const data: Record<string, unknown> = {
            name: name.trim(),
            kind,
            email: email.trim(),
            staging,
            provider: kind === "dns" ? provider : null,
          };
          if (kind === "dns" && token) data.token = { replace: token };
          // An HTTP resolver keeps no token (a DNS one turned HTTP: removed).
          if (kind === "http" && r?.has_token) data.token = { clear: true };
          try {
            if (r) await resources.update("cert_resolvers", r.id, data);
            else await resources.create("cert_resolvers", data);
            props.onSaved();
          } catch (err) {
            if (err instanceof ResourceError && err.status === 422) setFields(err.fieldErrors);
            else setError(err);
          }
        }}
      >
        <Field label={t("Name")} help={t("Traefik's name for it: lowercase letters and digits, e.g. cloudflare")} error={fields.name}>
          <Input autoFocus value={name} onChange={(e) => setName(e.target.value)} disabled={!!r} />
        </Field>
        <Field label={t("Challenge")} error={fields.kind}>
          <NativeSelect value={kind} onChange={(e) => setKind(e.target.value)}>
            <option value="dns">{t("DNS (wildcards too): a token of your DNS provider")}</option>
            <option value="http">{t("HTTP: the domain must reach this server on port 80")}</option>
          </NativeSelect>
        </Field>
        {kind === "dns" && (
          <>
            <Field label={t("DNS provider")} error={fields.provider}>
              <NativeSelect value={provider} onChange={(e) => setProvider(e.target.value)}>
                {props.providers.map((p) => (
                  <option key={p.name} value={p.name}>
                    {p.label}
                  </option>
                ))}
              </NativeSelect>
            </Field>
            <Field
              label={t("API token")}
              help={r?.has_token ? t("Set: leave empty to keep it") : t("Kept sealed; Traefik reads it as a Docker secret")}
              error={fields.token}
            >
              <Input type="password" autoComplete="new-password" value={token} onChange={(e) => setToken(e.target.value)} />
            </Field>
          </>
        )}
        <Field label={t("Email")} help={t("Let's Encrypt's account: expiry notices")} error={fields.email}>
          <Input type="email" value={email} onChange={(e) => setEmail(e.target.value)} />
        </Field>
        <label className="rdp2-row">
          <Switch checked={staging} onCheckedChange={setStaging} /> {t("Let's Encrypt staging (untrusted certificates, for tests)")}
        </label>
        <ErrorNote error={error} />
        <div className="rdp2-row rdp2-end">
          <Button type="submit">{r ? t("Save") : t("Create")}</Button>
        </div>
      </form>
    </Dialog>
  );
}

export function Certificates(props: PageProps<unknown>) {
  const t = props.t;
  const status = useAdminQuery<Status>(props, "/apps/traefik/resolvers");
  const [editing, setEditing] = useState<Resolver | "new" | null>(null);
  const [applying, setApplying] = useState(false);
  const [error, setError] = useState<unknown>(null);
  if (status.isLoading) return <Loading />;
  if (status.error || !status.data) return <ErrorState error={status.error} />;
  const s = status.data;
  return (
    <Section
      level={2}
      title={t("Certificate resolvers")}
      description={t("Certificate resolvers of Traefik: a domain chooses one in its dialog (else the panel's default). A wildcard needs a DNS one.")}
      actions={<Button onClick={() => setEditing("new")}>{t("Add certificate resolver")}</Button>}
    >
      <p className="rdp2-muted rdp2-small">
        {t("Default")}: <span className="rdp2-mono">{s.default || t("Traefik's own certificate (development)")}</span>
      </p>
      {s.applied === false && (
        <Alert variant="warning">
          <div className="rdp2-row">
            <span>{t("Traefik does not run these resolvers yet: apply them (Traefik restarts, a few seconds without routing).")}</span>
            <Button
              size="sm"
              disabled={applying}
              onClick={async () => {
                setApplying(true);
                setError(null);
                try {
                  await props.api.post("/apps/traefik/apply", {});
                  props.toast.success(t("Applied: Traefik restarts with them."));
                  status.refetch();
                } catch (err) {
                  setError(err);
                } finally {
                  setApplying(false);
                }
              }}
            >
              {t("Apply")}
            </Button>
          </div>
        </Alert>
      )}
      <ErrorNote error={error} />
      {s.resolvers.length === 0 && <p className="rdp2-muted">{t("No certificate resolver of the panel's yet.")}</p>}
      {s.resolvers.length > 0 && (
        <table className="rdp2-table">
          <thead>
            <tr>
              <th>{t("Name")}</th>
              <th>{t("Challenge")}</th>
              <th>{t("Email")}</th>
              <th />
            </tr>
          </thead>
          <tbody>
            {s.resolvers.map((r) => (
              <tr key={r.id}>
                <td className="rdp2-mono">{r.name}</td>
                <td>
                  {r.kind === "dns" ? `DNS · ${s.providers.find((p) => p.name === r.provider)?.label ?? r.provider}` : "HTTP"}
                  {r.staging && <span className="rdp2-muted"> · {t("staging")}</span>}
                </td>
                <td className="rdp2-small rdp2-ellipsis">{r.email}</td>
                <td className="rdp2-row rdp2-end">
                  <Button size="sm" variant="ghost" onClick={() => setEditing(r)}>
                    {t("Edit")}
                  </Button>
                  <Button
                    size="sm"
                    variant="ghost"
                    disabled={r.in_use}
                    title={r.in_use ? t("A domain uses it") : undefined}
                    onClick={async () => {
                      if (!window.confirm(t("Delete this certificate resolver? Apply afterwards."))) return;
                      setError(null);
                      try {
                        await resources.remove("cert_resolvers", r.id);
                        status.refetch();
                      } catch (err) {
                        setError(err);
                      }
                    }}
                  >
                    {t("Delete")}
                  </Button>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
      {editing && (
        <ResolverForm
          {...props}
          resolver={editing === "new" ? null : editing}
          providers={s.providers}
          onClose={() => setEditing(null)}
          onSaved={() => {
            setEditing(null);
            status.refetch();
          }}
        />
      )}
    </Section>
  );
}
