// The `images` page: images on this server, pulling one (a queued job whose
// progress is followed through its action), removing, pruning.
window.rustcms.registerPage("rust-panel/images", {
  apiVersion: 1,
  mount(element, ctx) {
    const { api, h, bytes, date, shortId, badge, button, act, poll, table, page } = window.rustPanel;
    const t = ctx.t;
    const can = ctx.data?.can ?? {};
    let alive = true;
    let images = [];

    const toolbar = [button(t("Refresh"), () => load())];
    if (can["docker.manage"]) {
      toolbar.push(
        button(
          t("Prune"),
          (event) => {
            if (!window.confirm(t("Remove stopped containers and dangling images? The panel's own are kept."))) return;
            act(event.target, note, () => api("POST", "/docker/prune")).then((result) => {
              if (!result) return;
              const r = result.report;
              note.ok(
                t("Removed {n} containers and {m} images, freed {size}.")
                  .replace("{n}", r.containers_deleted)
                  .replace("{m}", r.images_deleted)
                  .replace("{size}", bytes(r.space_reclaimed)),
              );
              load();
            });
          },
          "danger",
        ),
      );
    }
    const { body, note } = page(element, t, "Images", toolbar);

    // Pull: queue it, then follow its action until it ends.
    const progress = h("pre", { className: "rp-log rp-log-short", hidden: true });
    if (can["docker.manage"]) {
      const reference = h("input", { type: "text", className: "rp-input rp-grow", placeholder: t("Image to pull, e.g. nginx:1.27"), required: true });
      const submit = h("button", { type: "submit", className: "rp-btn rp-btn-primary" }, t("Pull"));
      const form = h(
        "form",
        {
          className: "rp-form",
          onsubmit: async (event) => {
            event.preventDefault();
            const queued = await act(submit, note, () => api("POST", "/docker/images/pull", { image: reference.value }));
            if (queued && alive) follow(queued.action.id, submit);
          },
        },
        reference,
        submit,
      );
      body.append(form, progress);
    }
    const list = h("div");
    body.append(list);

    let stopFollow = () => {};
    function follow(id, submit) {
      stopFollow();
      submit.disabled = true;
      progress.hidden = false;
      progress.textContent = t("Pulling…");
      const end = () => {
        stopFollow();
        stopFollow = () => {};
        submit.disabled = false;
      };
      stopFollow = poll(async () => {
        if (!alive) return end();
        try {
          const { action } = await api("GET", `/docker/actions/${id}`);
          if (!alive) return end();
          if (action.log) {
            progress.textContent = action.log;
            progress.scrollTop = progress.scrollHeight;
          }
          if (action.status === "ok" || action.status === "failed") {
            end();
            if (action.status === "ok") note.ok(t("Pulled."));
            else note.error(`${t("Pull failed")}: ${action.error ?? ""}`);
            load();
          }
        } catch (err) {
          note.error(err);
          // Signed out, no longer allowed, or gone: following can't recover.
          if ([401, 403, 404].includes(err.status)) {
            end();
            if (err.status === 401) ctx.reload();
          }
        }
      }, 1500);
    }

    function remove(image, tag, event) {
      if (!window.confirm(t("Remove {name}?").replace("{name}", tag))) return;
      // `containers` is -1 when Docker did not count: maybe in use.
      const force = image.containers !== 0 && can["system.manage"] && window.confirm(t("Force (also when in use)"));
      const query = force ? "?force=true" : "";
      act(event.target, note, () => api("DELETE", `/docker/images/${encodeURIComponent(tag)}${query}`), t("Removed.")).then(load);
    }

    function render() {
      const rows = images.map((image) => {
        const tags = image.tags.length ? image.tags : [shortId(image.id)];
        return [
          h("div", { className: "rp-stack" }, tags.map((tag) => h("span", { className: "rp-mono" }, tag))),
          h("span", { className: "rp-mono", title: image.id }, shortId(image.id)),
          bytes(image.size),
          date(image.created),
          image.containers > 0 ? badge(t("In use"), "ok") : "",
          can["docker.manage"]
            ? h("div", { className: "rp-actions" }, tags.map((tag) => button(`${t("Remove")} ${image.tags.length > 1 ? tag : ""}`.trim(), (event) => remove(image, image.tags.length ? tag : image.id, event), "danger")))
            : "",
        ];
      });
      list.replaceChildren(table([t("Tags"), t("ID"), t("Size"), t("Created"), t("Status"), t("Actions")], rows, t("No images.")));
    }

    async function load() {
      try {
        images = (await api("GET", "/docker/images")).images;
        images.sort((a, b) => b.created - a.created);
        if (alive) render();
      } catch (err) {
        note.error(err);
      }
    }

    load();
    return {
      update() {},
      unmount() {
        alive = false;
        stopFollow();
        element.replaceChildren();
      },
    };
  },
});
