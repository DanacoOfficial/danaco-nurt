// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           website.config.navigation
// Responsibility:   Navigation of the site: the main menu, the footer columns and the quick links, as keys of the interface dictionary and paths.
// Dependencies:     website.config.sections

import type { SectionKey } from './sections';

export interface NavigationItem {
  readonly key: string;
  readonly path: string;
}

export const mainNavigation: readonly NavigationItem[] = [
  { key: 'product', path: '/product/' },
  { key: 'ai', path: '/ai/' },
  { key: 'quality', path: '/quality/' },
  { key: 'architecture', path: '/architecture/' },
  { key: 'design', path: '/design/' },
  { key: 'docs', path: '/docs/' },
  { key: 'project', path: '/project/' },
];

export interface FooterColumn {
  readonly key: string;
  readonly section: SectionKey | 'docs';
  readonly pages: readonly string[];
}

export const footerColumns: readonly FooterColumn[] = [
  { key: 'product', section: 'product', pages: ['overview', 'features', 'journal-and-undo', 'language', 'ai-agents', 'platforms'] },
  { key: 'ai', section: 'ai', pages: ['overview', 'model-labs', 'agent-engines', 'training-packages', 'benchmark'] },
  { key: 'quality', section: 'quality', pages: ['overview', 'testing', 'performance', 'reliability', 'security', 'supply-chain'] },
  { key: 'architecture', section: 'architecture', pages: ['overview', 'engine', 'crates', 'c-interface', 'protocols'] },
  { key: 'design', section: 'design', pages: ['overview', 'mark-and-colour', 'identity'] },
  { key: 'project', section: 'project', pages: ['releases', 'status', 'contributing', 'license', 'support'] },
];

export const legalLinks: readonly NavigationItem[] = [
  { key: 'privacy', path: '/legal/privacy/' },
  { key: 'accessibility', path: '/legal/accessibility/' },
  { key: 'search', path: '/search/' },
];

export const quickStart: readonly NavigationItem[] = [
  { key: 'installation', path: '/docs/guide/installation/' },
  { key: 'firstSteps', path: '/docs/guide/first-steps/' },
  { key: 'cookbook', path: '/docs/cookbook/' },
];
