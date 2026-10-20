# UI guidelines

The rules every RDockPanel page follows. They match the RustCMS admin shell the panel runs in
(its Users, Roles and Audit pages are the reference), so the panel reads as one product. A page
that needs something these rules do not cover adds the rule here first, in the same change.

Status: **approved 2026-10-20**; [ui-redesign.md](ui-redesign.md) is the plan that applies them.

## 1. Foundations

### Building blocks
- Pages are React, built with the kit `@rustcms/admin-sdk/ui` and the panel's own components in
  `resources/admin/src/rdockpanel/lib/` (section 7). No page is plain JavaScript any more.
- No new CSS class for something the kit or `lib/` already draws. A new shared component goes in
  `lib/` with a line in section 7.

### Color
- Only theme tokens: `--background`, `--foreground`, `--card`, `--muted`, `--muted-foreground`,
  `--border`, `--ring`, `--primary`, `--primary-foreground`, `--destructive`. Never a literal
  color in a component or in CSS.
- The panel adds **status tokens**, defined for light and dark in `dashboard.css`:
  `--rdp-ok` (green), `--rdp-busy` (amber), `--rdp-bad` (red), `--rdp-off` (= `--muted-foreground`,
  so a stopped dot keeps 4.8:1 contrast on a card), and `--rdp-terminal-bg` / `--rdp-terminal-fg`
  for logs and the terminal. Each tone keeps at least 3:1 against `--card` in both themes.
- Status colors only fill a dot. The label beside it stays in the normal text color (green or
  amber text would fall under 4.5:1), and the state is always written, never shown by color alone.
- Primary (the brand color, Settings → Branding) is for the one main action of a view and the
  selected state. Nothing else is colored.

### Type
The shell's font (`--font-sans`, the system UI face), five sizes, three weights:

| Token | Size / line | Use |
|---|---|---|
| `xs` | 12px / 16px | table headers, badges, captions, help text under a field, small buttons |
| `sm` | 14px / 20px | **the default UI text**: table cells, form labels and inputs, descriptions, buttons |
| `base` | 16px / 24px | section titles (600), long reading text (rare) |
| `lg` | 20px / 28px | metric values (tiles, 600, tabular numbers) |
| `2xl` | 24px / 32px | the page title only (shell `h1`, 600) |

- Weights: 400 text, 500 labels and table cells that name a row, 600 titles and values.
- One monospace face, `ui-monospace, SFMono-Regular, Menlo, monospace`, at 13px (class
  `rdp2-mono`), for images, hashes, hosts, ports, paths, environment and logs. Never `em`-relative.
- Sentence case everywhere ("Create from schema", not "Create from Schema"). Uppercase only for
  the page eyebrow (12px, 500, letter-spacing 0.05em).
- Numbers that line up use `tabular-nums`.

### Space and shape
- A 4px grid: gaps of 4, 8, 12, 16, 24, 32px only.
- Page: the shell's padding. Between sections: 24px. Inside a section: 16px between blocks,
  padding 20px (24px from 640px up), as the shell's form sections.
- Radius: sections and dialogs `calc(var(--radius) + 4px)`; buttons, inputs, badges, tiles
  `calc(var(--radius) - 2px)`. Nothing else.
- Borders 1px `--border`. Shadow only the shell's `shadow-xs` on sections. No nested cards: a
  block inside a section is separated by space or a divider, not another border.

## 2. Page layout

Every page has the same frame, top to bottom:

1. **Page header** (`PageHeader`): eyebrow, title, description, actions on the right.
   - Eyebrow: where the page lives (`Docker`, `Settings`). On a project or a service it is the
     path back, as links (`Dashboard / demo`), because the shell's breadcrumb shows only the
     page's own label today (a structured breadcrumb is a RustCMS request, section 11).
   - Title: the thing itself (`Containers`, `web`). A service's status badge sits after its title.
   - Description: one sentence, 14px muted, only where it helps.
2. **Tabs**, when the page has several views: kit `Tabs` inside `ScrollTabs`, which keeps the
   tab list on one row and lets it scroll sideways on a narrow screen (the kit's list wraps).
3. **Content**: a vertical stack of sections, 24px apart (`Stack`).

Nothing sits between the header and the tabs, or between the tabs and the first section.

### Sections
- A section is a `Section`: a card with a header (title 16px/600, optional description, actions
  on the right) and a body.
- A section holds one topic. A long form is several sections, not one.
- A page with a single table may use the table's card as its only section.

## 3. Buttons

### Sizes
| Where | Size |
|---|---|
| Page header actions, form submit, dialog footer | `default` (36px) |
| Section header actions, toolbars, filters | `sm` (32px) |
| Table rows and lists | `sm`, or an `IconButton` (32px square) with a tooltip and an `aria-label` |

The kit's `size="icon"` is 36px; `IconButton` is that button narrowed to 32px (`size-8`) so it
lines up with `sm` buttons. Its `aria-label` names the target ("Logs of demo_web.1").

`lg` is not used. A button's size depends only on where it is, never on how important it feels.

### Variants
| Variant | For |
|---|---|
| `default` (primary) | the one main action of a page header, a form or a dialog. At most one per region. |
| `outline` | the other actions beside it |
| `ghost` | row actions, Cancel, icon buttons, menu triggers inside tables |
| `destructive` | the confirm button of a destructive dialog, and the Danger zone's button. Never in a list or table row. |
| `link` | an action inside a sentence |

### Labels
- Verb first, sentence case, no `+` or other glyphs: `New project`, `Add domain`, `Deploy`.
- `New …` creates something on its own (project, service, token, role); `Add …` attaches
  something to its parent (domain, mount, port, member).
- An icon is optional, 16px, before the label, from `lib/icons` (section 7). No emoji, no text
  arrows (`▾`, `↗`, `✎`). A menu trigger gets the chevron icon, an outside link the
  external-link icon.

### Order and placement
- Right-aligned groups; the primary button is the last (rightmost), after any `⋯` menu:
  `[Open] [Restart] [⋯] [Deploy]`. Dialog footers: `[Cancel (ghost)] [Primary]`.
- On a phone the header keeps the primary and `⋯`; the actions it hides move into `⋯`, with the
  same permission and busy-state rules (disabled with a reason, never removed silently).
- An action that applies to the current state is shown, the others are not (`Start` when
  stopped, `Stop` when running).
- A row shows at most two actions; anything more goes into a `⋯` menu (`RowActions`).
  Destructive items go last in that menu, after a separator, marked `destructive`.
- Links that look like buttons use `buttonVariants()`; never a hand-made class.

## 4. Data: tables, lists, metrics, status

- **A collection with two or more attributes is a table**: kit `Table` in a `Section` (or
  `TableCard`), header 12px muted on `--muted`, cells 14px, 12px padding.
  - The name column is 500 weight; secondary text under it is 12px muted.
  - Numbers and sizes are right-aligned, `tabular-nums`.
  - The actions column is last, right-aligned, with a fixed width. Layout goes in a `div` inside
    the cell, never on the `td` itself.
  - Long values (images, hashes, paths) are truncated with an ellipsis and a tooltip with the
    whole value; copyable ones get a copy icon button.
  - On a narrow screen the table scrolls sideways inside its card. Columns hide only by a class
    the table chooses (`hide-sm`), never by position.
  - Long collections are paginated (25 rows) or filtered, like the shell's tables.
- **Pagination**: 25 rows a page, a stable order (newest first unless the column says
  otherwise), a footer `1–25 of 132` with Previous/Next, filters kept in the page's sub-path so a
  link reopens the same view, back to page 1 when a filter changes. "No results" (a filter
  matched nothing, with Clear filters) differs from "empty" (nothing exists yet). A history the
  server keeps only partly says so (`Latest 100 actions`) until the server paginates.
- **Row details**: the name cell is a link to the item's detail (inspect, logs) when it has one.
  A row shows one labeled frequent action plus `⋯`; icon-only buttons only for actions every row
  has (logs, terminal), each with a tooltip.
- **Hidden rows** are never hidden silently: a filter that hides some (old tasks of past
  deployments) is a visible toggle with the count (`Show 8 old tasks`).
- **A list** (`List`) is for a collection with one attribute and an action, such as tokens or
  members. If it grows a second column, it becomes a table.
- **Service cards** on the Dashboard are the one place for cards in a section: each card is a
  link to the service, shows its name, kind, status (dot and word) and domain.
- **Facts** (label above value) show the properties of one object: labels 12px muted, values
  14px; monospace only for values that are code (`<Facts mono>` per item).
- **Metric tiles** (`Metric`): label 12px muted, value 20px/600, optional sub-line 12px muted and
  a sparkline. The same component on the Dashboard and on a service. Tiles are not cards inside
  cards: on a service they are a row inside the Monitor section, divided by space.
- **Status** (`StatusBadge`): a dot and a word in the kit `Badge` (neutral, `color="gray"`).
  Four **tones** are shared; the **word** belongs to the kind of thing, so a finished deployment
  never reads "Running":

| Kind | ok | busy | bad | off |
|---|---|---|---|---|
| Service | Running | Deploying, Starting | Failed | Stopped |
| Deployment | Succeeded | Queued, Running | Failed, Interrupted | Rolled back |
| Action (pull, backup, prune) | Done | Queued, Running | Failed | — |
| Container | Running | Restarting | Dead | Exited, Paused, Created |
| Tunnel, Traefik, health | Connected, Healthy | Starting | Unhealthy, Not connected | Off |

Raw values (`exited (0)`) stay available in a tooltip or a detail view, not as the badge.

## 5. Forms

- Label above the field (14px/500), help text below it (12px muted), the error below that (12px
  destructive). A field's width follows its content: hosts and names full width, ports and
  numbers narrow.
- Two columns from 768px up for short related fields; one column below.
- The submit button closes the section, bottom right. A form that saves shows a toast `Saved`
  and stays where it is.
- Adding a row to a list or table opens a **dialog** (`Add domain`, `Add port`), never an inline
  form above the list. Small inline edits (a toggle, a select) save on change with a toast.
- No raw native controls: file pickers are a `Button` over a hidden input (`FilePicker`), colors
  a swatch plus a hex input (`ColorField`), checkboxes and switches from the kit.
- Secrets follow their kind:
  - **Readable** (environment values, with `services.secrets`): masked; `Show` reveals them, and
    each reveal is audited; they mask again when the section is left.
  - **Write-only** (registry passwords, provider keys, tunnel tokens): the field says `Kept` and
    stays empty; typing replaces, an explicit `Remove` clears. Never a `Show`.
  - **Shown once** (API tokens): one view with Copy and "it is not shown again".
- A form with unsaved changes says so before the user leaves its tab; a save rejected because
  someone else changed the record explains it and offers to reload.

## 6. Feedback

| Situation | Use |
|---|---|
| An action succeeded | toast (`props.toast`), short: `Deployed`, `Token revoked` |
| An action failed | toast with the error, or `ErrorNote` in the form that caused it |
| A lasting state the user must know | `Alert` at the top of the section it concerns (one size: 14px) |
| Confirming something destructive or disruptive | `ConfirmDialog` (destructive button; typed name for the irreversible: reboot, delete a project or a volume) |
| Nothing to show yet | `EmptyState`: one 14px line, an optional 12px hint and an optional action, centered in the section |
| Loading a whole page | kit `Loading` |
| Loading a block of known shape | kit `Skeleton` of that shape, so nothing jumps |

`window.alert`, `window.confirm` and `window.prompt` are not used.

### Work that takes time (deploy, pull, backup, restart, prune)
- The button that starts it shows its pending label (`Deploying…`) and is disabled until the
  server answers: no double submit.
- The answer is "accepted", not "done": the toast says `Deployment queued` with a link to it.
- Its progress lives where it belongs (the deployment, the action) with a busy status and the
  live log; a dropped stream shows `Reconnecting…`, then the last state.
- Waiting for something to come back (a restart, a reboot) has a time limit; past it, an
  `Alert` says what to check and offers to retry.

### Dialogs
- Three widths through the kit `Dialog`'s `className`: compact 440px (confirmations), form
  560px (most forms), wide 880px (logs, Compose files, long forms). Taller content scrolls inside.
- Every dialog has a visible Cancel or Close; focus starts on the first field (or Cancel for a
  confirmation) and returns to the button that opened it; a failed submit focuses the first
  field in error, which is linked to its message (`aria-describedby`).

## 7. Shared components (`lib/`)

Their CSS classes keep the `rdp2-` prefix (the plain-JS pages' `panel.css` owns `rdp-` until R5).

| Component | What |
|---|---|
| `Stack` | the vertical 24px rhythm of a page's sections (also inside a tab) |
| `Section` | card + header (title, description, actions) + body |
| `ScrollTabs` | the kit tabs on one row that scrolls sideways |
| `TableCard` | a `Section` whose body is a kit `Table`, with toolbar and empty state |
| `RowActions` | up to two ghost buttons and a `⋯` menu |
| `IconButton` | `ghost` `icon-sm` button with tooltip and `aria-label` |
| `StatusBadge` | dot + label from the status vocabulary |
| `Metric` | the metric tile |
| `Facts` | label/value grid |
| `EmptyState` | the empty line, hint and action |
| `ConfirmDialog` | the confirmation dialog (optional typed confirmation) |
| `FilePicker`, `ColorField` | the styled file and color inputs |
| `icons` | the one icon set (Lucide shapes, 16px, stroke 2, `currentColor`) |

## 8. Phone and dark mode

- Every page works at 390px wide with no sideways page scroll.
  - Tabs scroll sideways in one row.
  - Page header actions wrap under the title; beyond two, the rest goes into a `⋯` menu.
  - Grids become one column.
- Every page works in light and dark: colors only from tokens (section 1). Check both.

## 9. Words and formats

- Name things as users know them (`Domain`, `Environment`), with the action first on buttons.
- Every string goes through `t()` with its Vietnamese translation in `lib/i18n.ts` (or
  `lang/vi.toml` for server-side labels) in the same change. Vietnamese runs about a third
  longer: check labels and buttons at 390px in Vietnamese.
- Numbers, percentages and dates through shared helpers on `Intl.NumberFormat` and
  `Intl.DateTimeFormat` in the user's locale. Sizes are 1024-based, written KB, MB, GB as Docker
  does. A relative time ("2 hours ago") has the absolute time, with its time zone, in a tooltip.

## 10. Checklist for a new or changed page

- [ ] `PageHeader` with eyebrow and title; tabs right under it; sections in a `Stack`.
- [ ] Buttons: size by place, one primary per region, no `+`, no glyphs or emoji.
- [ ] Collections in `TableCard`, row actions in `RowActions`, statuses in `StatusBadge`.
- [ ] Empty, loading and error states drawn with the shared components.
- [ ] No `window.confirm`/`alert`; destructive actions in `ConfirmDialog`.
- [ ] No new CSS class, no literal color, sizes from the type scale.
- [ ] Long work shows pending, queued and done states; dialogs have the right width and focus.
- [ ] Checked at 1440px, 768px and 390px, light and dark, English and Vietnamese.

## 11. Needs from RustCMS

- A structured breadcrumb for sub-pages (`Dashboard › demo › web`, each part a link), and a way
  to hide the shell's heading without changing the breadcrumb. Until then, section 2's eyebrow
  links carry the path.
