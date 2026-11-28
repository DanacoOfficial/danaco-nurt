// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           website.src.data.facts
// Responsibility:   The numbers the site quotes: counted from the repository at build time or measured on the release candidate of 0.1.1, each with its source and, for measurements, the date and the revision. Pages use the Fact component, so that a number is written in one place only.
// Dependencies:     node:fs, website.config.site

import { existsSync, readdirSync } from 'node:fs';
import path from 'node:path';
import type { Locale } from '../../config/i18n';

export type FactId =
  | 'commands'
  | 'coreCommands'
  | 'optionalGroups'
  | 'crates'
  | 'specFiles'
  | 'tests'
  | 'gitComparisons'
  | 'windowsTests'
  | 'scenarios'
  | 'platforms'
  | 'license';

export interface Fact {
  readonly value: string;
  readonly label: Record<Locale, string>;
  readonly source: Record<Locale, string>;
  readonly href?: string;
  readonly kind: 'counted' | 'measured' | 'stated';
}

// The build runs in the website directory, so the repository is its parent; a module URL points into the bundle and cannot be used.
const repositoryRoot = path.resolve(process.cwd(), '..');

function countEntries(directory: string, predicate: (name: string) => boolean): number {
  const location = path.join(repositoryRoot, directory);
  if (!existsSync(location)) {
    throw new Error(`facts: directory not found: ${directory} (the build must run in the website directory)`);
  }
  return readdirSync(location).filter(predicate).length;
}

const commandPages = countEntries('docs/commands', (name) => name.endsWith('.md') && name !== 'README.md' && name !== '_template.md');
const crateDirectories = countEntries('crates', (name) => !name.startsWith('.'));
const specificationFiles = countEntries('spec', (name) => name.endsWith('.md'));

const format = (value: number): string => value.toLocaleString('en-GB');

export const facts: Record<FactId, Fact> = {
  commands: {
    value: String(commandPages),
    kind: 'counted',
    label: { en: 'commands in the catalog', pl: 'poleceń w katalogu' },
    source: { en: 'one page per command in docs/commands', pl: 'jedna strona na polecenie w docs/commands' },
    href: '/docs/commands/',
  },
  coreCommands: {
    value: '34',
    kind: 'stated',
    label: { en: 'core commands', pl: 'poleceń rdzenia' },
    source: { en: 'documentation index, docs/site/index.md', pl: 'indeks dokumentacji, docs/site/index.md' },
    href: '/docs/guide/',
  },
  optionalGroups: {
    value: '14',
    kind: 'stated',
    label: { en: 'optional command groups', pl: 'opcjonalnych grup poleceń' },
    source: { en: 'documentation index, docs/site/index.md', pl: 'indeks dokumentacji, docs/site/index.md' },
    href: '/docs/guide/',
  },
  crates: {
    value: String(crateDirectories),
    kind: 'counted',
    label: { en: 'crates in the workspace', pl: 'skrzynek w przestrzeni roboczej' },
    source: { en: 'directories of crates/', pl: 'katalogi w crates/' },
    href: '/architecture/crates/',
  },
  specFiles: {
    value: String(specificationFiles),
    kind: 'counted',
    label: { en: 'specification documents', pl: 'dokumentów specyfikacji' },
    source: { en: 'files of spec/', pl: 'pliki w spec/' },
    href: '/docs/specification/',
  },
  tests: {
    value: format(13965),
    kind: 'measured',
    label: { en: 'tests passed in the workspace run', pl: 'testów zaliczonych w przebiegu przestrzeni roboczej' },
    source: { en: 'release candidate of 0.1.1, full run on Linux, 27 November 2026', pl: 'kandydat na wydanie 0.1.1, pełny przebieg na Linuksie, 27 listopada 2026' },
    href: '/quality/testing/',
  },
  gitComparisons: {
    value: '775 / 775',
    kind: 'measured',
    label: { en: 'comparisons with git agree', pl: 'porównań z gitem zgodnych' },
    source: { en: 'nurt-git-difftest, release candidate of 0.1.1', pl: 'nurt-git-difftest, kandydat na wydanie 0.1.1' },
    href: '/quality/conformance/',
  },
  windowsTests: {
    value: format(13455),
    kind: 'measured',
    label: { en: 'tests passed on Windows 11, none failed', pl: 'testów zaliczonych na Windows 11, żaden niezaliczony' },
    source: { en: 'full set without ignored tests, release candidate of 0.1.1', pl: 'pełny zestaw bez testów ignorowanych, kandydat na wydanie 0.1.1' },
    href: '/quality/testing/',
  },
  scenarios: {
    value: '19',
    kind: 'measured',
    label: { en: 'end-to-end scenarios passed on a clean system', pl: 'scenariuszy end-to-end zaliczonych na czystym systemie' },
    source: { en: 'archives of the release, 3 more need Windows', pl: 'archiwa wydania, 3 kolejne wymagają Windows' },
    href: '/quality/testing/',
  },
  platforms: {
    value: '3',
    kind: 'stated',
    label: { en: 'operating systems, one executable each', pl: 'systemy operacyjne, po jednym pliku wykonywalnym' },
    source: { en: 'Linux, Windows and macOS', pl: 'Linux, Windows i macOS' },
    href: '/product/platforms/',
  },
  license: {
    value: 'Apache-2.0',
    kind: 'stated',
    label: { en: 'license of the code', pl: 'licencja kodu' },
    source: { en: 'LICENSE and NOTICE in the repository', pl: 'LICENSE i NOTICE w repozytorium' },
    href: '/project/license/',
  },
};
