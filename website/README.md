<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# The website of Danaco Nurt

The directory `website/` holds the public website of Danaco Nurt: the product, AI, quality, architecture, design and project pages in English and Polish, the documentation of the repository imported at build time, the identity files and the training packages to download. The site is static; the build writes plain files to `website/dist/`, which any web server can publish.

The site is written from the ground up with Astro 7 and TypeScript. It uses no page template and no component kit. Third-party parts are libraries and assets only: Astro and its MDX and sitemap integrations, the Sätteri Markdown processor, Shiki for code highlighting, three.js for the three-dimensional mark, fflate for the download archives, Pagefind for the search index, the typefaces Geist and Geist Mono, and the Lucide icon set.

## 1. Commands

All commands run in the directory `website/`. Node.js 22.12 or later is required; the search index needs the program `pagefind` 1.5 on the `PATH`.

| Command | Effect |
|---|---|
| `npm ci` | installs the dependencies recorded in `package-lock.json` |
| `npm run dev` | serves the site with reloading at `http://localhost:4321/` |
| `npm run build` | packs the downloads, builds the site into `dist/` and writes the search index into `dist/pagefind/` |
| `npm run build:site` | packs the downloads and builds the site without the search index |
| `npm run downloads` | packs the downloads into `public/downloads/` and writes their manifest |
| `npm run training -- --nurt <nurt> --nurt-bench <nurt-bench>` | rebuilds the training packages in `training/` from the repository |
| `npm run build:search` | writes the search index of an existing `dist/` |
| `npm run preview` | serves `dist/` as built |
| `npm run check` | runs the three checks below, in order |
| `npm run check:i18n` | every English page has a Polish page with the same structure |
| `npm run check:content` | every page of `config/sections.ts` exists in both languages and keeps the page contract |
| `npm run check:links` | every internal link and resource of `dist/` resolves, including fragments |

The build must run in `website/`: the import of the documentation and the counted numbers read the parent directory as the repository.

## 2. Deployment address

One build serves one address. The address is chosen by environment variables read in `config/site.ts`; no file has to be edited.

| Variable | Values | Default | Meaning |
|---|---|---|---|
| `SITE_MODE` | `subdomain`, `path` | `subdomain` | `subdomain` builds for `https://nurt.danaco-group.pl/`; `path` builds for `https://www.danaco-group.pl/nurt/` |
| `SITE_URL` | an origin | from the mode | overrides the origin, for example for a staging host |
| `SITE_BASE` | a path that starts and ends with `/` | from the mode | overrides the path under the origin |

```sh
SITE_MODE=subdomain npm run build
SITE_MODE=path npm run build
SITE_MODE=path SITE_BASE=/nurt/ npm run check:links
```

Every internal address of the site passes through `withBase` in `src/lib/urls.ts`, and links written in Markdown and MDX receive the base from `src/lib/markdown/hast-base.ts`. `check:links` reads `SITE_BASE`, so a build for a path is checked with the same value.

Mounting the site means publishing the content of `dist/` at the chosen address. For the subdomain, `dist/` is the document root of `nurt.danaco-group.pl`. For the path, `dist/` is served under `/nurt/` of `www.danaco-group.pl`. The server must serve `index.html` for a directory address, serve `404.html` with status 404 for a missing address, and may cache everything under `_assets/` without limit, because those file names carry a content hash.

## 3. Structure

| Path | Content |
|---|---|
| `astro.config.ts` | the build: integrations, Markdown processing, code highlighting with the Nurt grammar of `editors/textmate` |
| `config/site.ts` | the deployment address, the product version and date, the repository address and revision |
| `config/i18n.ts` | the languages and their address prefixes |
| `config/navigation.ts` | the header navigation, the footer columns, the legal links |
| `config/sections.ts` | the authored pages of each section, in order |
| `config/docs.ts` | which directories of the repository become which documentation sections |
| `config/downloads.json` | the downloads of the site: the identity files and the training packages, with their titles in both languages |
| `config/seo.ts`, `config/search.ts`, `config/theme.ts`, `config/icons.ts` | metadata defaults, search limits, colour themes, the icons in use |
| `src/content/pages/<locale>/<section>/<slug>.mdx` | the authored pages; `AUTHORING.md` is their contract |
| `src/content.config.ts` | the content collections and their schemas |
| `src/lib/docs/` | the import of the documentation: front matter, link rewriting, the loader, the navigation model |
| `src/lib/urls.ts`, `src/lib/pages.ts` | addresses and page lookups |
| `src/i18n/` | the interface strings of both languages |
| `src/data/facts.ts` | every number the site quotes, with its source; pages use `<Fact id="…" />` |
| `src/data/home-page/`, `src/data/session.ts` | the content of the home page and the recorded terminal session |
| `src/components/base`, `brand`, `layout`, `content`, `sections`, `docs`, `motion`, `pages` | the components, one file each, grouped by role |
| `src/layouts/` | the document, the page and the documentation layouts |
| `src/styles/` | the foundation: tokens, fonts, reset, base, typography, layout, utilities, motion, print |
| `src/styles/components/` | one style file for each component |
| `src/scripts/` | the browser scripts: theme, menu, table of contents, code copy, search |
| `src/scripts/motion/` | the motion layer: entrances, counters, tilting cards, the terminal session, the WebGL mark |
| `src/pages/` | the routes; each file only selects a page component |
| `training/` | the training packages: pairs, chat conversations, tool calls, tool definitions and the documentation corpus, with `manifest.json` |
| `scripts/` | build and check tools |
| `public/` | files published as they are: icons, the social card, the downloads of the brand |

## 4. Documentation import

The pages under `/docs/` are not copied into this directory. `src/lib/docs/loader.ts` reads `docs/` and `spec/` of the repository at build time, strips the file headers, rewrites relative links to site addresses or to the repository, and gives every page a link to its source file. The site and the repository therefore cannot disagree. The documentation is English only; the Polish interface links to the English pages.

Links into the repository point at the revision named by `sourceRef` in `config/site.ts`. Set it to the tag of the release when the release is published.

## 5. Downloads and training packages

`scripts/build-downloads.mjs` runs before every site build. It packs the directories named in `config/downloads.json` into ZIP archives in `public/downloads/` and writes `public/downloads/manifest.json` with the size, the number of files and the SHA-256 sum of every download. The identity archives are packed from `branding/` of the repository and the training archives from `training/`; the archives are not committed. The entries of an archive are sorted and carry the date of `archiveDate`, so the same sources give the same archive. Pages show a group of downloads with `<Downloads group="identity" />` or `<Downloads group="training" />`.

The training packages in `training/` are committed. `scripts/build_training_packs.py` rebuilds them with the executables `nurt` and `nurt-bench` of one release: it takes the candidates of `nurt-bench pairs`, sends every program to the method `check` of `nurt serve`, and writes the accepted pairs, their chat and tool-call forms, the tool definitions of every format and the documentation corpus. `training/README.md` describes the formats.

## 6. Numbers

`src/data/facts.ts` holds three kinds of numbers. Counted numbers (commands, crates, specification documents, benchmark tasks, training pairs, tool definition formats, integration examples) are read from the repository by every build. Stated numbers quote a document of the repository. Measured numbers quote a recorded run and carry its release, platform and date in their source line; they change only when a new run is recorded.

## 7. Motion

The motion layer is optional for the reader. A script in the head sets the class `motion` on the document only when the reader has not asked for reduced motion; every animation rule in `src/styles/motion.css` applies under that class. Without scripts, without WebGL or with reduced motion the site shows the same content at rest: the studio render replaces the live mark, and the terminal session is shown complete.

The three-dimensional mark (`src/scripts/motion/mark-scene.ts`) is built from the path data of the identity, loads only on pages that contain it, and renders only while it is in view and the tab is visible.

## 8. Regenerating derived files

| File | Command | Source |
|---|---|---|
| `src/assets/fonts/*.woff2` | `FONT_SOURCE=<directory of the Geist fonts> npm run fonts` | the variable fonts Geist and Geist Mono, subset to the Latin ranges of English and Polish |
| `src/components/base/icon-data.ts` | `npm run icons -- <lucide.json>` | the Lucide collection of Iconify; the names come from `config/icons.ts` |
| `src/components/brand/brand-data.ts` | `python3 scripts/build_brand_data.py <identity pack>` | the mark and lockups of the production identity 1.0.0 |
| `public/brand/`, `src/assets/brand/` | `python3 scripts/sync_brand_assets.py <identity pack> public src/assets` | the downloads, renders and mockups of the identity |

## 9. Licences of third-party parts

| Part | Licence |
|---|---|
| Astro, `@astrojs/mdx`, `@astrojs/sitemap`, `@astrojs/markdown-satteri`, Sätteri, Shiki | MIT |
| three.js | MIT |
| fflate | MIT |
| sharp | Apache-2.0 |
| yaml | ISC |
| Pagefind | MIT |
| Geist, Geist Mono | SIL Open Font License 1.1 |
| Lucide | ISC |
