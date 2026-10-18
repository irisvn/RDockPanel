// The `volumes` page: Docker volumes, creating and removing them.
window.rustcms.registerPage("rdockpanel/volumes", {
  apiVersion: 1,
  mount(element, ctx) {
    const { api, h, button, act, table, page } = window.rdockPanel;
    const t = ctx.t;
    const can = ctx.data?.can ?? {};
    let alive = true;

    const { body, note } = page(element, t, "Volumes", [button(t("Refresh"), () => load())]);
    if (can["docker.manage"]) {
      const name = h("input", { type: "text", className: "rdp-input rdp-grow", placeholder: t("New volume name"), required: true });
      const submit = h("button", { type: "submit", className: "rdp-btn rdp-btn-primary" }, t("Create"));
      body.append(
        h(
          "form",
          {
            className: "rdp-form",
            onsubmit: async (event) => {
              event.preventDefault();
              if (await act(submit, note, () => api("POST", "/docker/volumes", { name: name.value.trim() }), t("Created."))) {
                name.value = "";
                load();
              }
            },
          },
          name,
          submit,
        ),
      );
    }
    const list = h("div");
    body.append(list);

    async function load() {
      try {
        const volumes = (await api("GET", "/docker/volumes")).volumes;
        volumes.sort((a, b) => a.name.localeCompare(b.name));
        if (!alive) return;
        const rows = volumes.map((v) => [
          h("span", { className: "rdp-mono" }, v.name),
          v.driver,
          h("span", { className: "rdp-mono rdp-muted" }, v.mountpoint),
          can["docker.manage"]
            ? button(
                t("Remove"),
                (event) => {
                  if (!window.confirm(t("Remove {name}?").replace("{name}", v.name))) return;
                  act(event.target, note, () => api("DELETE", `/docker/volumes/${encodeURIComponent(v.name)}`), t("Removed.")).then(load);
                },
                "danger",
              )
            : "",
        ]);
        list.replaceChildren(table([t("Name"), t("Driver"), t("Mountpoint"), t("Actions")], rows, t("No volumes.")));
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
