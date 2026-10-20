// A database's Backups tab, like EasyPanel's: its backup configurations
// (database, schedule, provider, path, retention) with "Run now", the last
// runs with their logs, and Restore (a file of a provider, into a database).
import { useRef, useState } from "react";
import { type PageProps, useAdminQuery } from "@rustcms/admin-sdk";
import { Alert, Button, Dialog, ErrorState, Input, Loading, NativeSelect, Switch } from "@rustcms/admin-sdk/ui";
import { bytes, EmptyState, ErrorNote, Field, type MenuItem, NavLink, RowActions, Section, Status, StatusBadge, TableScroll, useAsk, when } from "../lib/ui";
import { dateTime, relative, size } from "../lib/format";
import { ResourceError, resources } from "../lib/resources";
import type { ServiceDetail } from "./types";

type Config = {
  id: number;
  storage_provider_id: number;
  database_name: string;
  volume: string | null;
  enabled: boolean;
  schedule: string;
  retention: number;
  path: string;
  last_slot_at: string | null;
};
type Run = {
  id: number;
  kind: string;
  trigger: string;
  status: string;
  database_name: string;
  file: string | null;
  size: number | null;
  error: string | null;
  created_at: string;
  finished_at: string | null;
  log?: string;
};
type Tab = {
  supported: boolean;
  volumes: string[];
  configs: Config[];
  runs: Run[];
  providers: Array<{ id: number; name: string; kind: string }>;
  /** This service's backups (for a project admin, who browses no provider). */
  restorable: Array<{ storage_provider_id: number; file: string; at: string | null }>;
};

/** What a configuration backs up: "" (the database) or a volume's path. */
function whatOptions(t: (s: string) => string, tab: Tab): Array<[string, string]> {
  return [
    ...(tab.supported ? ([["", t("The database")]] as Array<[string, string]>) : []),
    ...tab.volumes.map((v) => [v, `${t("Volume")} ${v}`] as [string, string]),
  ];
}

/** A volume backup's label in its files: its whole path (`/var/lib/data` →
 * `var-lib-data`), so two volumes never share files (or retention). */
function volumeLabel(path: string): string {
  const name = path
    .split("/")
    .filter(Boolean)
    .join("-")
    .replace(/[^A-Za-z0-9_$-]/g, "-")
    .replace(/^-+/, "");
  // A short hash of the whole path: `/data-a` and `/data/a` stay apart.
  let hash = 0x811c9dc5;
  for (const c of path) hash = Math.imul(hash ^ c.charCodeAt(0), 0x01000193) >>> 0;
  return `${name.slice(0, 55) || "volume"}-${hash.toString(16).padStart(8, "0")}`;
}
type Props = PageProps<unknown> & { d: ServiceDetail };

const SCHEDULES: Array<[string, string]> = [
  ["0 2 * * *", "Daily at 02:00 (UTC)"],
  ["0 */6 * * *", "Every 6 hours"],
  ["0 * * * *", "Hourly"],
  ["0 3 * * 0", "Weekly, Sunday 03:00 (UTC)"],
];

function ConfigForm(props: Props & { tab: Tab; config: Config | null; onClose: () => void; onSaved: () => void }) {
  const { t, d } = props;
  const c = props.config;
  const [provider, setProvider] = useState(String(c?.storage_provider_id ?? props.tab.providers[0]?.id ?? ""));
  const options = whatOptions(t, props.tab);
  const [what, setWhat] = useState(c ? (c.volume ?? "") : (options[0]?.[0] ?? ""));
  const [database, setDatabase] = useState(c?.database_name ?? d.database?.database ?? "");
  const [schedule, setSchedule] = useState(c?.schedule ?? "0 2 * * *");
  const [retention, setRetention] = useState(String(c?.retention ?? 7));
  const [path, setPath] = useState(c?.path ?? `${d.project.name}/${d.service.name}`);
  const [enabled, setEnabled] = useState(c?.enabled ?? true);
  const [fields, setFields] = useState<Record<string, string[]>>({});
  const [error, setError] = useState<unknown>(null);
  return (
    <Dialog open onClose={props.onClose} title={c ? t("Edit backup") : t("Create database backup")}>
      <form
        className="rdp2-form"
        onSubmit={async (e) => {
          e.preventDefault();
          setFields({});
          setError(null);
          const data = {
            storage_provider_id: provider,
            volume: what || null,
            database_name: what ? volumeLabel(what) : database.trim(),
            schedule: schedule.trim(),
            retention: Number(retention),
            path: path.trim(),
            enabled,
          };
          try {
            if (c) await resources.update("database_backups", c.id, data);
            else await resources.create("database_backups", { ...data, service_id: String(d.service.id) });
            props.onSaved();
          } catch (err) {
            if (err instanceof ResourceError && err.status === 422) setFields(err.fieldErrors);
            else setError(err);
          }
        }}
      >
        <Field label={t("Storage provider")} error={fields.storage_provider_id}>
          <NativeSelect value={provider} onChange={(e) => setProvider(e.target.value)}>
            {props.tab.providers.map((p) => (
              <option key={p.id} value={p.id}>
                {p.name} ({p.kind})
              </option>
            ))}
          </NativeSelect>
        </Field>
        <Field label={t("Back up")} error={fields.volume}>
          <NativeSelect value={what} onChange={(e) => setWhat(e.target.value)} disabled={!!c}>
            {options.map(([v, label]) => (
              <option key={v} value={v}>
                {label}
              </option>
            ))}
          </NativeSelect>
        </Field>
        {!what && (
          <Field label={t("Database name")} error={fields.database_name}>
            <Input value={database} onChange={(e) => setDatabase(e.target.value)} />
          </Field>
        )}
        <Field label={t("Schedule")} help={t("Cron, in UTC (minute hour day month weekday)")} error={fields.schedule}>
          <div className="rdp2-row">
            <NativeSelect value={SCHEDULES.some(([v]) => v === schedule) ? schedule : ""} onChange={(e) => e.target.value && setSchedule(e.target.value)}>
              {SCHEDULES.map(([v, label]) => (
                <option key={v} value={v}>
                  {t(label)}
                </option>
              ))}
              <option value="">{t("Custom")}</option>
            </NativeSelect>
            <Input className="rdp2-mono" value={schedule} onChange={(e) => setSchedule(e.target.value)} />
          </div>
        </Field>
        <Field label={t("Destination path")} help={t("A folder of the provider, one per database")} error={fields.path}>
          <Input value={path} onChange={(e) => setPath(e.target.value)} />
        </Field>
        <Field label={t("Retention")} help={t("Files of this database kept in the path")} error={fields.retention}>
          <Input type="number" min={1} max={1000} value={retention} onChange={(e) => setRetention(e.target.value)} />
        </Field>
        <label className="rdp2-row">
          <Switch checked={enabled} onCheckedChange={setEnabled} /> {t("Run on the schedule")}
        </label>
        <ErrorNote error={error} />
        <div className="rdp2-row rdp2-end">
          <Button type="submit">{c ? t("Save") : t("Create")}</Button>
        </div>
      </form>
    </Dialog>
  );
}

function RunLog(props: Props & { id: number; onClose: () => void }) {
  const t = props.t;
  const run = useAdminQuery<Run>(props, `/apps/backup-runs/${props.id}`, {
    refetchInterval: (q) => (q.state.data && ["queued", "running"].includes(q.state.data.status) ? 1500 : false),
  });
  return (
    <Dialog open onClose={props.onClose} title={`${t("Backup")} #${props.id}`} className="rdp2-wide">
      {run.isLoading && <Loading />}
      {run.data?.error && <Alert variant="destructive">{run.data.error}</Alert>}
      {run.data && <pre className="rdp2-log">{run.data.log || t("Waiting for a worker…")}</pre>}
    </Dialog>
  );
}

function RestoreForm(props: Props & { tab: Tab; onClose: () => void; onDone: () => void }) {
  const ask = useAsk();
  const { t, d } = props;
  const [provider, setProvider] = useState(String(props.tab.providers[0]?.id ?? ""));
  const [folder, setFolder] = useState(props.tab.configs[0]?.path ?? `${d.project.name}/${d.service.name}`);
  const [files, setFiles] = useState<Array<{ file: string; size: number }> | null>(null);
  const [file, setFile] = useState("");
  const [database, setDatabase] = useState(d.database?.database ?? "");
  const options = whatOptions(t, props.tab);
  const [what, setWhat] = useState(options[0]?.[0] ?? "");
  const [fields, setFields] = useState<Record<string, string[]>>({});
  const [error, setError] = useState<unknown>(null);
  // The listing shown is the last asked for, of the provider and folder shown.
  const asked = useRef(0);
  const reset = () => {
    asked.current += 1;
    setFiles(null);
    setFile("");
  };
  const list = async () => {
    setError(null);
    const ticket = ++asked.current;
    // Without a role's restore permission: this service's own backups only.
    if (!props.can("backups.restore")) {
      const own = props.tab.restorable.filter((r) => String(r.storage_provider_id) === provider).map((r) => ({ file: r.file, size: 0 }));
      setFiles(own);
      setFile(own[0]?.file ?? "");
      return;
    }
    try {
      const out = await props.api.get<{ files: Array<{ file: string; size: number }> }>(
        // The query as an option: RustCMS refuses a `?` in the path.
        `/apps/storage-providers/${provider}/files`,
        { query: { path: folder.trim() } },
      );
      if (ticket !== asked.current) return;
      setFiles(out.files);
      setFile(out.files[0]?.file ?? "");
    } catch (err) {
      setError(err);
    }
  };
  return (
    <Dialog open onClose={props.onClose} title={t("Restore")} className="rdp2-wide">
      <form
        className="rdp2-form"
        onSubmit={async (e) => {
          e.preventDefault();
          const ok = await ask({
            title: t("Restore this backup?"),
            body: t("Restore this file into the database? Its current data is replaced."),
            action: t("Restore"),
            destructive: true,
          });
          if (!ok) return;
          setFields({});
          setError(null);
          try {
            await props.api.post(`/apps/services/${d.service.id}/restore`, {
              storage_provider_id: Number(provider),
              file,
              database: what ? volumeLabel(what) : database.trim(),
              volume: what || undefined,
            });
            props.onDone();
          } catch (err) {
            if (err instanceof Error && "fieldErrors" in err) setFields((err as ResourceError).fieldErrors);
            setError(err);
          }
        }}
      >
        <Alert variant="warning">{t("A restore replaces the database's data. Stop the apps writing to it, and take a backup first.")}</Alert>
        <Field label={t("Storage provider")}>
          <NativeSelect
            value={provider}
            onChange={(e) => {
              setProvider(e.target.value);
              reset();
            }}
          >
            {props.tab.providers.map((p) => (
              <option key={p.id} value={p.id}>
                {p.name}
              </option>
            ))}
          </NativeSelect>
        </Field>
        <Field label={t("Folder")}>
          <div className="rdp2-row">
            <Input
              value={folder}
              onChange={(e) => {
                setFolder(e.target.value);
                reset();
              }}
            />
            <Button type="button" variant="outline" onClick={list} disabled={!provider || !folder.trim()}>
              {t("List files")}
            </Button>
          </div>
        </Field>
        {files && (
          <Field label={t("File")} error={fields.file}>
            {files.length ? (
              <NativeSelect value={file} onChange={(e) => setFile(e.target.value)}>
                {files.map((f) => (
                  <option key={f.file} value={f.file}>
                    {f.file.split("/").pop()} · {bytes(f.size)}
                  </option>
                ))}
              </NativeSelect>
            ) : (
              <p className="rdp2-muted">{t("No file in this folder.")}</p>
            )}
          </Field>
        )}
        <Field label={t("Restore")} error={fields.volume}>
          <NativeSelect value={what} onChange={(e) => setWhat(e.target.value)}>
            {options.map(([v, label]) => (
              <option key={v} value={v}>
                {label}
              </option>
            ))}
          </NativeSelect>
        </Field>
        {!what && (
          <Field label={t("Restore into database")} error={fields.database}>
            <Input value={database} onChange={(e) => setDatabase(e.target.value)} />
          </Field>
        )}
        <ErrorNote error={error} />
        <div className="rdp2-row rdp2-end">
          <Button type="submit" variant="destructive" disabled={!file || (!what && !database.trim())}>
            {t("Restore")}
          </Button>
        </div>
      </form>
    </Dialog>
  );
}

export function BackupsTab(props: Props) {
  const { t, d } = props;
  const tab = useAdminQuery<Tab>(props, `/apps/services/${d.service.id}/backups`, { refetchInterval: 3000 });
  const [editing, setEditing] = useState<Config | "new" | null>(null);
  const [log, setLog] = useState<number | null>(null);
  const [restoring, setRestoring] = useState(false);
  const [error, setError] = useState<unknown>(null);
  const ask = useAsk();
  if (!d.permissions.includes("database_backups.view_any")) return <EmptyState title={t("You may not see backups.")} />;
  if (tab.isLoading) return <Loading />;
  if (tab.error || !tab.data) return <ErrorState error={tab.error} />;
  const data = tab.data;
  if (!data.supported && data.volumes.length === 0)
    return <EmptyState title={t("Redis keeps its data in its volume (append-only file): it has no database backups.")} />;
  const providerName = (id: number) => data.providers.find((p) => p.id === id)?.name ?? `#${id}`;
  const runNow = async (c: Config) => {
    setError(null);
    try {
      const run = await props.api.post<Run>(`/apps/backups/${c.id}/run`, {});
      setLog(run.id);
      tab.refetch();
    } catch (err) {
      setError(err);
    }
  };
  return (
    <>
      <Section
        title={t("Backups")}
        description={t("Dumps of the database (pg_dump, mysqldump, mariadb-dump, mongodump) or archives of its volumes, gzipped, written to a storage provider on a schedule.")}
        actions={
          <>
            {d.permissions.includes("backups.restore") && data.providers.length > 0 && (
              <Button variant="outline" onClick={() => setRestoring(true)}>
                {t("Restore")}
              </Button>
            )}
            {d.permissions.includes("database_backups.create") && data.providers.length > 0 && (
              <Button onClick={() => setEditing("new")}>{t("Create backup")}</Button>
            )}
          </>
        }
      >
        {data.providers.length === 0 && (
          <Alert>
            {t("No storage provider yet.")}{" "}
            <NavLink to="/p/settings/backups" navigate={props.navigate}>
              {t("Add one in Settings → Backups")}
            </NavLink>
          </Alert>
        )}
        <ErrorNote error={error} />
        {data.configs.length === 0 && data.providers.length > 0 && (
          <EmptyState title={t("No backup scheduled")} hint={t("Create one to dump the database or archive a volume on a schedule.")} />
        )}
        {data.configs.length > 0 && (
          <TableScroll>
            <table className="rdp2-table">
              <thead>
                <tr>
                  <th>{t("Backs up")}</th>
                  <th>{t("Schedule")}</th>
                  <th className="rdp2-hide-sm">{t("Storage provider")}</th>
                  <th className="rdp2-hide-sm">{t("Path")}</th>
                  <th className="rdp2-right">{t("Retention")}</th>
                  <th className="rdp2-right">
                    <span className="rdp2-sr">{t("Actions")}</span>
                  </th>
                </tr>
              </thead>
              <tbody>
                {data.configs.map((c) => {
                  const menu: MenuItem[] = [];
                  if (d.permissions.includes("database_backups.update")) menu.push({ label: t("Edit"), onSelect: () => setEditing(c) });
                  if (d.permissions.includes("database_backups.delete"))
                    menu.push("separator", {
                      label: t("Delete…"),
                      destructive: true,
                      onSelect: async () => {
                        const ok = await ask({
                          title: t("Delete this backup configuration?"),
                          body: t("Its files stay in the provider."),
                          action: t("Delete backup"),
                          destructive: true,
                        });
                        if (!ok) return;
                        try {
                          await resources.remove("database_backups", c.id);
                          tab.refetch();
                        } catch (err) {
                          setError(err);
                        }
                      },
                    });
                  return (
                    <tr key={c.id}>
                      <td className="rdp2-strong-cell">{c.volume ? `${t("Volume")} ${c.volume}` : c.database_name}</td>
                      <td className="rdp2-mono rdp2-nowrap">
                        {c.schedule}
                        {!c.enabled && <span className="rdp2-muted"> · {t("off")}</span>}
                      </td>
                      <td className="rdp2-hide-sm">{providerName(c.storage_provider_id)}</td>
                      <td className="rdp2-hide-sm rdp2-mono">{c.path}</td>
                      <td className="rdp2-right rdp2-num">{c.retention}</td>
                      <td>
                        <RowActions label={t("Actions for {host}").replace("{host}", c.volume ?? c.database_name ?? "")} menu={menu}>
                          {d.permissions.includes("backups.run") && (
                            <Button size="sm" variant="ghost" onClick={() => runNow(c)}>
                              {t("Run now")}
                            </Button>
                          )}
                        </RowActions>
                      </td>
                    </tr>
                  );
                })}
              </tbody>
            </table>
          </TableScroll>
        )}
      </Section>
      <Section title={t("Backups log")} flush>
        {data.runs.length === 0 ? (
          <EmptyState title={t("No backup yet.")} />
        ) : (
          <TableScroll>
            <table className="rdp2-table">
              <thead>
                <tr>
                  <th>#</th>
                  <th>{t("Action")}</th>
                  <th>{t("State")}</th>
                  <th className="rdp2-hide-sm">{t("File")}</th>
                  <th className="rdp2-right">{t("Size")}</th>
                  <th className="rdp2-hide-sm">{t("Started")}</th>
                  <th className="rdp2-right">
                    <span className="rdp2-sr">{t("Actions")}</span>
                  </th>
                </tr>
              </thead>
              <tbody>
                {data.runs.map((r) => (
                  <tr key={r.id}>
                    <td className="rdp2-num">{r.id}</td>
                    <td className="rdp2-nowrap">
                      {t(r.kind)} · {t(r.trigger)}
                    </td>
                    <td>
                      <Status kind="action" status={r.status} t={t} />
                    </td>
                    <td className="rdp2-hide-sm">
                      <span className="rdp2-mono rdp2-ellipsis rdp2-cell-clip" title={r.file ?? ""}>
                        {r.file?.split("/").pop() ?? ""}
                      </span>
                    </td>
                    <td className="rdp2-right rdp2-num rdp2-nowrap">{r.size != null ? size(r.size, props.locale) : ""}</td>
                    <td className="rdp2-hide-sm rdp2-nowrap" title={dateTime(r.created_at, props.locale)}>
                      {relative(r.created_at, props.locale)}
                    </td>
                    <td>
                      <RowActions label={t("Actions for {host}").replace("{host}", `#${r.id}`)}>
                        <Button size="sm" variant="ghost" onClick={() => setLog(r.id)}>
                          {t("Log")}
                        </Button>
                      </RowActions>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </TableScroll>
        )}
      </Section>
      {editing && (
        <ConfigForm
          {...props}
          tab={data}
          config={editing === "new" ? null : editing}
          onClose={() => setEditing(null)}
          onSaved={() => {
            setEditing(null);
            tab.refetch();
          }}
        />
      )}
      {restoring && (
        <RestoreForm
          {...props}
          tab={data}
          onClose={() => setRestoring(false)}
          onDone={() => {
            setRestoring(false);
            tab.refetch();
          }}
        />
      )}
      {log !== null && <RunLog {...props} id={log} onClose={() => setLog(null)} />}
    </>
  );
}
