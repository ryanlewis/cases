// Capture the README screenshots from a running `cases serve`.
// Usage: node capture.cjs URL OUTDIR  (needs the playwright package, found
// through NODE_PATH or a node_modules above this file)
const { chromium } = require("playwright");

const [base, out] = process.argv.slice(2);

async function shot(browser, scheme, path, file, width, height) {
  const page = await browser.newPage({
    viewport: { width, height },
    colorScheme: scheme,
    bypassCSP: true,
  });
  // Cross-document view transitions can stop headless Chromium painting.
  await page.addInitScript(() => {
    document.addEventListener("DOMContentLoaded", () => {
      const s = document.createElement("style");
      s.textContent = "@view-transition { navigation: none; }";
      document.head.append(s);
    });
  });
  await page.goto(base + path, { waitUntil: "networkidle" });
  await page.screenshot({ path: `${out}/${file}-${scheme}.png` });
  await page.close();
}

async function main() {
  // CHROMIUM names a browser to use in place of the one playwright downloads.
  const browser = await chromium.launch({ executablePath: process.env.CHROMIUM || undefined });

  const res = await fetch(base + "/");
  const html = await res.text();
  const link = (slug) => html.match(new RegExp(`href="(/cases/[^"]*${slug})"`))[1];

  for (const scheme of ["light", "dark"]) {
    await shot(browser, scheme, link("stream-uploads-or-raise-the-size-limit"), "inbox", 1280, 800);
    await shot(browser, scheme, link("run-the-data-migration-on-staging"), "case", 420, 900);
  }
  await browser.close();
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
