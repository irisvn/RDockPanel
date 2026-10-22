// Settings → Maintenance page: the page every service in maintenance shows
// ("Default"), unless it has its own HTML, and the contact it names. Empty:
// the built-in page.
import { useState } from "react";
import { fieldErrors, type PageProps } from "@rustcms/admin-sdk";
import { Button, ErrorState, Input, Loading } from "@rustcms/admin-sdk/ui";
import { useAdminQuery } from "../lib/query";
import { ErrorNote, Field, Section } from "../lib/ui";
import { HtmlEditor, Preview } from "../apps/MaintenancePage";

type Current = { template: string; contact: string; starter: string; placeholders: string[] };

export function MaintenanceSettings(props: PageProps<unknown>) {
  const t = props.t;
  const current = useAdminQuery<Current>(props, "/apps/settings/maintenance");
  const [template, setTemplate] = useState<string | null>(null);
  const [contact, setContact] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<unknown>(null);
  if (current.isLoading) return <Loading />;
  if (current.error || !current.data) return <ErrorState error={current.error} />;
  const c = current.data;
  const fields = error ? fieldErrors(error) : {};
  const html = template ?? c.template;
  const contactValue = contact ?? c.contact;
  return (
    <Section
      title={t("Maintenance page")}
      description={t("The page of every service in maintenance with the default page. Empty: the built-in page. A service may have its own HTML instead.")}
    >
      <form
        className="rdp2-form"
        onSubmit={async (e) => {
          e.preventDefault();
          setBusy(true);
          setError(null);
          try {
            await props.api.put("/apps/settings/maintenance", { template: html, contact: contactValue });
            await current.refetch();
            setTemplate(null);
            setContact(null);
            props.toast.success(t("Saved and applied."));
          } catch (err) {
            setError(err);
          } finally {
            setBusy(false);
          }
        }}
      >
        <div className="rdp2-maint">
          <div className="rdp2-form">
            <Field label={t("Default contact")} help={t("An email or an https link, for services that name none")} error={fields.contact}>
              <Input value={contactValue} onChange={(e) => setContact(e.target.value)} placeholder="support@example.com" />
            </Field>
            <HtmlEditor t={t} value={html} onChange={setTemplate} starter={c.starter} placeholders={c.placeholders} error={fields.template} />
            {Object.keys(fields).length === 0 && <ErrorNote error={error} />}
            <div className="rdp2-row rdp2-end">
              <Button type="submit" disabled={busy}>{t("Save")}</Button>
            </div>
          </div>
          <Preview {...props} label={t("Preview")} body={{ title: "", subtitle: "", contact: contactValue, html }} />
        </div>
      </form>
    </Section>
  );
}
