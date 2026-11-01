# Vendored libraries and fonts

The inbox's page runs on these files, served as they are from the binary
under `/static/vendor/`. Each is taken unmodified from its package's npm
tarball, after checking the tarball against the registry's `integrity`.
A test (`commands::serve::page::tests`) checks each file against the
SHA-256 below, so a file that changes fails the build until this table and
the test change with it. Each library's licence is beside it.

| File | Package | Path in the tarball | SHA-256 |
| --- | --- | --- | --- |
| `preact.module.js` | preact 10.29.8, MIT | `package/dist/preact.module.js` | `c30e721ebfdc6e2ad4c18c14d2dfb82667829c8aec27de1207774e3fc16858a8` |
| `hooks.module.js` | preact 10.29.8, MIT | `package/hooks/dist/hooks.module.js` | `a6ee626f2d01570592dd569a792e3f050154aa02890eead8c223fa3ed5aa3d5a` |
| `htm.module.js` | htm 3.1.1, Apache-2.0 | `package/dist/htm.module.js` | `ab33dd3f38059b9be4d5f5350128eefb2356639c4e0bbe9d9e8b3ba75847e9e4` |
| `marked.esm.js` | marked 18.0.14, MIT | `package/lib/marked.esm.js` | `528a1b88bef88fc27277e06036ce7f4afb6a220b18110ba57c09e292b24a7ce0` |
| `inter-latin-wght-normal.woff2` | @fontsource-variable/inter 5.3.0, OFL-1.1 | `package/files/inter-latin-wght-normal.woff2` | `3100e775e8616cd2611beecfa23a4263d7037586789b43f035236a2e6fbd4c62` |
| `jetbrains-mono-latin-wght-normal.woff2` | @fontsource-variable/jetbrains-mono 5.3.0, OFL-1.1 | `package/files/jetbrains-mono-latin-wght-normal.woff2` | `18be452724bfdc236c074ca94a249a7f41a86752c7d04ab258ce9ed5651f6a7e` |
| `mermaid/chunks/mermaid.esm.min/chunk-3KU6CYTB.mjs` | mermaid 12.2.0, MIT | `package/dist/chunks/mermaid.esm.min/chunk-3KU6CYTB.mjs` | `00edcfae1102c086d7d7c0f69e03bb9e66d341c2d7bbd4a60a60ec6127e40007` |
| `mermaid/chunks/mermaid.esm.min/chunk-4GHFHL4L.mjs` | mermaid 12.2.0, MIT | `package/dist/chunks/mermaid.esm.min/chunk-4GHFHL4L.mjs` | `396c335a6630a82ef77635b7014b65d6b3095e72da420e7946a6a5c88f6d3c7e` |
| `mermaid/chunks/mermaid.esm.min/chunk-674HQBI4.mjs` | mermaid 12.2.0, MIT | `package/dist/chunks/mermaid.esm.min/chunk-674HQBI4.mjs` | `78a14415742410cd85c530442d1fbc121550f42ae99e0618150e0920314f5ff3` |
| `mermaid/chunks/mermaid.esm.min/chunk-7P5HNUOW.mjs` | mermaid 12.2.0, MIT | `package/dist/chunks/mermaid.esm.min/chunk-7P5HNUOW.mjs` | `d0aa9958356deea20cadbccb3e2d1283ddbc955aa7ccf8a707f673f2dbda8d5e` |
| `mermaid/chunks/mermaid.esm.min/chunk-BOZ75642.mjs` | mermaid 12.2.0, MIT | `package/dist/chunks/mermaid.esm.min/chunk-BOZ75642.mjs` | `258c1f8a5e8fc1cc5edfd343779c9c50f0bf8932a6e7fa67aa2d174283b617fa` |
| `mermaid/chunks/mermaid.esm.min/chunk-D36F7FYG.mjs` | mermaid 12.2.0, MIT | `package/dist/chunks/mermaid.esm.min/chunk-D36F7FYG.mjs` | `6fa5cda64a6227731eb422b312aaff0070f0986097e8d94354ac6101a25c398b` |
| `mermaid/chunks/mermaid.esm.min/chunk-DXAG4NAJ.mjs` | mermaid 12.2.0, MIT | `package/dist/chunks/mermaid.esm.min/chunk-DXAG4NAJ.mjs` | `c907572175ed48fd9471e38ed333fc3b2edaadf482839998b2f57079d318902a` |
| `mermaid/chunks/mermaid.esm.min/chunk-F6PIGZHG.mjs` | mermaid 12.2.0, MIT | `package/dist/chunks/mermaid.esm.min/chunk-F6PIGZHG.mjs` | `7a600c611d09d8a05cd2e6f96f13901d9ba27576fc21c77e63668af1005cd163` |
| `mermaid/chunks/mermaid.esm.min/chunk-FNR2RGIF.mjs` | mermaid 12.2.0, MIT | `package/dist/chunks/mermaid.esm.min/chunk-FNR2RGIF.mjs` | `8d7c420d9526ab350be333dbf743e91354691a6481275e64f1db7549db4dba05` |
| `mermaid/chunks/mermaid.esm.min/chunk-FWWDAFD2.mjs` | mermaid 12.2.0, MIT | `package/dist/chunks/mermaid.esm.min/chunk-FWWDAFD2.mjs` | `6599aa9fff9fe86e90eb3cc860483ee0910e5fa029e6336895aeb38e5b58bbe8` |
| `mermaid/chunks/mermaid.esm.min/chunk-GDOATIN3.mjs` | mermaid 12.2.0, MIT | `package/dist/chunks/mermaid.esm.min/chunk-GDOATIN3.mjs` | `0613e167b74c7f000549b95541b971a1ec378792a71beed86196c60a1fce0700` |
| `mermaid/chunks/mermaid.esm.min/chunk-GHI2JMQX.mjs` | mermaid 12.2.0, MIT | `package/dist/chunks/mermaid.esm.min/chunk-GHI2JMQX.mjs` | `46d67e0fa2bf1579a00d9565db7e0cadcfe7ee3a4449aad4cec62d9cd52f3f28` |
| `mermaid/chunks/mermaid.esm.min/chunk-IER4RZHT.mjs` | mermaid 12.2.0, MIT | `package/dist/chunks/mermaid.esm.min/chunk-IER4RZHT.mjs` | `068959b1dfcb8d26bcbfd1e34ad922467d0590eae06cec62f29cb5a285f8748a` |
| `mermaid/chunks/mermaid.esm.min/chunk-KJ5T2YZX.mjs` | mermaid 12.2.0, MIT | `package/dist/chunks/mermaid.esm.min/chunk-KJ5T2YZX.mjs` | `88af19a0945746df13d4c25164c83d47cca25f24485412c982d70122b16a5283` |
| `mermaid/chunks/mermaid.esm.min/chunk-KTS5TP6K.mjs` | mermaid 12.2.0, MIT | `package/dist/chunks/mermaid.esm.min/chunk-KTS5TP6K.mjs` | `0ae23b8e2fb9297d068a8b1f745bc2b08a55320dfb0db90711863b7d74ba2f86` |
| `mermaid/chunks/mermaid.esm.min/chunk-L2B47AAZ.mjs` | mermaid 12.2.0, MIT | `package/dist/chunks/mermaid.esm.min/chunk-L2B47AAZ.mjs` | `57b417baaff19ed1a3383bf327d39247c0c4aa876dc433e09f708d2e5950b71a` |
| `mermaid/chunks/mermaid.esm.min/chunk-LGKD3V67.mjs` | mermaid 12.2.0, MIT | `package/dist/chunks/mermaid.esm.min/chunk-LGKD3V67.mjs` | `4bfc3262830f618555cfa3403a7773939d4ad11af2e36db796ac11f8ee726a92` |
| `mermaid/chunks/mermaid.esm.min/chunk-MFYRHTYG.mjs` | mermaid 12.2.0, MIT | `package/dist/chunks/mermaid.esm.min/chunk-MFYRHTYG.mjs` | `907abf3f3c54b58e7577c4bd6a3cfa18a9978110765843cd0d46748d6db4e748` |
| `mermaid/chunks/mermaid.esm.min/chunk-MIE3LDYP.mjs` | mermaid 12.2.0, MIT | `package/dist/chunks/mermaid.esm.min/chunk-MIE3LDYP.mjs` | `5d4f7c6773b99ffca185401cf164d111ab304d05709d933c10ce47a1e06d5188` |
| `mermaid/chunks/mermaid.esm.min/chunk-MMC4O45A.mjs` | mermaid 12.2.0, MIT | `package/dist/chunks/mermaid.esm.min/chunk-MMC4O45A.mjs` | `9a375b48e696b59371d6535c26cf4ce73a759911d6af71d0ca60ef4526c944c5` |
| `mermaid/chunks/mermaid.esm.min/chunk-QVIEFLGF.mjs` | mermaid 12.2.0, MIT | `package/dist/chunks/mermaid.esm.min/chunk-QVIEFLGF.mjs` | `5ad0557882607c7b100c33722714c62832a84feb26d79a7151fdbe4680f2165b` |
| `mermaid/chunks/mermaid.esm.min/chunk-SDIBIOXT.mjs` | mermaid 12.2.0, MIT | `package/dist/chunks/mermaid.esm.min/chunk-SDIBIOXT.mjs` | `c927a4a7d111b376cddf6c30a73bc2366a6fa1be145ca69abdead4c55d24fb4f` |
| `mermaid/chunks/mermaid.esm.min/chunk-XGFVAD2T.mjs` | mermaid 12.2.0, MIT | `package/dist/chunks/mermaid.esm.min/chunk-XGFVAD2T.mjs` | `e9826d66fac551f54b354d1499ad41b94136845c3d833ebdd9da1c597b077204` |
| `mermaid/chunks/mermaid.esm.min/chunk-XUG7IRDC.mjs` | mermaid 12.2.0, MIT | `package/dist/chunks/mermaid.esm.min/chunk-XUG7IRDC.mjs` | `1d906b46af61082cb43f6d3bc1a903b7108975c7a66606b6cf52d46d2670647e` |
| `mermaid/chunks/mermaid.esm.min/chunk-YX6CVSZ3.mjs` | mermaid 12.2.0, MIT | `package/dist/chunks/mermaid.esm.min/chunk-YX6CVSZ3.mjs` | `940039393be1f982148fd4dfc998b4512fba3ae556b7d75d86b3d831992e9315` |
| `mermaid/chunks/mermaid.esm.min/chunk-ZZ52EIBP.mjs` | mermaid 12.2.0, MIT | `package/dist/chunks/mermaid.esm.min/chunk-ZZ52EIBP.mjs` | `05866f9f6191141f83fd225d0ae8071873cdd24f94e8b99023cd3401ad4a12a8` |
| `mermaid/chunks/mermaid.esm.min/classDiagram-v2-Y5FV6ONJ.mjs` | mermaid 12.2.0, MIT | `package/dist/chunks/mermaid.esm.min/classDiagram-v2-Y5FV6ONJ.mjs` | `c36c276b44e7807b119af2434346d4a4a6e53846ed196dd819bfae8a717f922d` |
| `mermaid/chunks/mermaid.esm.min/dagre-JKPHJXCU.mjs` | mermaid 12.2.0, MIT | `package/dist/chunks/mermaid.esm.min/dagre-JKPHJXCU.mjs` | `c9dbd1a82b6d86ed7f7e4979c5f9ec94406c727be4a9928e99cc142deff1683c` |
| `mermaid/chunks/mermaid.esm.min/erDiagram-Y4LBXKHG.mjs` | mermaid 12.2.0, MIT | `package/dist/chunks/mermaid.esm.min/erDiagram-Y4LBXKHG.mjs` | `10d0cd14992ac9ae0f432399b43ee2802752b812a68438770ceb142dc367c0e0` |
| `mermaid/chunks/mermaid.esm.min/flowDiagram-KMDKLPNL.mjs` | mermaid 12.2.0, MIT | `package/dist/chunks/mermaid.esm.min/flowDiagram-KMDKLPNL.mjs` | `0130c02c9cbea5f095c5318affd4a840a17f35652beaa0ff1ff085532d0eef32` |
| `mermaid/chunks/mermaid.esm.min/sequenceDiagram-OV7DSCPC.mjs` | mermaid 12.2.0, MIT | `package/dist/chunks/mermaid.esm.min/sequenceDiagram-OV7DSCPC.mjs` | `7eb32f7cac87fe60c149f14824e4b31dbe6c0898f152dd30fcd382823e0d46ff` |
| `mermaid/chunks/mermaid.esm.min/stateDiagram-v2-PVXPCO6V.mjs` | mermaid 12.2.0, MIT | `package/dist/chunks/mermaid.esm.min/stateDiagram-v2-PVXPCO6V.mjs` | `fc40c5b8600e2cc3462be10f72aaa997bdda89568d5b3d82d84fc1cf3d19933c` |
| `mermaid/mermaid.esm.min.mjs` | mermaid 12.2.0, MIT | `package/dist/mermaid.esm.min.mjs` | `dae3440dd5a4adecf76cfd2951c7a69523df3237f110dd186bebbd55f6038c9d` |

The tarballs and their integrity, from `https://registry.npmjs.org/<name>/<version>`:

- https://registry.npmjs.org/preact/-/preact-10.29.8.tgz
  `sha512-ej2aVZ+vZ8WO7tvlQWRM9N63A0KzF9q4mWJfDUHgYaIofWY9hu74QdnQrjoPMmZi2/nZ5gN0bJCQF49xQqx09Q==`
- https://registry.npmjs.org/htm/-/htm-3.1.1.tgz
  `sha512-983Vyg8NwUE7JkZ6NmOqpCZ+sh1bKv2iYTlUkzlWmA5JD2acKoxd4KVxbMmxX/85mtfdnDmTFoNKcg5DGAvxNQ==`
- https://registry.npmjs.org/marked/-/marked-18.0.14.tgz
  `sha512-mBHK6FBHuBAlhgRe88w9F0O1AbwwXJUcQibUbC/QcdTbVGAD7aWza+xt3N6oT/jCZx3/OMeS+8rnuiHZcQ9s7A==`
- https://registry.npmjs.org/@fontsource-variable/inter/-/inter-5.3.0.tgz
  `sha512-OupL48va4JNofb97w6NYeF9S7W/kHNKM0Er8Dem5nqi4jeOLrVJDoE8tZEpnMJmtkvNbB1EIPPwHcdkF6b1oUA==`
- https://registry.npmjs.org/@fontsource-variable/jetbrains-mono/-/jetbrains-mono-5.3.0.tgz
  `sha512-F32xpS2NsGYoQi2ADSkKTgpJj7ozajsGgDJ8woTnqjmIB+dxDIqImjl4pXZVEExu8UFZ2ndhmX18EBS/hdz3Lw==`
- https://registry.npmjs.org/mermaid/-/mermaid-12.2.0.tgz
  `sha512-w7nr/SFrLO7BaVTs3dQTvYSV6nYSBj0Yy9DjBhsIJtAcHqRzXSq4Pg32ujHZ/AdWsAMNEvvhbm5dt9E2tOiHzw==`

The fonts are for the keel look: Inter for the chrome and prose, JetBrains
Mono for ids and code. Each is one variable woff2, weights 100 to 900 (Inter)
and 100 to 800 (JetBrains Mono), cut by Fontsource to the Latin range that
the `@font-face` rules in `style.css` name as their `unicode-range`; a
character outside it is drawn in the system face that follows in the stack.
Inter is 48,256 bytes and JetBrains Mono 40,404. There is no italic: the
browser slants the upright face. Each font's licence, the SIL Open Font
License 1.1 with its copyright line, is the `LICENSE` from its tarball,
beside it as `<font>.LICENSE.txt`.

To take a new release: download its tarball with curl, check it against
the registry's `integrity` (`openssl dgst -sha512 -binary <tgz> | base64`),
copy the file out unchanged, and update this table and the test. The
page's modules are type-checked against the same releases:
`web/package.json` pins each at the version above, for its types, and a
test checks that the two agree. Update its pin with `npm install
--ignore-scripts --save-exact <name>@<version>` in `web/`, and check that
the lockfile's `integrity` for it is the one above.

`hooks.module.js` imports `preact` by its bare name, which the page's import
map resolves to `/static/vendor/preact.module.js`. `preact.module.js`,
`hooks.module.js` and `marked.esm.js` end with a `sourceMappingURL` comment;
the maps are not vendored, so a browser's developer tools find none.

`mermaid/` is part of mermaid's minified ES module build, the files under
`package/dist/` at the same paths, which import each other by relative
path: `mermaid.esm.min.mjs`, what it imports as it loads, and what five
kinds of diagram import when one is drawn: flowchart, sequence, class,
state and ER, with the dagre layout they use. That is 33 files, 1,126,745
bytes of the release's 5.4 MB. Every other kind of diagram, the other
layouts (elk alone is 1.6 MB) and KaTeX, for maths in labels, are left out;
a diagram that needs one fails to import it, and the page shows its text
as a code block. To take a new release, find the files again by following
the imports from `mermaid.esm.min.mjs` and from those five kinds'
`<kind>Diagram-*.mjs` and `dagre-*.mjs` (their names carry a hash, which
changes with each release), and replace the directory whole. A test checks
that every file one imports as it loads is served.

The files in `mermaid/` are committed as they are in the tarball, and the
table's hashes are theirs. build.rs gzips them into the binary, and serve
sends them gzipped, with `Content-Encoding: gzip`, to a request that takes
gzip, as every browser's does; it unzips them for one that does not. The
test checks each file as it unzips. mermaid's licence, MIT, is the
`LICENSE` from its tarball, beside it as `mermaid.LICENSE.txt`. The
minified build bundles code from its dependencies, under their own
licences: most are MIT, d3 is ISC, DOMPurify is MPL-2.0 or Apache-2.0, and
chevrotain is Apache-2.0 (elkjs, EPL-2.0, is in the elk file, which is
left out). The files keep only the banners esbuild keeps, such as
DOMPurify's. `web/package.json` also pins
`type-fest`, which mermaid's types import and its package does not install.

The bundled code is at the versions mermaid was built with, not the ones
npm resolves in `web/package-lock.json`: mermaid 12.2.0 bundles DOMPurify
3.4.12 and lodash-es 4.18.1. `web/vendor-inventory.json` lists each package
a file's banner names, with the text in that file that gives its version.
`web/advisories.js` (CI's `advisories` job) checks that inventory against
the files, and checks the registry's advisories against the releases in
the table above, the bundled packages and the lockfile; an advisory that
does not apply is excepted, with its reason, in
`web/advisory-exceptions.json`. To take a new mermaid release, update the
inventory with the files, and run `node advisories.js` in `web/`: an
exception that no longer matches an advisory fails, and is removed.
