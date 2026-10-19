// Settings → API tokens: the caller's own tokens for the public API and the
// `rdp` command line (docs/api.md). Made with scopes and an expiry, shown
// once; revoked here. A token does what its user may, within its scopes.
import { useState } from "react";
import { fieldErrors, type PageProps, useAdminQuery } from "@rustcms/admin-sdk";
import { Alert, Button, Checkbox, CodeBlock, Dialog, ErrorState, Input, Loading } from "@rustcms/admin-sdk/ui";
import { ErrorNote, Field, Section, when } from "../lib/ui";

type Token = { id: number; name: string; scopes: string[]; created_at: string; last_used_at: string | null; expires_at: string | null };
type List = { tokens: Token[]; scopes: string[]; max_days: number };

const SCOPE_HELP: Record<string, string> = {
  read: "See projects, services, deployments and domains",
  deploy: "Deploy and restart services",
  write: "Change and add services and domains",
  secrets: "Read environment values",
  logs: "Read services' logs",
};

export function Tokens(props: PageProps<unknown>) {
  const t = props.t;
  const list = useAdminQuery<List>(props, "/apps/tokens");
  const [adding, setAdding] = useState(false);
  const [made, setMade] = useState<string | null>(null);
  const [error, setError] = useState<unknown>(null);
  if (list.isLoading) return <Loading />;
  if (list.error || !list.data) return <ErrorState error={list.error} />;
  const data = list.data;
  return (
    <Section
      level={2}
      title={t("API tokens")}
      description={t("For the API (/api/v1) and the rdp command line: a token does what you may, within its scopes.")}
      actions={<Button onClick={() => setAdding(true)}>+ {t("New token")}</Button>}
    >
      <ErrorNote error={error} />
      {made && (
        <div className="rdp2-form">
          <Alert variant="warning">{t("Copy it now: it is not shown again.")}</Alert>
          <CodeBlock code={made} label={t("Token")} />
          <CodeBlock code={`rdp login ${window.location.origin}`} label={t("Command line")} />
        </div>
      )}
      {data.tokens.length === 0 && <p className="rdp2-muted">{t("No token yet.")}</p>}
      <ul className="rdp2-list">
        {data.tokens.map((tok) => (
          <li key={tok.id} className="rdp2-list-row">
            <span className="rdp2-form">
              <strong>{tok.name}</strong>
              <span className="rdp2-small rdp2-muted">
                {tok.scopes.join(", ")} · {t("expires")} {when(tok.expires_at, props.locale)} · {t("last used")} {tok.last_used_at ? when(tok.last_used_at, props.locale) : t("never")}
              </span>
            </span>
            <Button
              size="sm"
              variant="ghost"
              onClick={async () => {
                if (!window.confirm(t("Revoke this token? What uses it stops working."))) return;
                setError(null);
                try {
                  await props.api.delete(`/apps/tokens/${tok.id}`);
                  list.refetch();
                } catch (err) {
                  setError(err);
                }
              }}
            >
              {t("Revoke")}
            </Button>
          </li>
        ))}
      </ul>
      {adding && (
        <NewToken
          {...props}
          scopes={data.scopes}
          maxDays={data.max_days}
          onClose={() => setAdding(false)}
          onMade={(token) => {
            setAdding(false);
            setMade(token);
            list.refetch();
          }}
        />
      )}
    </Section>
  );
}

function NewToken(props: PageProps<unknown> & { scopes: string[]; maxDays: number; onClose: () => void; onMade: (token: string) => void }) {
  const t = props.t;
  const [name, setName] = useState("");
  const [scopes, setScopes] = useState<string[]>(["read"]);
  const [days, setDays] = useState("90");
  const [error, setError] = useState<unknown>(null);
  const fields = error ? fieldErrors(error) : {};
  return (
    <Dialog open onClose={props.onClose} title={t("New token")}>
      <form
        className="rdp2-form"
        onSubmit={async (e) => {
          e.preventDefault();
          setError(null);
          try {
            const out = await props.api.post<{ token: string }>("/apps/tokens", { name, scopes, days: Number(days) });
            props.onMade(out.token);
          } catch (err) {
            setError(err);
          }
        }}
      >
        <Field label={t("Name")} error={fields.name}>
          <Input autoFocus value={name} onChange={(e) => setName(e.target.value)} placeholder="CI" />
        </Field>
        <Field label={t("Scopes")} error={fields.scopes}>
          <div className="rdp2-form">
            {props.scopes.map((scope) => (
              <label key={scope} className="rdp2-row">
                <Checkbox
                  checked={scopes.includes(scope)}
                  onChange={(e) => {
                    const on = e.target.checked;
                    setScopes((old) => (on ? [...old, scope] : old.filter((s) => s !== scope)));
                  }}
                />
                <strong>{scope}</strong> <span className="rdp2-small rdp2-muted">{t(SCOPE_HELP[scope] ?? "")}</span>
              </label>
            ))}
          </div>
        </Field>
        <Field label={t("Expires in (days)")} error={fields.days}>
          <Input type="number" min={1} max={props.maxDays} value={days} onChange={(e) => setDays(e.target.value)} />
        </Field>
        {Object.keys(fields).length === 0 && <ErrorNote error={error} />}
        <div className="rdp2-row rdp2-end">
          <Button type="button" variant="ghost" onClick={props.onClose}>
            {t("Cancel")}
          </Button>
          <Button type="submit" disabled={!name.trim() || scopes.length === 0}>
            {t("Create")}
          </Button>
        </div>
      </form>
    </Dialog>
  );
}
