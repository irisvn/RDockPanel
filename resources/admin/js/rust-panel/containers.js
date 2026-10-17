// The `containers` page: every container with its state, start/stop/…
// buttons, and a panel with its logs or its inspect document. The list
// refreshes every 5 s; logs every 3 s while "Follow" is on (server-sent
// events replace the polling once the framework has them).
window.rustcms.registerPage("rust-panel/containers", {
  apiVersion: 1,
  mount(element, ctx) {
    const { api, h, date, badge, button, act, poll, table, page } = window.rustPanel;
    const t = ctx.t;
    const can = ctx.data?.can ?? {};
    let alive = true;
    let containers = [];
    let open = null; // { id, name, tab }

    const search = h("input", { type: "search", className: "rp-input", placeholder: t("Search"), oninput: () => renderList() });
    const running = h("input", { type: "checkbox", onchange: () => renderList() });
    const refresh = button(t("Refresh"), () => loadList());
    const { body, note } = page(element, t, "Containers", [search, h("label", { className: "rp-check" }, running, t("Only running")), refresh]);
    const list = h("div");
    const detail = h("section", { className: "rp-detail", hidden: true });
    body.append(list, detail);

    const stateKind = (state) => (state === "running" ? "ok" : state === "paused" || state === "restarting" ? "warn" : "muted");
    const path = (id, rest = "") => `/docker/containers/${encodeURIComponent(id)}${rest}`;

    function actions(c) {
      if (!can["docker.manage"]) return [];
      const run = (action, label) =>
        button(t(label), (event) => act(event.target, note, () => api("POST", path(c.id, `/${action}`)), t("Done.")).then(loadList));
      const buttons = [];
      if (c.state === "running") buttons.push(run("restart", "Restart"), run("stop", "Stop"), run("pause", "Pause"));
      else if (c.state === "paused") buttons.push(run("unpause", "Unpause"));
      else buttons.push(run("start", "Start"));
      buttons.push(
        button(
          t("Remove"),
          (event) => {
            if (!window.confirm(t("Remove {name}?").replace("{name}", c.name))) return;
            act(event.target, note, () => api("DELETE", path(c.id, "?force=true")), t("Removed.")).then(() => {
              if (open?.id === c.id) closeDetail();
              loadList();
            });
          },
          "danger",
        ),
      );
      return h("div", { className: "rp-actions" }, buttons);
    }

    function renderList() {
      const term = search.value.trim().toLowerCase();
      const shown = containers.filter(
        (c) => (!running.checked || c.state === "running") && (!term || c.name.toLowerCase().includes(term) || c.image.toLowerCase().includes(term)),
      );
      const rows = shown.map((c) => [
        h("button", { type: "button", className: "rp-link", onclick: () => openDetail(c, can["docker.logs"] ? "logs" : "inspect") }, c.name),
        h("span", { className: "rp-mono" }, c.image),
        badge(c.state, stateKind(c.state)),
        h("span", { title: date(c.created) }, c.status),
        h("span", { className: "rp-mono" }, c.ports.join(", ")),
        actions(c),
      ]);
      list.replaceChildren(table([t("Name"), t("Image"), t("State"), t("Status"), t("Ports"), t("Actions")], rows, t("No containers.")));
    }

    async function loadList() {
      try {
        containers = (await api("GET", "/docker/containers")).containers;
        containers.sort((a, b) => a.name.localeCompare(b.name));
        if (alive) renderList();
      } catch (err) {
        note.error(err);
      }
    }

    // The detail panel: tabs, and the logs (with their own polling) or inspect.
    let stopLogs = () => {};
    function closeDetail() {
      stopLogs();
      open = null;
      detail.hidden = true;
      detail.replaceChildren();
    }

    function openDetail(c, tab) {
      stopLogs();
      open = { id: c.id, name: c.name, tab };
      const tabs = [];
      if (can["docker.logs"]) tabs.push(["logs", "Logs"]);
      tabs.push(["inspect", "Inspect"]);
      const nav = h(
        "div",
        { className: "rp-tabs", role: "tablist" },
        tabs.map(([key, label]) =>
          h("button", { type: "button", role: "tab", "aria-selected": String(key === tab), className: `rp-tab${key === tab ? " rp-tab-active" : ""}`, onclick: () => openDetail(c, key) }, t(label)),
        ),
      );
      const content = h("div", { className: "rp-detail-body" });
      detail.replaceChildren(
        h("header", { className: "rp-detail-header" }, h("h2", { className: "rp-card-title" }, c.name), nav, button(t("Close"), closeDetail)),
        content,
      );
      detail.hidden = false;
      if (tab === "logs") showLogs(c, content);
      else showInspect(c, content);
      detail.scrollIntoView({ block: "nearest", behavior: "smooth" });
    }

    function showLogs(c, content) {
      const lines = h("select", { className: "rp-input" }, [100, 500, 2000].map((n) => h("option", { value: n }, String(n))));
      lines.value = "500";
      const follow = h("input", { type: "checkbox", checked: true });
      const output = h("pre", { className: "rp-log", tabindex: "0" });
      const info = h("p", { className: "rp-muted", hidden: true });
      content.replaceChildren(
        h("div", { className: "rp-toolbar" }, h("label", { className: "rp-check" }, t("Lines"), lines), h("label", { className: "rp-check" }, follow, t("Follow"))),
        info,
        output,
      );
      const load = async () => {
        if (!follow.checked && output.childNodes.length) return;
        try {
          const result = await api("GET", path(c.id, `/logs?tail=${lines.value}`));
          if (open?.id !== c.id) return;
          const stick = output.scrollTop + output.clientHeight >= output.scrollHeight - 8;
          output.replaceChildren(
            ...(result.lines.length
              ? result.lines.map((l) => h("span", { className: l.stream === "stderr" ? "rp-log-err" : "" }, l.text.endsWith("\n") ? l.text : `${l.text}\n`))
              : [h("span", { className: "rp-muted" }, t("No output."))]),
          );
          info.hidden = !result.truncated;
          info.textContent = t("Output cut: too large.");
          if (stick) output.scrollTop = output.scrollHeight;
        } catch (err) {
          note.error(err);
        }
      };
      lines.addEventListener("change", () => {
        output.replaceChildren();
        load();
      });
      stopLogs = poll(load, 3000);
    }

    async function showInspect(c, content) {
      const output = h("pre", { className: "rp-log", tabindex: "0" });
      content.replaceChildren(output);
      try {
        const doc = await api("GET", path(c.id));
        output.textContent = JSON.stringify(doc, null, 2);
      } catch (err) {
        note.error(err);
      }
    }

    const stopList = poll(loadList, 5000);
    return {
      update() {},
      unmount() {
        alive = false;
        stopList();
        stopLogs();
        element.replaceChildren();
      },
    };
  },
});
