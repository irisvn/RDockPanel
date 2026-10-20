// EasyPanel's templates in a project: the catalog (searchable), a
// template's form (its JSON Schema), and "Create from schema" (a generator's
// `{services}` output, pasted). Creating can deploy at once.
import { useMemo, useState } from "react";
import { type PageProps, useAdminQuery } from "@rustcms/admin-sdk";
import { Alert, Button, Dialog, ErrorState, Input, Loading, NativeSelect, Switch, Textarea } from "@rustcms/admin-sdk/ui";
import { ErrorNote, Field } from "../lib/ui";
import type { Project } from "./types";

type Item = { slug: string; name: string; description: string | null; tags: string[]; logo_file: string | null };
type Property = {
  type?: string;
  title?: string;
  description?: string;
  default?: unknown;
  enum?: unknown[];
  oneOf?: Array<{ enum?: unknown[]; const?: unknown; title?: string }>;
};
type Page = {
  slug: string;
  name: string;
  description: string | null;
  instructions: string | null;
  schema: { required?: string[]; properties?: Record<string, Property> } | null;
  links: Array<{ label: string; url: string }> | null;
  logo: boolean;
};
type Made = { project: string; services: Array<{ id: number; name: string; kind: string }>; warnings: string[] };

const logoUrl = (slug: string) => `${document.baseURI.replace(/\/$/, "")}/api/x/apps/templates/${slug}/logo`;

function TemplateForm(props: PageProps<unknown> & { slug: string; project: Project; onBack: () => void; onDone: (m: Made) => void }) {
  const t = props.t;
  const page = useAdminQuery<Page>(props, `/apps/templates/${props.slug}`);
  const [values, setValues] = useState<Record<string, unknown>>({});
  const [deploy, setDeploy] = useState(props.project.can.deploy);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<unknown>(null);
  if (page.isLoading) return <Loading />;
  if (page.error || !page.data) return <ErrorState error={page.error} />;
  const p = page.data;
  const properties = Object.entries(p.schema?.properties ?? {});
  const required = new Set(p.schema?.required ?? []);
  const optionsOf = (prop: Property) =>
    prop.oneOf?.map((o) => ({ value: o.const ?? o.enum?.[0], title: o.title ?? String(o.const ?? o.enum?.[0]) })) ??
    prop.enum?.map((v) => ({ value: v, title: String(v) }));
  // A select without a default starts on its first option (what it shows).
  const value = (name: string, prop: Property) =>
    name in values ? values[name] : prop.default ?? optionsOf(prop)?.[0]?.value ?? (prop.type === "boolean" ? false : "");
  const set = (name: string, v: unknown) => setValues((old) => ({ ...old, [name]: v }));
  return (
    <form
      className="rdp2-form"
      onSubmit={async (e) => {
        e.preventDefault();
        setBusy(true);
        setError(null);
        const input: Record<string, unknown> = {};
        for (const [name, prop] of properties) {
          const v = value(name, prop);
          input[name] = prop.type === "number" || prop.type === "integer" ? Number(v) : v;
        }
        try {
          const made = await props.api.post<Made>(`/apps/projects/${props.project.id}/templates/${props.slug}`, { input, deploy });
          props.onDone(made);
        } catch (err) {
          setError(err);
        } finally {
          setBusy(false);
        }
      }}
    >
      <div className="rdp2-row">
        <Button type="button" variant="ghost" size="sm" onClick={props.onBack}>
          ← {t("Templates")}
        </Button>
      </div>
      <div className="rdp2-row">
        {p.logo && <img className="rdp2-logo" src={logoUrl(p.slug)} alt="" />}
        <h3 className="rdp2-section-title">{p.name}</h3>
      </div>
      {p.description && <p className="rdp2-muted rdp2-small rdp2-clamp">{p.description}</p>}
      {p.instructions && <Alert>{p.instructions}</Alert>}
      {properties.map(([name, prop]) => {
        const options = optionsOf(prop);
        const label = `${prop.title ?? name}${required.has(name) ? " *" : ""}`;
        if (prop.type === "boolean")
          return (
            <label key={name} className="rdp2-row">
              <Switch checked={Boolean(value(name, prop))} onCheckedChange={(v) => set(name, v)} /> {label}
            </label>
          );
        return (
          <Field key={name} label={label} help={prop.description}>
            {options ? (
              <NativeSelect required={required.has(name)} value={String(value(name, prop))} onChange={(e) => set(name, options.find((o) => String(o.value) === e.target.value)?.value)}>
                {options.map((o) => (
                  <option key={String(o.value)} value={String(o.value)}>
                    {o.title}
                  </option>
                ))}
              </NativeSelect>
            ) : (
              <Input
                type={prop.type === "number" || prop.type === "integer" ? "number" : "text"}
                required={required.has(name)}
                value={String(value(name, prop) ?? "")}
                onChange={(e) => set(name, e.target.value)}
              />
            )}
          </Field>
        );
      })}
      {props.project.can.deploy && (
        <label className="rdp2-row">
          <Switch checked={deploy} onCheckedChange={setDeploy} /> {t("Deploy now")}
        </label>
      )}
      <ErrorNote error={error} />
      <div className="rdp2-row rdp2-end">
        <Button type="submit" disabled={busy}>
          {t("Create")}
        </Button>
      </div>
    </form>
  );
}

function FromSchema(props: PageProps<unknown> & { project: Project; onBack: () => void; onDone: (m: Made) => void }) {
  const t = props.t;
  const [text, setText] = useState("");
  const [deploy, setDeploy] = useState(false);
  const [error, setError] = useState<unknown>(null);
  return (
    <form
      className="rdp2-form"
      onSubmit={async (e) => {
        e.preventDefault();
        setError(null);
        try {
          const schema = JSON.parse(text);
          props.onDone(await props.api.post<Made>(`/apps/projects/${props.project.id}/schema`, { schema, deploy }));
        } catch (err) {
          setError(err);
        }
      }}
    >
      <div className="rdp2-row">
        <Button type="button" variant="ghost" size="sm" onClick={props.onBack}>
          ← {t("Templates")}
        </Button>
      </div>
      <Field label={t("Schema")} help={t("A template's output: {\"services\": [{\"type\": \"app\", \"data\": {…}}]}")}>
        <Textarea className="rdp2-env" rows={14} value={text} onChange={(e) => setText(e.target.value)} />
      </Field>
      {props.project.can.deploy && (
        <label className="rdp2-row">
          <Switch checked={deploy} onCheckedChange={setDeploy} /> {t("Deploy now")}
        </label>
      )}
      <ErrorNote error={error} />
      <div className="rdp2-row rdp2-end">
        <Button type="submit" disabled={!text.trim()}>
          {t("Create")}
        </Button>
      </div>
    </form>
  );
}

/** The templates of a project's "Templates" button. */
export function Templates(props: PageProps<unknown> & { project: Project; onClose: () => void; onDone: (m: Made) => void }) {
  const t = props.t;
  const list = useAdminQuery<{ templates: Item[] }>(props, "/apps/templates", { staleTime: 10 * 60 * 1000 });
  const [search, setSearch] = useState("");
  const [open, setOpen] = useState<string | null>(null);
  const [schema, setSchema] = useState(false);
  const shown = useMemo(() => {
    const q = search.trim().toLowerCase();
    const all = list.data?.templates ?? [];
    return (q ? all.filter((i) => `${i.name} ${i.description ?? ""} ${i.tags.join(" ")}`.toLowerCase().includes(q)) : all).slice(0, 120);
  }, [list.data, search]);
  return (
    <Dialog open onClose={props.onClose} title={`${t("Templates")} · ${props.project.name}`} className="rdp2-wide">
      {open ? (
        <TemplateForm {...props} slug={open} onBack={() => setOpen(null)} />
      ) : schema ? (
        <FromSchema {...props} onBack={() => setSchema(false)} />
      ) : (
        <div className="rdp2-form">
          <div className="rdp2-row">
            <Input autoFocus placeholder={t("Search templates")} value={search} onChange={(e) => setSearch(e.target.value)} />
            <Button type="button" variant="outline" onClick={() => setSchema(true)}>
              {t("Create from schema")}
            </Button>
          </div>
          {list.isLoading && <Loading />}
          {list.error && <ErrorState error={list.error} />}
          {list.data && (
            <p className="rdp2-muted rdp2-small">
              {list.data.templates.length} {t("templates from easypanel.io, fetched when opened.")}
            </p>
          )}
          <div className="rdp2-templates">
            {shown.map((i) => (
              <button key={i.slug} type="button" className="rdp2-template" onClick={() => setOpen(i.slug)}>
                {i.logo_file ? <img className="rdp2-logo" src={logoUrl(i.slug)} alt="" loading="lazy" /> : <span className="rdp2-logo" />}
                <span className="rdp2-template-text">
                  <strong>{i.name}</strong>
                  <span className="rdp2-muted rdp2-small rdp2-clamp">{i.description}</span>
                </span>
              </button>
            ))}
          </div>
        </div>
      )}
    </Dialog>
  );
}
