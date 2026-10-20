// Settings → Git: GitHub tokens, like EasyPanel's GitHub settings. Added
// once GitHub accepts them, sealed, never shown again; each given to the
// projects whose services may clone with it (their developers then pick a
// repository among those it reaches). The panel's administrators'.
import { useState } from "react";
import { fieldErrors, type PageProps, useAdminQuery } from "@rustcms/admin-sdk";
import { Button, Checkbox, Dialog, ErrorState, Input, Loading } from "@rustcms/admin-sdk/ui";
import { EmptyState, ErrorNote, Field, RowActions, Section, TableScroll, useAsk, when } from "../lib/ui";

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
  const ask = useAsk();
  if (list.isLoading) return <Loading />;
  if (list.error || !list.data) return <ErrorState error={list.error} />;
  const data = list.data;
  const names = (ids: number[]) => data.projects.filter((p) => ids.includes(p.id)).map((p) => p.name).join(", ");
  return (
    <Section
      title="GitHub"
      description={t("Tokens that clone private repositories, each for the projects you give it. Never shown again once saved.")}
      flush
      actions={<Button onClick={() => setEditing("new")}>{t("Add GitHub token")}</Button>}
    >
      {error != null && (
        <div className="rdp2-flush-note">
          <ErrorNote error={error} />
        </div>
      )}
      {data.providers.length === 0 ? (
        <EmptyState title={t("No GitHub token yet")} hint={t("Services clone public repositories, or private ones with a deploy key.")} />
      ) : (
        <TableScroll>
          <table className="rdp2-table">
            <thead>
              <tr>
                <th>{t("Name")}</th>
                <th>{t("Projects")}</th>
                <th className="rdp2-hide-sm">{t("Used by")}</th>
                <th className="rdp2-right">
                  <span className="rdp2-sr">{t("Actions")}</span>
                </th>
              </tr>
            </thead>
            <tbody>
              {data.providers.map((p) => (
                <tr key={p.id}>
                  <td>
                    <div className="rdp2-strong-cell">{p.name}</div>
                    <div className="rdp2-muted rdp2-small">{p.account ? `@${p.account}` : t("account unknown")}</div>
                  </td>
                  <td>{p.projects.length ? names(p.projects) : <span className="rdp2-muted">{t("no project")}</span>}</td>
                  <td className="rdp2-hide-sm rdp2-num">
                    {p.used_by} {t("service(s)")}
                  </td>
                  <td>
                    <RowActions
                      label={t("Actions for {host}").replace("{host}", p.name)}
                      menu={[
                        { label: t("Edit"), onSelect: () => setEditing(p) },
                        "separator",
                        {
                          label: p.used_by > 0 ? t("Remove (services clone with it)") : t("Remove…"),
                          destructive: true,
                          disabled: p.used_by > 0,
                          onSelect: async () => {
                            const ok = await ask({ title: t("Remove the GitHub token {name}?").replace("{name}", p.name), action: t("Remove token"), destructive: true });
                            if (!ok) return;
                            setError(null);
                            try {
                              await props.api.delete(`/apps/git/providers/${p.id}`);
                              list.refetch();
                            } catch (err) {
                              setError(err);
                            }
                          },
                        },
                      ]}
                    >
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
                    </RowActions>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </TableScroll>
      )}
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
