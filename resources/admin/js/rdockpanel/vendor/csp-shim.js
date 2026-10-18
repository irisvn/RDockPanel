// xterm.js under the panel's CSP (style-src 'self', no inline styles).
//
// xterm creates <style> elements with document.createElement, appends them,
// then sets their text (scrollbar, renderer dimensions and theme) or calls
// sheet.insertRule: the CSP blocks both (xterm.js issues #4445, #4133). For
// <style> elements made by script (never parsed markup, so injected HTML
// still cannot add styles), this shim keeps the rules in a constructed
// CSSStyleSheet instead, adopted by the document while the element is
// "attached":
// - appendChild / insertBefore attach it (it is not put into the DOM: an
//   inline <style> there is what the CSP refuses);
// - removeChild / remove detach it (adopted rules go away, as the element's
//   would); attaching again adopts the same rules again;
// - textContent and sheet work on those rules, attached or not (rules of a
//   detached element apply only once it is attached).
// Loaded before any `new Terminal()` (from RustCMS-Core's W6 spike).
(() => {
  const made = new WeakSet();
  const attached = new WeakSet();
  const sheets = new WeakMap();
  const texts = new WeakMap();
  const text = Object.getOwnPropertyDescriptor(Node.prototype, "textContent");
  const sheetOf = Object.getOwnPropertyDescriptor(HTMLStyleElement.prototype, "sheet");
  const sheetFor = (element) => {
    let sheet = sheets.get(element);
    if (!sheet) {
      sheet = new CSSStyleSheet();
      sheets.set(element, sheet);
    }
    return sheet;
  };
  const attach = (element) => {
    attached.add(element);
    const sheet = sheetFor(element);
    if (!document.adoptedStyleSheets.includes(sheet)) document.adoptedStyleSheets = [...document.adoptedStyleSheets, sheet];
  };
  const detach = (element) => {
    attached.delete(element);
    const sheet = sheets.get(element);
    if (sheet) document.adoptedStyleSheets = document.adoptedStyleSheets.filter((s) => s !== sheet);
  };

  const createElement = Document.prototype.createElement;
  Document.prototype.createElement = function (tag, options) {
    const element = createElement.call(this, tag, options);
    if (element instanceof HTMLStyleElement) made.add(element);
    return element;
  };
  for (const name of ["appendChild", "insertBefore"]) {
    const insert = Node.prototype[name];
    Node.prototype[name] = function (child, ...rest) {
      if (!made.has(child)) return insert.call(this, child, ...rest);
      attach(child);
      return child;
    };
  }
  const removeChild = Node.prototype.removeChild;
  Node.prototype.removeChild = function (child) {
    if (!made.has(child)) return removeChild.call(this, child);
    detach(child);
    return child;
  };
  const remove = Element.prototype.remove;
  Element.prototype.remove = function () {
    if (made.has(this)) detach(this);
    else remove.call(this);
  };
  Object.defineProperty(HTMLStyleElement.prototype, "textContent", {
    configurable: true,
    get() {
      return made.has(this) ? (texts.get(this) ?? "") : text.get.call(this);
    },
    set(value) {
      if (!made.has(this)) return text.set.call(this, value);
      texts.set(this, String(value ?? ""));
      sheetFor(this).replaceSync(texts.get(this));
    },
  });
  Object.defineProperty(HTMLStyleElement.prototype, "sheet", {
    configurable: true,
    get() {
      return made.has(this) ? sheetFor(this) : sheetOf.get.call(this);
    },
  });
})();
