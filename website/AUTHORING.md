<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# Authoring the pages of the site

The authored pages of the website are MDX files in `src/content/pages`. English is the source language; every English page has a Polish translation with the same path. The documentation (`/docs/`) is not authored here: it is imported from `docs/` and `spec/` of the repository when the site is built.

## 1. Files

| Path | Content |
|---|---|
| `src/content/pages/en/<section>/<slug>.mdx` | the English page |
| `src/content/pages/pl/<section>/<slug>.mdx` | the Polish translation of the same page |
| `config/sections.ts` | the sections and the order of the pages in each section; a page that is not listed there is not part of the navigation |

The sections are `product`, `quality`, `architecture`, `project` and `legal`. The address of a page is `/<section>/<slug>/` in English and `/pl/<section>/<slug>/` in Polish.

## 2. Front matter

```yaml
---
title: Testing
description: One or two sentences, 40 to 220 characters, used in search results and in the cards that link to the page.
section: quality
order: 2
eyebrow: Quality
lead: One sentence under the title of the page.
updated: 2026-11-28
toc: true
related: [quality/conformance, quality/performance]
keywords: [tests, nextest]
---
```

| Field | Rule |
|---|---|
| `title` | short noun phrase; the same words in the navigation, the breadcrumbs and the footer |
| `description` | 40 to 220 characters |
| `section` | one of the section keys |
| `order` | integer; equal in both languages |
| `eyebrow`, `lead` | optional; `lead` defaults to `description` |
| `updated` | date of the last change of the content |
| `toc` | `true` by default; the table of contents shows when the page has three or more headings of the second or third level |
| `related` | slugs written as `<section>/<slug>`; equal in both languages |

After the front matter every file starts with the three-line Danaco header as MDX comments, exactly as in `src/content/pages/en/product/overview.mdx`.

## 3. Writing rules

1. Write in a formal, factual register. A page states what the product is, what it does and how it is used. Every sentence is a positive statement about the product: a function, a behaviour, a result, a way of use.
2. Every statement of fact comes from the repository: `README.md`, `CHANGELOG.md`, `docs/`, `spec/`, `catalog/`, `crates/`, the workflows in `.github/workflows` and the files in `bench/`, `conformance/` and `packaging/`. A statement that has no source in the repository is left out; it is not replaced by a remark that the source is missing.
3. Numbers come from `src/data/facts.ts` through the `<Fact id="..." />` component. A number written in the text carries a short label of what was measured (the version, the platform, the date) and nothing more.
4. A page does not explain itself and does not argue. It does not say where its statements come from, how the site was written or checked, what the site or the product lacks, what is unfinished, unpublished, unmeasured or unverified, and it does not comment on disagreements between source documents. Lists of limitations, open points, caveats and disclaimers are not written. Where two sources disagree, the narrower statement is written without comment. The compatibility policy on `project/versioning` is written as the rules that hold, in the words of the repository.
5. Code examples are real programs of the language in fences marked `nurt`, or shell commands in fences marked `sh` or `powershell`. Examples come from `docs/`, `spec/` or `README.md`; a new example is checked against the command pages in `docs/commands`.
6. Do not copy a long passage of the documentation: summarise it in two or three sentences and link to the documentation page.
7. Names: the product is "Danaco Nurt", the program is `nurt`, the company is "Danaco Holding Group Sp. z o.o.".

## 4. Links

Internal links are root-relative and carry no deployment base: `/quality/testing/` in English and `/pl/quality/testing/` in Polish. The build adds the base. Links into the documentation are `/docs/<section>/<page>/` in both languages, for example `/docs/guide/installation/` and `/docs/commands/replace/`. Links to the repository are full GitHub addresses.

## 5. Components

The components below are available in every page without an import.

| Component | Use |
|---|---|
| `<Callout type="note|tip|warning|important" title="...">` | a highlighted remark |
| `<Terminal title="...">` with a code fence inside | a terminal window around a code block |
| `<CommandCatalog level={3} />` (with `locale="pl"` on a Polish page) | the index of every command with its summary, read from the command pages of the repository |
| `<Downloads group="identity" />` or `<Downloads group="training" />` (with `locale="pl"` on a Polish page) | the downloads of the group with their sizes, licenses and SHA-256 sums, from `config/downloads.json` |
| `<CardGrid columns={2|3|4}>` with `<Card title icon href eyebrow>` | linked cards; `icon` is a name from `config/icons.ts` |
| `<Stats>` with `<Fact id="..." />` or `<Stat value label source href />` | a row of figures |
| `<Steps>` with `<Step title="...">` | a numbered procedure |
| `<Facts items={[{ term, value, href }]} />` | a list of terms and values |
| `<Timeline items={[{ label, title, text, href, current }]} />` | a sequence of releases or events |
| `<Badge tone="neutral|accent|success|warning|danger">` | a short status label |
| `<BrandImage name alt width caption />` | a render or a mockup of the identity; `name` is a file in `src/assets/brand` without the extension |

Markdown tables, lists, fences and block quotes are styled by the site. Use a table for data with more than two columns and a list otherwise.

## 6. Translation

The Polish page is a translation, not a rewrite: the same headings in the same order, the same code fences, the same components and the same links with the `/pl/` prefix. Rules of the Polish text:

- full Polish orthography with all diacritical marks; formal register, second person singular as in the product documentation only where the English text addresses the reader;
- product names, command names, field names, code and file paths are not translated;
- established terms: "dziennik" (journal), "cofanie" (undo), "raport skutków" (effect report), "katalog poleceń" (command catalog), "potok" (pipeline), "rekord" (record), "etap" (stage), "skrzynka" (crate), "wydanie" (release);
- the `description`, `title`, `eyebrow` and `lead` are translated; `order`, `section`, `related` and `updated` are copied.

## 7. Checks

```sh
npm run check:i18n      # the Polish pages are a translation of the English pages
npm run check:content   # the front matter, the sections map and the facts used
npm run check:links     # the internal links of the built site (after npm run build)
```

A change is complete when the three checks pass and `npm run build` ends with code 0.
