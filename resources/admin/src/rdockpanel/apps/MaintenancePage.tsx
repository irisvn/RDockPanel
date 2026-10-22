// The maintenance page being written: its preview (rendered by the panel as
// visitors would get it, shown in a sandboxed frame) and the HTML editor
// with its placeholders. Used by a service's Maintenance tab and Settings.
import { useEffect, useRef, useState } from "react";
import { type PageProps } from "@rustcms/admin-sdk";
import { Button, Skeleton, Textarea } from "@rustcms/admin-sdk/ui";
import { adminHref, useAsk } from "../lib/ui";

/** What a preview renders (nothing saved). */
export type PreviewBody = {
  service_id?: number;
  title?: string;
  subtitle?: string;
  eta?: string | null;
  eta_offset?: number;
  contact?: string;
  page?: "default" | "custom";
  html?: string;
};

/** The page as `body` renders it, shown again a moment after it changes. */
export function Preview(props: PageProps<unknown> & { body: PreviewBody; label: string; revision?: string | null }) {
  const [src, setSrc] = useState<string | null>(null);
  const [failed, setFailed] = useState(false);
  // Rendered again when the body or what it does not carry (the logo) changes.
  const key = JSON.stringify(props.body) + String(props.revision ?? "");
  useEffect(() => {
    let live = true;
    const timer = window.setTimeout(async () => {
      try {
        const made = await props.api.post<{ id: string }>("/apps/maintenance/preview", props.body);
        if (live) {
          setSrc(adminHref(`/api/x/apps/maintenance/preview/${made.id}`));
          setFailed(false);
        }
      } catch {
        if (live) setFailed(true);
      }
    }, 500);
    return () => {
      live = false;
      window.clearTimeout(timer);
    };
    // The body's content, not its identity.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [key]);
  return (
    <div className="rdp2-preview">
      <div className="rdp2-preview-bar">
        <span>{props.label}</span>
        {failed && <span className="rdp2-error">{props.t("The preview could not be made.")}</span>}
      </div>
      {src ? <iframe sandbox="" src={src} title={props.label} /> : <Skeleton className="rdp2-preview-empty" />}
    </div>
  );
}

/** A page's own HTML: the editor, the placeholders to insert, and the
 * built-in page to start from. */
export function HtmlEditor(props: {
  t: (s: string) => string;
  value: string;
  onChange: (value: string) => void;
  starter: string;
  placeholders: string[];
  disabled?: boolean;
  error?: string[];
}) {
  const { t } = props;
  const ask = useAsk();
  const area = useRef<HTMLTextAreaElement>(null);
  // What was written is replaced: asked first.
  const start = async () => {
    if (props.value.trim() && props.value !== props.starter) {
      const ok = await ask({ title: t("Start from the default page?"), body: t("The HTML written here is replaced."), action: t("Replace it"), destructive: true });
      if (!ok) return;
    }
    props.onChange(props.starter);
  };
  const insert = (name: string) => {
    const text = `{{${name}}}`;
    const el = area.current;
    if (!el) return props.onChange(props.value + text);
    const [from, to] = [el.selectionStart, el.selectionEnd];
    props.onChange(props.value.slice(0, from) + text + props.value.slice(to));
    window.requestAnimationFrame(() => {
      el.focus();
      el.setSelectionRange(from + text.length, from + text.length);
    });
  };
  return (
    <div className="rdp2-field">
      <div className="rdp2-row rdp2-between">
        <label className="rdp2-field-label" htmlFor="rdp2-maintenance-html">
          {t("HTML")}
        </label>
        {!props.disabled && (
          <Button type="button" variant="outline" size="sm" onClick={() => void start()}>
            {t("Start from the default page")}
          </Button>
        )}
      </div>
      <Textarea
        ref={area}
        id="rdp2-maintenance-html"
        className="rdp2-mono"
        rows={16}
        spellCheck={false}
        value={props.value}
        onChange={(e) => props.onChange(e.target.value)}
        disabled={props.disabled}
      />
      {props.error && <p className="rdp2-field-error">{props.error.join(", ")}</p>}
      <p className="rdp2-field-help">
        {t("HTML and CSS only: scripts are blocked. Images and fonts: data URIs or https. Up to 256 KB. Insert a value:")}
      </p>
      <div className="rdp2-chips">
        {props.placeholders.map((name) => (
          <Button key={name} type="button" size="sm" variant="outline" className="rdp2-mono" disabled={props.disabled} onClick={() => insert(name)}>
            {`{{${name}}}`}
          </Button>
        ))}
      </div>
    </div>
  );
}

/** A `datetime-local` value (the browser's time) of an instant, and back. */
export function localInput(iso: string | null): string {
  if (!iso) return "";
  const at = new Date(iso);
  const local = new Date(at.getTime() - at.getTimezoneOffset() * 60_000);
  return local.toISOString().slice(0, 16);
}

export function fromLocalInput(value: string): { eta: string | null; offset: number } {
  const offset = -new Date().getTimezoneOffset();
  if (!value) return { eta: null, offset };
  const at = new Date(value);
  return Number.isNaN(at.getTime()) ? { eta: null, offset } : { eta: at.toISOString(), offset: -at.getTimezoneOffset() };
}
