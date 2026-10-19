// Settings → Git: GitHub tokens, like EasyPanel's GitHub settings. Added
// once GitHub accepts them, sealed, never shown again; each given to the
// projects whose services may clone with it (their developers then pick a
// repository among those it reaches). The panel's administrators'.
import { useState } from "react";
import { fieldErrors, type PageProps, useAdminQuery } from "@rustcms/admin-sdk";
import { Button, Checkbox, Dialog, ErrorState, Input, Loading } from "@rustcms/admin-sdk/ui";
import { ErrorNote, Field, Section, when } from "../lib/ui";

type Provider = { id: number; name: string; kind: string; account: string | null; projects: number[]; used_by: number; updated_at: string };
type List = { providers: Provider[]; projects: Array<{ id: number; name: string }> };

function ProviderForm(props: PageProps<unknown> & { list: List; provider?: Provider; onClose: () => void; onSaved: () => void }) {
  const t = props.t;
  const p = props.provider;
  const [name, setName] = useState(p?.name ?? "");
  const [token, setToken] = useState("");
  const [projects, setProjects] = useState<number[]>(p?.projects ?? []);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<unknown>(null);
  const fields = error ? fieldErrors(error) : {};
  return (
    <Dialog open onClose={props.onClose} title={p ? t("Edit GitHub token") : t("Add a GitHub token")}>
      <form
        className="rdp2-form"
        onSubmit={async (e) => {
          e.preventDefault();
          setBusy(true);
          setError(null);
          try {
            const body: Record<string, unknown> = { name, projects };
            if (token.trim() || !p) body.token = token;
            if (p) await props.api.patch(`/apps/git/providers/${p.id}`, body);
            else await props.api.post("/apps/git/providers", body);
            props.onSaved();
          } catch (err) {
            setError(err);
          } finally {
            setBusy(false);
          }
        }}
      >
        <Field label={t("Name")} error={fields.name}>
          <Input autoFocus value={name} onChange={(e) => setName(e.target.value)} placeholder="company" />
        </Field>
        <Field
          label={t("Token")}
          help={p ? t("Leave empty to keep it.") : t("A fine-grained token with read access to Contents and Metadata of the repositories, or a classic one with repo. GitHub checks it first.")}
          error={fields.token}
        >
          <Input type="password" autoComplete="new-password" value={token} onChange={(e) => setToken(e.target.value)} placeholder="github_pat_…" />
        </Field>
        <Field label={t("Projects that may use it")} help={t("Their developers can list and clone every repository this token reaches.")}>
          <div className="rdp2-form">
            {props.list.projects.length === 0 && <span className="rdp2-muted rdp2-small">{t("No project yet.")}</span>}
            {props.list.projects.map((project) => (
              <label key={project.id} className="rdp2-row">
                <Checkbox
                  checked={projects.includes(project.id)}
                  onChange={(e) => {
                    const on = e.target.checked;
                    setProjects((old) => (on ? [...old, project.id] : old.filter((x) => x !== project.id)));
                  }}
                />
                {project.name}
              </label>
            ))}
          </div>
        </Field>
        {Object.keys(fields).length === 0 && <ErrorNote error={error} />}
        <div className="rdp2-row rdp2-end">
          <Button type="button" variant="ghost" onClick={props.onClose}>
            {t("Cancel")}
          </Button>
          <Button type="submit" disabled={busy || !name.trim() || (!p && !token.trim())}>
            {p ? t("Save") : t("Add")}
          </Button>
        </div>
      </form>
    </Dialog>
  );
}

export function Git(props: PageProps<unknown>) {
  const t = props.t;
  const list = useAdminQuery<List>(props, "/apps/git/providers");
  const [editing, setEditing] = useState<Provider | "new" | null>(null);
  const [error, setError] = useState<unknown>(null);
  if (list.isLoading) return <Loading />;
  if (list.error || !list.data) return <ErrorState error={list.error} />;
  const data = list.data;
  const names = (ids: number[]) => data.projects.filter((p) => ids.includes(p.id)).map((p) => p.name).join(", ");
  return (
    <Section
      level={2}
      title="GitHub"
      description={t("Tokens that clone private repositories, each for the projects you give it. Never shown again once saved.")}
      actions={<Button onClick={() => setEditing("new")}>+ {t("Add a GitHub token")}</Button>}
    >
      <ErrorNote error={error} />
      {data.providers.length === 0 && <p className="rdp2-muted">{t("No token: services clone public repositories, or private ones with a deploy key.")}</p>}
      <ul className="rdp2-list">
        {data.providers.map((p) => (
          <li key={p.id} className="rdp2-list-row">
            <span className="rdp2-form">
              <strong>{p.name}</strong>
              <span className="rdp2-small rdp2-muted">
                {p.account ? `@${p.account}` : t("account unknown")} · {p.projects.length ? names(p.projects) : t("no project")} · {p.used_by} {t("service(s)")} ·{" "}
                {when(p.updated_at, props.locale)}
              </span>
            </span>
            <span className="rdp2-row">
              <Button
                size="sm"
                variant="ghost"
                onClick={async () => {
                  setError(null);
                  try {
                    const r = await props.api.post<{ account: string; scopes: string | null }>(`/apps/git/providers/${p.id}/test`, {});
                    props.toast.success(`${t("GitHub accepts it")}: @${r.account}${r.scopes ? ` (${r.scopes})` : ""}`);
                    list.refetch();
                  } catch (err) {
                    setError(err);
                  }
                }}
              >
                {t("Test")}
              </Button>
              <Button size="sm" variant="ghost" onClick={() => setEditing(p)}>
                {t("Edit")}
              </Button>
              <Button
                size="sm"
                variant="ghost"
                disabled={p.used_by > 0}
                title={p.used_by > 0 ? t("Services clone with it") : undefined}
                onClick={async () => {
                  if (!window.confirm(t("Remove this token?"))) return;
                  setError(null);
                  try {
                    await props.api.delete(`/apps/git/providers/${p.id}`);
                    list.refetch();
                  } catch (err) {
                    setError(err);
                  }
                }}
              >
                {t("Remove")}
              </Button>
            </span>
          </li>
        ))}
      </ul>
      {editing && (
        <ProviderForm
          {...props}
          list={data}
          provider={editing === "new" ? undefined : editing}
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
