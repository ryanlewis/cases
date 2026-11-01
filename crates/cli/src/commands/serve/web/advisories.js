// Checks the inbox's JavaScript against the npm registry's advisories: the
// code the binary serves, and the npm tree that tsc and the tests run on.
//
// What the binary serves is static/vendor/, not node_modules. Each library
// there is the release vendor/README.md records, and mermaid's bundle
// embeds code from its own dependencies at the versions it was built with,
// which need not be the ones npm resolves. Those are listed in
// vendor-inventory.json, each with the text in the file that names its
// version. This script fails when:
//
// - a vendored file names a bundled package the inventory does not list,
//   or the inventory's text is not in its file (the bundle changed);
// - the registry has an advisory against a vendored release, a bundled
//   package or a package in package-lock.json that advisory-exceptions.json
//   does not except, with its reason;
// - an exception matches no advisory any more, so it is removed.
//
// It installs nothing and runs no package's code: it reads files and sends
// the names and versions to the registry's bulk advisory endpoint, the one
// npm audit uses.
//
//   node advisories.js
import { readFileSync, readdirSync, statSync } from "node:fs";
import { join, relative } from "node:path";
import { fileURLToPath } from "node:url";

const ENDPOINT = "https://registry.npmjs.org/-/npm/v1/security/advisories/bulk";

// The packages named in each "Bundled license information" block, the
// banners esbuild keeps: an entry is a line `<package>/<path>:`.
export function bundled(text) {
  const names = new Set();
  for (const block of text.replace(/\r\n?/g, "\n").matchAll(/Bundled license information:\n([\s\S]*?)\*\//g)) {
    for (const line of block[1].split("\n")) {
      const m = /^(@[^/\s]+\/[^/\s:]+|[^@/\s:][^/\s:]*)(?:\/\S*)?:$/.exec(line);
      if (m) names.add(m[1]);
    }
  }
  return [...names];
}

// The vendored releases, from the README's table: `| file | name version, licence |`.
export function vendored(readme) {
  const found = new Map();
  for (const m of readme.matchAll(/^\| `[^`]+` \| (\S+) (\d[^,\s]*), /gm)) found.set(`${m[1]}@${m[2]}`, { package: m[1], version: m[2], where: "vendored" });
  return [...found.values()];
}

// The packages in the lockfile, by name and version.
export function locked(lock) {
  const found = new Map();
  for (const [path, entry] of Object.entries(lock.packages ?? {})) {
    if (!path || entry.link || !entry.version) continue;
    const at = path.lastIndexOf("node_modules/");
    const name = entry.name ?? (at < 0 ? null : path.slice(at + "node_modules/".length));
    if (!name) throw new Error(`package-lock.json: no name for ${path}`);
    found.set(`${name}@${entry.version}`, { package: name, version: entry.version, where: "npm" });
  }
  return [...found.values()];
}

// The problems with the inventory, given each vendored file's text by its
// path from web/.
export function inventoryProblems(inventory, files) {
  const problems = [];
  for (const entry of inventory.bundled) {
    const text = files.get(entry.file);
    if (text === undefined) problems.push(`${entry.package}: ${entry.file} is not vendored`);
    else if (!entry.marker.includes(entry.version)) problems.push(`${entry.package}: its marker does not name ${entry.version}`);
    else if (!text.includes(entry.marker)) problems.push(`${entry.package} ${entry.version}: ${entry.file} does not contain ${JSON.stringify(entry.marker)}`);
  }
  for (const [file, text] of files) {
    for (const name of bundled(text)) {
      if (!inventory.bundled.some((e) => e.file === file && e.package === name)) problems.push(`${file} bundles ${name}, which vendor-inventory.json does not list`);
    }
  }
  return problems;
}

// The registry's advisories for `packages`, each against the one it hits.
export async function advisories(packages, post = fetch) {
  const body = {};
  for (const p of packages) (body[p.package] ??= []).push(p.version);
  const res = await post(ENDPOINT, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify(body) });
  if (!res.ok) throw new Error(`${ENDPOINT}: ${res.status} ${res.statusText}`);
  const found = await res.json();
  const hits = [];
  for (const p of packages) {
    for (const a of found[p.package] ?? []) {
      if (satisfies(p.version, a.vulnerable_versions)) hits.push({ ...p, id: ghsa(a), severity: a.severity, title: a.title, url: a.url });
    }
  }
  return hits;
}

// The endpoint answers by package, so each version is checked against the
// advisory's range here, as npm writes them: comparators on whole versions,
// `<=1.2.3 || >= 2.0.0 <2.1.0`, or `*`. Any other form (`^`, `~`, `1.x`,
// `1.0.0 - 2.0.0`) throws, so an advisory is never dropped unread.
const VERSION = "v?(\\d+)\\.(\\d+)\\.(\\d+)(?:-([0-9A-Za-z.-]+))?(?:\\+[0-9A-Za-z.-]+)?";
const COMPARATOR = new RegExp(`^(<=|>=|<|>|=)?\\s*${VERSION}$`);

export function satisfies(version, range) {
  const v = parse(version);
  return range.split("||").some((set) => {
    const tokens = set.trim().replace(/(<=|>=|<|>|=)\s+/g, "$1").split(/\s+/).filter(Boolean);
    if (!tokens.length || (tokens.length === 1 && tokens[0] === "*")) return true;
    return tokens.map((c) => {
      const m = COMPARATOR.exec(c);
      if (!m) throw new Error(`unread range: ${range}`);
      const d = compare(v, parse(c.slice(m[1]?.length ?? 0)));
      switch (m[1] ?? "=") {
        case "<": return d < 0;
        case "<=": return d <= 0;
        case ">": return d > 0;
        case ">=": return d >= 0;
        default: return d === 0;
      }
    }).every(Boolean);
  });
}

function parse(version) {
  const m = new RegExp(`^${VERSION}$`).exec(version.trim());
  if (!m) throw new Error(`unread version: ${version}`);
  return { core: [Number(m[1]), Number(m[2]), Number(m[3])], pre: m[4]?.split(".") ?? [] };
}

// Semver precedence: the release, then a prerelease before its release,
// compared by identifier, numbers by value and below words.
function compare(a, b) {
  for (let i = 0; i < 3; i++) if (a.core[i] !== b.core[i]) return a.core[i] - b.core[i];
  if (!a.pre.length || !b.pre.length) return b.pre.length - a.pre.length;
  for (let i = 0; i < Math.max(a.pre.length, b.pre.length); i++) {
    const x = a.pre[i], y = b.pre[i];
    if (x === undefined) return -1;
    if (y === undefined) return 1;
    if (x === y) continue;
    const nx = /^\d+$/.test(x), ny = /^\d+$/.test(y);
    if (nx && ny) return Number(x) - Number(y);
    if (nx) return -1;
    if (ny) return 1;
    return x < y ? -1 : 1;
  }
  return 0;
}

// An advisory's GHSA id, from its url; else the registry's own id.
function ghsa(advisory) {
  return /GHSA(-[a-z0-9]{4}){3}/.exec(advisory.url ?? "")?.[0] ?? `npm-${advisory.id}`;
}

// The hits no exception covers, and the exceptions no hit uses.
export function judge(hits, exceptions) {
  const key = (x) => `${x.id} ${x.package}@${x.version} ${x.where}`;
  const excepted = new Set(exceptions.map(key));
  const hit = new Set(hits.map(key));
  return {
    open: hits.filter((h) => !excepted.has(key(h))),
    stale: exceptions.filter((e) => !hit.has(key(e))),
  };
}

function files(dir, root, into = new Map()) {
  for (const name of readdirSync(dir)) {
    if (name.startsWith(".")) continue;
    const path = join(dir, name);
    if (statSync(path).isDirectory()) files(path, root, into);
    else if (/\.m?js$/.test(name)) into.set(relative(root, path), readFileSync(path, "utf8"));
  }
  return into;
}

async function main() {
  const web = fileURLToPath(new URL(".", import.meta.url));
  const read = (path) => readFileSync(join(web, path), "utf8");
  const inventory = JSON.parse(read("vendor-inventory.json"));
  const { exceptions } = JSON.parse(read("advisory-exceptions.json"));
  const problems = inventoryProblems(inventory, files(join(web, "static/vendor"), web));

  const packages = [...new Map([
    ...vendored(read("static/vendor/README.md")),
    ...inventory.bundled.map((e) => ({ package: e.package, version: e.version, where: "vendored" })),
    ...locked(JSON.parse(read("package-lock.json"))),
  ].map((p) => [`${p.package}@${p.version} ${p.where}`, p])).values()];
  const hits = await advisories(packages);
  const { open, stale } = judge(hits, exceptions);

  for (const h of hits) {
    const mark = open.includes(h) ? "OPEN" : "excepted";
    console.log(`${mark}: ${h.id} ${h.severity} ${h.package}@${h.version} (${h.where}): ${h.title}`);
  }
  for (const p of problems) console.log(`INVENTORY: ${p}`);
  for (const e of stale) console.log(`STALE: ${e.id} ${e.package}@${e.version} (${e.where}) matches no advisory; remove it from advisory-exceptions.json`);
  console.log(`${packages.length} packages checked: ${hits.length} advisories, ${open.length} open, ${stale.length} stale exceptions, ${problems.length} inventory problems`);
  if (open.length || stale.length || problems.length) process.exitCode = 1;
}

if (process.argv[1] === fileURLToPath(import.meta.url)) await main();
