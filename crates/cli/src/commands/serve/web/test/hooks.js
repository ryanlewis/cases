// Lets node run the inbox's modules as they are: a relative import of
// "./x.js" from a module in static/ loads "./x.ts", as build.rs's output
// would have it. node strips the types itself.
import { registerHooks } from "node:module";
import { existsSync } from "node:fs";
import { fileURLToPath } from "node:url";

registerHooks({
  resolve(specifier, context, next) {
    if (specifier.startsWith(".") && specifier.endsWith(".js") && context.parentURL?.endsWith(".ts")) {
      const ts = new URL(specifier.replace(/\.js$/, ".ts"), context.parentURL);
      if (existsSync(fileURLToPath(ts))) return next(ts.href, context);
    }
    return next(specifier, context);
  },
});

// markdown.ts reads a link's target through an element it never puts in the
// page: it sets the element's HTML to an <a> with the target as its href, and
// reads the href back, character references decoded, as the browser would.
// This stand-in does that for the one element markdown.ts writes. The page's
// own origin is the inbox's.
const references = { amp: "&", lt: "<", gt: ">", quot: '"', apos: "'" };
function decoded(text) {
  return text.replace(/&(?:#x([0-9a-f]+)|#(\d+)|(\w+));/gi, (all, hex, dec, name) =>
    hex ? String.fromCodePoint(parseInt(hex, 16)) : dec ? String.fromCodePoint(Number(dec)) : references[name] ?? all);
}
globalThis.document ??= {
  createElement: () => ({
    set innerHTML(html) {
      const href = decoded(/^<a href="([^"]*)"><\/a>$/.exec(html)[1]);
      this.firstChild = { getAttribute: () => href };
    },
  }),
};
globalThis.location ??= { origin: "http://127.0.0.1:7337" };
