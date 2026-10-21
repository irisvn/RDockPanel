// Settings → Branding: the panel's name, primary color, logos and favicon,
// changed while it runs (the sign-in page too). Images are PNG, WebP or ICO
// up to 512 KiB (no SVG). Left empty: RDockPanel's own. A reload shows the
// change here; other open pages and instances within about 30 seconds.
import { useState } from "react";
import { fieldErrors, type PageProps } from "@rustcms/admin-sdk";
import { useAdminQuery } from "../lib/query";
import { Button, ErrorState, Input, Loading } from "@rustcms/admin-sdk/ui";
import { ColorField, ErrorNote, Field, FilePicker, RowActions, Section, TableScroll } from "../lib/ui";
import { size } from "../lib/format";

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
    <>
      <Section
        title={t("Name and color")}
        description={t("The panel's name and primary color, on every page and the sign-in page. Left empty: RDockPanel's own.")}
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
          <div className="rdp2-grid2">
            <Field label={t("Name")} error={fields.name}>
              <Input value={nameValue} maxLength={64} placeholder="RDockPanel" onChange={(e) => setName(e.target.value)} />
            </Field>
            <Field label={t("Primary color")} error={fields.primary}>
              <ColorField label={t("Primary color")} value={primaryValue} placeholder="#2563eb" onChange={setPrimary} />
            </Field>
          </div>
          {!fields.data && Object.keys(fields).length === 0 && <ErrorNote error={error} />}
          <div className="rdp2-dialog-foot">
            <Button type="submit" disabled={busy}>
              {t("Save")}
            </Button>
          </div>
        </form>
      </Section>
      <Section title={t("Logos and favicon")} description={t("PNG, WebP or ICO, at most 512 KB each (no SVG).")} flush>
        {fields.data && (
          <div className="rdp2-flush-note">
            <ErrorNote error={new Error(fields.data.join(" "))} />
          </div>
        )}
        <TableScroll>
          <table className="rdp2-table">
            <thead>
              <tr>
                <th>{t("Image")}</th>
                <th>{t("Now")}</th>
                <th className="rdp2-right">
                  <span className="rdp2-sr">{t("Actions")}</span>
                </th>
              </tr>
            </thead>
            <tbody>
              {c.kinds.map((kind) => {
                const image = c.images[kind];
                const label = t(KIND_LABELS[kind] ?? kind);
                return (
                  <tr key={kind}>
                    <td className="rdp2-strong-cell">{label}</td>
                    <td>
                      {image ? (
                        <div className="rdp2-row">
                          <img src={image.url} alt="" className="rdp2-brand-preview" />
                          <span className="rdp2-small rdp2-muted">{size(image.size, props.locale)}</span>
                        </div>
                      ) : (
                        <span className="rdp2-muted">{t("RDockPanel's own")}</span>
                      )}
                    </td>
                    <td>
                      <RowActions
                        label={t("Actions for {host}").replace("{host}", label)}
                        menu={
                          image
                            ? [{ label: t("Remove…"), destructive: true, disabled: busy, onSelect: () => void run(() => props.api.delete(`/apps/branding/images/${kind}`)) }]
                            : []
                        }
                      >
                        <FilePicker
                          label={image ? t("Replace") : t("Upload")}
                          ariaLabel={`${image ? t("Replace") : t("Upload")}: ${label}`}
                          accept={ACCEPT}
                          disabled={busy}
                          onFile={(file) => void run(async () => props.api.put(`/apps/branding/images/${kind}`, { mime: mimeOf(file), data: await base64(file) }))}
                        />
                      </RowActions>
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        </TableScroll>
      </Section>
      {saved && <p className="rdp2-versions">{t("Saved: reload the page to see it.")}</p>}
    </>
  );
}
