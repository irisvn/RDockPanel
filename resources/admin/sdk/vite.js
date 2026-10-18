// The Vite preset of RustCMS admin extensions (ADR 0032): one entry per
// build, an IIFE, the host's modules external, the CSS extracted, nothing
// inline (the panel's CSP allows only its own files).
import path from "node:path";

/** Import ids mapped to the panel's instances (`window.rustcms.v2.modules`). */
const GLOBALS = {
  react: "rustcms.v2.modules.react",
  "react/jsx-runtime": 'rustcms.v2.modules["react/jsx-runtime"]',
  "react-dom": 'rustcms.v2.modules["react-dom"]',
  "@tanstack/react-query": 'rustcms.v2.modules["@tanstack/react-query"]',
  "@rustcms/admin-sdk/ui": "rustcms.v2.modules.ui",
  "@rustcms/admin-sdk/host": "rustcms.v2.modules.host",
};
const REFUSED = /^(react\/jsx-dev-runtime|react-dom\/.+|react\/.+|@tanstack\/react-query\/.+)$/;

/** Refuses imports the panel does not provide, and dynamic imports. */
function guard() {
  return {
    name: "rustcms-extension-guard",
    enforce: "pre",
    resolveId(id) {
      if (REFUSED.test(id) && !(id in GLOBALS)) {
        this.error(`rustcms: \`${id}\` is not provided by the panel; import one of ${Object.keys(GLOBALS).join(", ")}`);
      }
      return null;
    },
    // Every `import()`, local ones too (they would be inlined otherwise).
    resolveDynamicImport() {
      this.error("rustcms: dynamic imports are not supported (one file per entry)");
    },
  };
}

export function rustcmsExtension({ entry, root = process.cwd() }) {
  const rel = path.relative(path.join(root, "src"), entry).replace(/\\/g, "/");
  const match = /^([a-z0-9-]+)\/([a-z0-9-]+)\.tsx$/.exec(rel);
  if (!match) throw new Error(`rustcms: entries are src/<vendor>/<name>.tsx (got ${rel})`);
  const [, vendor, name] = match;
  return {
    root,
    configFile: false,
    mode: "production",
    define: { "process.env.NODE_ENV": JSON.stringify("production") },
    esbuild: { jsx: "automatic" },
    plugins: [guard()],
    // The output folder is the app's `resources/admin` (Vite warns about it; harmless).
    logLevel: "error",
    build: {
      // Outputs of other entries stay: each build writes its own two files.
      emptyOutDir: false,
      outDir: root,
      minify: true,
      sourcemap: false,
      cssCodeSplit: false,
      assetsInlineLimit: 0,
      modulePreload: false,
      lib: {
        entry,
        formats: ["iife"],
        name: `rustcmsExtension_${vendor}_${name}`.replace(/-/g, "_"),
        fileName: () => `js/${vendor}/${name}.js`,
        cssFileName: `css/${vendor}/${name}`,
      },
      rollupOptions: {
        external: Object.keys(GLOBALS),
        output: { globals: GLOBALS, inlineDynamicImports: true },
      },
    },
  };
}
