// ⌘K / Ctrl+K: a quick search over projects, services and the panel's
// pages (EasyPanel's command palette). Arrows move, Enter opens.
import { useEffect, useMemo, useState } from "react";
import { type PageProps, useAdminQuery } from "@rustcms/admin-sdk";
import { Dialog, Input } from "@rustcms/admin-sdk/ui";
import type { Overview } from "../apps/types";

type Item = { label: string; hint: string; path: string };

function Search(props: PageProps<unknown> & { onClose: () => void }) {
  const t = props.t;
  const overview = useAdminQuery<Overview>(props, "/apps/overview");
  const [query, setQuery] = useState("");
  const [at, setAt] = useState(0);
  const items = useMemo(() => {
    const all: Item[] = [
      { label: t("Dashboard"), hint: t("Page"), path: "/p/dashboard" },
      { label: t("Actions"), hint: t("Page"), path: "/p/actions" },
      { label: t("Settings"), hint: t("Page"), path: "/p/settings" },
    ];
    for (const p of overview.data?.projects ?? []) {
      all.push({ label: p.name, hint: t("Project"), path: `/p/dashboard/${p.name}` });
      for (const s of p.services)
        all.push({ label: `${p.name} / ${s.name}`, hint: t(s.kind), path: `/p/dashboard/${p.name}/${s.name}` });
    }
    const q = query.trim().toLowerCase();
    return (q ? all.filter((i) => i.label.toLowerCase().includes(q)) : all).slice(0, 30);
  }, [overview.data, query, t]);
  const open = (item: Item | undefined) => {
    if (!item) return;
    props.onClose();
    props.navigate(item.path);
  };
  return (
    <Dialog open onClose={props.onClose} title={t("Go to…")}>
      <Input
        autoFocus
        placeholder={t("Search projects, services, pages")}
        value={query}
        onChange={(e) => {
          setQuery(e.target.value);
          setAt(0);
        }}
        onKeyDown={(e) => {
          if (e.key === "ArrowDown") {
            e.preventDefault();
            setAt((i) => Math.min(i + 1, items.length - 1));
          } else if (e.key === "ArrowUp") {
            e.preventDefault();
            setAt((i) => Math.max(i - 1, 0));
          } else if (e.key === "Enter") {
            e.preventDefault();
            open(items[at]);
          }
        }}
      />
      <ul className="rdp2-palette" role="listbox">
        {items.map((item, i) => (
          <li key={item.path} role="option" aria-selected={i === at}>
            <button type="button" className={i === at ? "rdp2-palette-item rdp2-palette-active" : "rdp2-palette-item"} onClick={() => open(item)} onMouseEnter={() => setAt(i)}>
              <span>{item.label}</span>
              <span className="rdp2-muted rdp2-small">{item.hint}</span>
            </button>
          </li>
        ))}
      </ul>
    </Dialog>
  );
}

/** Mounted once per page: opens the search on ⌘K / Ctrl+K. */
export function Palette(props: PageProps<unknown>) {
  const [open, setOpen] = useState(false);
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "k") {
        e.preventDefault();
        setOpen((o) => !o);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);
  return open && props.can("projects.view_any") ? <Search {...props} onClose={() => setOpen(false)} /> : null;
}
