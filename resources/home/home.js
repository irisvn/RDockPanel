// The homepage's screens, larger, in a dialog: previous and next (buttons,
// arrow keys, a swipe), closed with ×, Escape or a click beside the image.
// Without this script each image still opens on its own.
(() => {
  const dialog = document.getElementById("viewer");
  const links = [...document.querySelectorAll("a[data-slide]")];
  if (!dialog || !links.length || typeof dialog.showModal !== "function") return;
  const image = dialog.querySelector("img");
  const caption = dialog.querySelector("figcaption");
  const count = dialog.querySelector("[data-count]");
  let at = 0;
  let opener = null;

  const show = (i) => {
    at = (i + links.length) % links.length;
    const link = links[at];
    const thumb = link.querySelector("img");
    image.src = link.href;
    image.alt = thumb ? thumb.alt : "";
    caption.textContent = link.dataset.caption || "";
    count.textContent = `${at + 1} / ${links.length}`;
  };

  links.forEach((link, i) =>
    link.addEventListener("click", (e) => {
      if (e.metaKey || e.ctrlKey || e.shiftKey || e.button !== 0) return;
      e.preventDefault();
      opener = link;
      show(i);
      dialog.showModal();
    }),
  );
  dialog.querySelector("[data-prev]").addEventListener("click", () => show(at - 1));
  dialog.querySelector("[data-next]").addEventListener("click", () => show(at + 1));
  dialog.querySelector("[data-close]").addEventListener("click", () => dialog.close());
  dialog.addEventListener("click", (e) => {
    if (e.target === dialog) dialog.close();
  });
  dialog.addEventListener("keydown", (e) => {
    if (e.key === "ArrowLeft") show(at - 1);
    else if (e.key === "ArrowRight") show(at + 1);
  });
  // A swipe sideways (mostly): a scroll up or down changes nothing.
  let start = null;
  dialog.addEventListener("touchstart", (e) => (start = { x: e.touches[0].clientX, y: e.touches[0].clientY }), { passive: true });
  dialog.addEventListener("touchend", (e) => {
    if (start === null) return;
    const dx = e.changedTouches[0].clientX - start.x;
    const dy = e.changedTouches[0].clientY - start.y;
    if (Math.abs(dx) > 40 && Math.abs(dx) > 2 * Math.abs(dy)) show(dx < 0 ? at + 1 : at - 1);
    start = null;
  });
  dialog.addEventListener("close", () => opener && opener.focus());
})();
