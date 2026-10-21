// A project's members: users of the panel given a role in it — viewer
// (sees it), developer (changes, deploys and runs its services, their
// secrets and terminal), admin (also removes services, restores databases,
// chooses members). Its admins manage them; the project keeps one admin.
import { useState } from "react";
import { type PageProps } from "@rustcms/admin-sdk";
import { useAdminQuery } from "../lib/query";
import { Button, Dialog, ErrorState, Input, Loading, NativeSelect } from "@rustcms/admin-sdk/ui";
import { ErrorNote, RowActions, useAsk } from "../lib/ui";
import type { Project } from "./types";

type Member = { user_id: number; name: string | null; email: string | null; role: string };
type List = { members: Member[]; roles: string[] };

export function Members(props: PageProps<unknown> & { project: Project; onClose: () => void }) {
  const ask = useAsk();
  const t = props.t;
  const base = `/apps/projects/${props.project.id}/members`;
  const list = useAdminQuery<List>(props, base);
  const [email, setEmail] = useState("");
  const [role, setRole] = useState("developer");
  const [found, setFound] = useState<Array<{ id: number; name: string; email: string }>>([]);
  const [error, setError] = useState<unknown>(null);
  const call = async (run: () => Promise<unknown>) => {
    setError(null);
    try {
      await run();
      list.refetch();
    } catch (err) {
      setError(err);
    }
  };
  return (
    <Dialog open onClose={props.onClose} title={`${t("Members")} · ${props.project.name}`} className="rdp2-wide">
      {list.isLoading && <Loading />}
      {list.error && <ErrorState error={list.error} />}
      <p className="rdp2-small rdp2-muted">
        {t("Viewer: sees the project. Developer: changes, deploys and runs its services (their secrets and terminal). Admin: also removes services, restores databases and chooses members.")}
      </p>
      <ul className="rdp2-list">
        {list.data?.members.map((m) => (
          <li key={m.user_id} className="rdp2-list-row">
            <span>
              <strong>{m.name ?? m.email}</strong> <span className="rdp2-muted rdp2-small">{m.email}</span>
            </span>
            <span className="rdp2-row">
              <NativeSelect value={m.role} onChange={(e) => call(() => props.api.patch(`${base}/${m.user_id}`, { role: e.target.value }))}>
                {list.data?.roles.map((r) => (
                  <option key={r} value={r}>
                    {t(r)}
                  </option>
                ))}
              </NativeSelect>
              <RowActions
                label={t("Actions for {host}").replace("{host}", m.email ?? "")}
                menu={[
                  {
                    label: t("Remove…"),
                    destructive: true,
                    onSelect: async () => {
                      const ok = await ask({
                        title: t("Remove {name} from this project?").replace("{name}", m.name ?? m.email ?? ""),
                        body: t("They lose what their role gave them in this project."),
                        action: t("Remove member"),
                        destructive: true,
                      });
                      if (ok) void call(() => props.api.delete(`${base}/${m.user_id}`));
                    },
                  },
                ]}
              />
            </span>
          </li>
        ))}
        {list.data && list.data.members.length === 0 && <li className="rdp2-muted">{t("No member yet: only users whose roles see every project see it.")}</li>}
      </ul>
      <form
        className="rdp2-row rdp2-wrap"
        onSubmit={(e) => {
          e.preventDefault();
          void call(async () => {
            await props.api.post(base, { email, role });
            setEmail("");
            setFound([]);
          });
        }}
      >
        <Input
          className="rdp2-grow"
          type="email"
          list="rdp2-member-users"
          value={email}
          placeholder={t("A user's email")}
          onChange={async (e) => {
            setEmail(e.target.value);
            // Suggestions for whoever may see the users; else the exact email.
            if (props.can("users.view_any") && e.target.value.trim().length >= 2) {
              try {
                const r = await props.api.get<{ users: Array<{ id: number; name: string; email: string }> }>("/apps/users", { query: { q: e.target.value } });
                setFound(r.users);
              } catch {
                setFound([]);
              }
            }
          }}
        />
        <datalist id="rdp2-member-users">
          {found.map((u) => (
            <option key={u.id} value={u.email}>
              {u.name}
            </option>
          ))}
        </datalist>
        <NativeSelect value={role} onChange={(e) => setRole(e.target.value)}>
          {(list.data?.roles ?? ["viewer", "developer", "admin"]).map((r) => (
            <option key={r} value={r}>
              {t(r)}
            </option>
          ))}
        </NativeSelect>
        <Button type="submit" disabled={!email.trim()}>
          {t("Add")}
        </Button>
      </form>
      <ErrorNote error={error} />
    </Dialog>
  );
}
