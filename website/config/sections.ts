// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           website.config.sections
// Responsibility:   Map of the authored pages: the sections of the site and the ordered list of page slugs in each of them.
// Dependencies:     none.

export const sectionKeys = ['product', 'ai', 'quality', 'architecture', 'design', 'project', 'legal'] as const;
export type SectionKey = (typeof sectionKeys)[number];

export const sectionPages: Record<SectionKey, readonly string[]> = {
  product: [
    'overview',
    'features',
    'journal-and-undo',
    'language',
    'commands',
    'effect-reports',
    'ai-agents',
    'use-cases',
    'migration',
    'platforms',
    'integrations',
  ],
  ai: ['overview', 'model-labs', 'agent-engines', 'training-packages', 'benchmark'],
  quality: [
    'overview',
    'testing',
    'conformance',
    'performance',
    'reliability',
    'security',
    'supply-chain',
    'engineering-standards',
  ],
  architecture: ['overview', 'engine', 'crates', 'c-interface', 'protocols', 'dependencies'],
  design: ['overview', 'mark-and-colour', 'identity'],
  project: ['releases', 'status', 'versioning', 'contributing', 'license', 'about', 'support'],
  legal: ['privacy', 'accessibility'],
};

export function pageId(section: SectionKey, slug: string): string {
  return `${section}/${slug}`;
}

export function allPageIds(): string[] {
  return sectionKeys.flatMap((section) => sectionPages[section].map((slug) => pageId(section, slug)));
}
