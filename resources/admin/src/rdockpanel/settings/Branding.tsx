// Settings → Branding: the panel's name, primary color, logos and favicon,
// changed while it runs (the sign-in page too). Images are PNG, WebP or ICO
// up to 512 KiB (no SVG). Left empty: RDockPanel's own. A reload shows the
// change here; other open pages and instances within about 30 seconds.
import { useState } from "react";
import { fieldErrors, type PageProps, useAdminQuery } from "@rustcms/admin-sdk";
import { Button, ErrorState, Input, Loading } from "@rustcms/admin-sdk/ui";
import { ErrorNote, Field, Section, bytes } from "../lib/ui";

type Image = { mime: string; size: number; url: string };
type Current = { name: string | null; primary: string | null; images: Record<string, Image>; kinds: string[] };

const KIND_LABELS: Record<string, string> = { logo: "Logo", "logo-dark": "Logo (dark mode)", favicon: "Favicon" };
const ACCEPT = "image/png,image/webp,image/x-icon,image/vnd.microsoft.icon,.ico";

/** A file as base64 (without its data URL prefix). */
function base64(file: File): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => resolve(String(reader.result).split(",", 2)[1] ?? "");
    reader.onerror = () => reject(reader.error);
    reader.readAsDataURL(file);
  });
}

/** The type the panel checks: a browser may call an icon `image/x-icon` or nothing. */
function mimeOf(file: File): string {
  if (file.type) return file.type;
  return file.name.toLowerCase().endsWith(".ico") ? "image/x-icon" : "";
}

export function BrandingSettings(props: PageProps<unknown>) {
  const t = props.t;
  const current = useAdminQuery<Current>(props, "/apps/branding");
  const [name, setName] = useState<string | null>(null);
  const [primary, setPrimary] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<unknown>(null);
  const [saved, setSaved] = useState(false);
  if (current.isLoading) return <Loading />;
  if (current.error || !current.data) return <ErrorState error={current.error} />;
  const c = current.data;
  const fields = error ? fieldErrors(error) : {};
  const nameValue = name ?? c.name ?? "";
  const primaryValue = primary ?? c.primary ?? "";
  const run = async (action: () => Promise<unknown>) => {
    setBusy(true);
    setError(null);
    setSaved(false);
    try {
      await action();
      await current.refetch();
      setSaved(true);
    } catch (err) {
      setError(err);
    } finally {
      setBusy(false);
    }
  };
  return (
    <Section
      level={2}
      title={t("Branding")}
      description={t("The panel's name, color, logos and favicon, on every page and the sign-in page. Left empty: RDockPanel's own.")}
    >
      <form
        className="rdp2-form"
        onSubmit={(e) => {
          e.preventDefault();
          void run(async () => {
            await props.api.put("/apps/branding", { name: nameValue, primary: primaryValue });
            setName(null);
            setPrimary(null);
          });
        }}
      >
        <Field label={t("Name")} error={fields.name}>
          <Input value={nameValue} maxLength={64} placeholder="RDockPanel" onChange={(e) => setName(e.target.value)} />
        </Field>
        <Field label={t("Primary color")} error={fields.primary}>
          <div className="rdp2-row">
            <input
              type="color"
              aria-label={t("Primary color")}
              value={/^#[0-9a-f]{6}$/i.test(primaryValue) ? primaryValue : "#2563eb"}
              onChange={(e) => setPrimary(e.target.value)}
            />
            <Input className="rdp2-mono" value={primaryValue} placeholder="#2563eb" onChange={(e) => setPrimary(e.target.value)} />
          </div>
        </Field>
        <div className="rdp2-row rdp2-end">
          <Button type="submit" disabled={busy}>
            {t("Save")}
          </Button>
        </div>
      </form>
      {c.kinds.map((kind) => {
        const image = c.images[kind];
        return (
          <Field key={kind} label={t(KIND_LABELS[kind] ?? kind)} error={undefined}>
            <div className="rdp2-row">
              {image ? (
                <img src={image.url} alt="" className="rdp2-brand-preview" />
              ) : (
                <span className="rdp2-small rdp2-muted">{t("RDockPanel's own")}</span>
              )}
              {image && <span className="rdp2-small rdp2-muted">{bytes(image.size)}</span>}
              <input
                type="file"
                accept={ACCEPT}
                aria-label={t(KIND_LABELS[kind] ?? kind)}
                disabled={busy}
                onChange={(e) => {
                  const file = e.target.files?.[0];
                  e.target.value = "";
                  if (!file) return;
                  void run(async () => props.api.put(`/apps/branding/images/${kind}`, { mime: mimeOf(file), data: await base64(file) }));
                }}
              />
              {image && (
                <Button size="sm" variant="ghost" disabled={busy} onClick={() => void run(() => props.api.delete(`/apps/branding/images/${kind}`))}>
                  {t("Remove")}
                </Button>
              )}
            </div>
          </Field>
        );
      })}
      <p className="rdp2-small rdp2-muted">{t("PNG, WebP or ICO, at most 512 KB each (no SVG).")}</p>
      {saved && <p className="rdp2-small">{t("Saved: reload the page to see it.")}</p>}
      <ErrorNote error={fields.data ? new Error(fields.data.join(" ")) : Object.keys(fields).length ? null : error} />
    </Section>
  );
}
