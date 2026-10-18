// The `containers` page: every container with its state, start/stop/…
// buttons, and a panel with its logs, its live stats or its inspect
// document. The list refreshes every 5 s; logs and stats are server-sent
// events.
window.rustcms.registerPage("rust-panel/containers", {
  apiVersion: 1,
  mount(element, ctx) {
    const { api, h, bytes, date, badge, button, act, poll, table, page, showState } = window.rustPanel;
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
      if (c.state === "running") tabs.push(["stats", "Stats"]);
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
      else if (tab === "stats") showStats(c, content);
      else showInspect(c, content);
      detail.scrollIntoView({ block: "nearest", behavior: "smooth" });
    }

    // Logs: the last lines, then new ones as they come while "Follow" is on.
    function showLogs(c, content) {
      const MAX_LINES = 5000;
      const lines = h("select", { className: "rp-input" }, [100, 500, 2000].map((n) => h("option", { value: n }, String(n))));
      lines.value = "500";
      const follow = h("input", { type: "checkbox", checked: true });
      const output = h("pre", { className: "rp-log", tabindex: "0", "aria-live": "off" });
      content.replaceChildren(
        h("div", { className: "rp-toolbar" }, h("label", { className: "rp-check" }, t("Lines"), lines), h("label", { className: "rp-check" }, follow, t("Follow"))),
        output,
      );
      const append = (batch) => {
        const stick = output.scrollTop + output.clientHeight >= output.scrollHeight - 8;
        output.querySelector(".rp-log-empty")?.remove();
        output.append(...batch.map((l) => h("span", { className: l.stream === "stderr" ? "rp-log-err" : "" }, l.text.endsWith("\n") ? l.text : `${l.text}\n`)));
        while (output.childNodes.length > MAX_LINES) output.firstChild.remove();
        if (stick) output.scrollTop = output.scrollHeight;
      };
      let stream = null;
      const start = () => {
        stream?.close();
        stream = ctx.realtime.events(path(c.id, `/logs/follow?tail=${lines.value}`), {
          onEvent(name, data) {
            if (name === "lines") append(data.lines);
          },
          onState(state) {
            // The server sends the last lines again after each (re)connect.
            if (state.state === "open") output.replaceChildren(h("span", { className: "rp-muted rp-log-empty" }, t("No output.")));
            showState(note, t, state);
          },
        });
      };
      follow.addEventListener("change", () => (follow.checked ? start() : stream?.close()));
      lines.addEventListener("change", () => (follow.checked ? start() : null));
      start();
      stopLogs = () => stream?.close();
    }

    // Stats: CPU, memory, network and disk I/O, live.
    function showStats(c, content) {
      const value = (label) => {
        const out = h("div", { className: "rp-card-value" }, "–");
        return [h("section", { className: "rp-card" }, h("h3", { className: "rp-card-title" }, t(label)), out), out];
      };
      const [cpuCard, cpu] = value("CPU");
      const [memCard, mem] = value("Memory");
      const [netCard, net] = value("Network");
      const [diskCard, disk] = value("Disk");
      content.replaceChildren(h("div", { className: "rp-cards" }, cpuCard, memCard, netCard, diskCard));
      const stream = ctx.realtime.events(path(c.id, "/stats"), {
        onEvent(name, s) {
          if (name !== "stats") return;
          cpu.textContent = `${s.cpu_percent.toFixed(1)}%`;
          mem.textContent = s.mem_limit ? `${bytes(s.mem_used)} / ${bytes(s.mem_limit)}` : bytes(s.mem_used);
          net.textContent = `${t("In")} ${bytes(s.net_rx)} · ${t("Out")} ${bytes(s.net_tx)}`;
          disk.textContent = `${t("Read")} ${bytes(s.blk_read)} · ${t("Written")} ${bytes(s.blk_write)}`;
        },
        onState: (state) => showState(note, t, state),
      });
      stopLogs = () => stream.close();
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
