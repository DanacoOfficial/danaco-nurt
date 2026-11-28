// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           website.config.docs
// Responsibility:   Import of the documentation: which directories of the repository become which sections of the site, and the files left out.
// Dependencies:     none.

export type DocsRoot = 'docs' | 'spec';

export interface DocsSource {
  readonly root: DocsRoot;
  readonly directory: string;
  readonly section: string;
  readonly labelKey: string;
  readonly order: number;
  readonly sectionIndex?: string;
}

export const docsRoots: Record<DocsRoot, string> = {
  docs: '../docs',
  spec: '../spec',
};

export const docsSources: readonly DocsSource[] = [
  { root: 'docs', directory: 'site', section: 'guide', labelKey: 'guide', order: 1, sectionIndex: 'index' },
  { root: 'docs', directory: 'guides', section: 'guides', labelKey: 'guides', order: 2 },
  { root: 'docs', directory: 'cookbook', section: 'cookbook', labelKey: 'cookbook', order: 3, sectionIndex: 'README' },
  { root: 'docs', directory: 'commands', section: 'commands', labelKey: 'commands', order: 4 },
  { root: 'docs', directory: 'groups', section: 'groups', labelKey: 'groups', order: 5 },
  { root: 'docs', directory: 'cheatsheets', section: 'cheatsheets', labelKey: 'cheatsheets', order: 6 },
  { root: 'docs', directory: 'reference', section: 'reference', labelKey: 'reference', order: 7 },
  { root: 'docs', directory: 'integrators', section: 'integrators', labelKey: 'integrators', order: 8, sectionIndex: 'README' },
  { root: 'docs', directory: 'architecture', section: 'architecture', labelKey: 'architectureDocs', order: 9 },
  { root: 'docs', directory: 'cli', section: 'cli', labelKey: 'cli', order: 10 },
  { root: 'docs', directory: 'providers', section: 'providers', labelKey: 'providers', order: 11 },
  { root: 'docs', directory: 'conventions', section: 'conventions', labelKey: 'conventions', order: 12 },
  { root: 'spec', directory: '', section: 'specification', labelKey: 'specification', order: 13 },
];

export const docsExcludedFiles: readonly string[] = ['_template.md'];

export const docsPathPrefix = '/docs';
