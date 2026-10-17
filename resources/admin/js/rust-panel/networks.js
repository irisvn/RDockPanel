// The `networks` page: Docker networks, creating (bridge or overlay) and
// removing them. Docker's own (bridge, host, none, ingress) can't be removed.
window.rustcms.registerPage("rust-panel/networks", {
  apiVersion: 1,
  mount(element, ctx) {
    const { api, h, shortId, badge, button, act, table, page } = window.rustPanel;
    const t = ctx.t;
    const can = ctx.data?.can ?? {};
    const builtIn = new Set(["bridge", "host", "none", "ingress", "docker_gwbridge"]);
    let alive = true;

    const { body, note } = page(element, t, "Networks", [button(t("Refresh"), () => load())]);
    if (can["docker.manage"]) {
      const name = h("input", { type: "text", className: "rp-input rp-grow", placeholder: t("New network name"), required: true });
      const driver = h(
        "select",
        { className: "rp-input" },
        h("option", { value: "overlay" }, t("overlay (the swarm)")),
        h("option", { value: "bridge" }, t("bridge (this server)")),
      );
      const submit = h("button", { type: "submit", className: "rp-btn rp-btn-primary" }, t("Create"));
      body.append(
        h(
          "form",
          {
            className: "rp-form",
            onsubmit: async (event) => {
              event.preventDefault();
              const payload = { name: name.value.trim(), driver: driver.value };
              if (await act(submit, note, () => api("POST", "/docker/networks", payload), t("Created."))) {
                name.value = "";
                load();
              }
            },
          },
          name,
          driver,
          submit,
        ),
      );
    }
    const list = h("div");
    body.append(list);

    async function load() {
      try {
        const networks = (await api("GET", "/docker/networks")).networks;
        networks.sort((a, b) => a.name.localeCompare(b.name));
        if (!alive) return;
        const rows = networks.map((n) => [
          h("span", { className: "rp-mono" }, n.name),
          h("span", { className: "rp-mono rp-muted", title: n.id }, shortId(n.id)),
          n.driver,
          badge(n.scope, n.scope === "swarm" ? "ok" : "muted"),
          can["docker.manage"] && !builtIn.has(n.name)
            ? button(
                t("Remove"),
                (event) => {
                  if (!window.confirm(t("Remove {name}?").replace("{name}", n.name))) return;
                  act(event.target, note, () => api("DELETE", `/docker/networks/${encodeURIComponent(n.id)}`), t("Removed.")).then(load);
                },
                "danger",
              )
            : "",
        ]);
        list.replaceChildren(table([t("Name"), t("ID"), t("Driver"), t("Scope"), t("Actions")], rows, t("No networks.")));
      } catch (err) {
        note.error(err);
      }
    }

    load();
    return {
      update() {},
      unmount() {
        alive = false;
        element.replaceChildren();
      },
    };
  },
});
