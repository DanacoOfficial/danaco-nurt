// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           website.config.site
// Responsibility:   Identity of the site: names, product facts, repository addresses and the deployment address.
// Dependencies:     none.

export const deploymentModes = {
  subdomain: { url: 'https://nurt.danaco-group.pl', base: '/' },
  path: { url: 'https://www.danaco-group.pl', base: '/nurt/' },
} as const;

export type DeploymentMode = keyof typeof deploymentModes;

const requestedMode = process.env.SITE_MODE as DeploymentMode | undefined;
const mode: DeploymentMode = requestedMode && requestedMode in deploymentModes ? requestedMode : 'subdomain';
const rawBase = process.env.SITE_BASE ?? deploymentModes[mode].base;

export const site = {
  name: 'Danaco Nurt',
  programName: 'nurt',
  tagline: 'Command shell and language where every change can be undone',
  company: 'Danaco Holding Group Sp. z o.o.',
  creator: 'Dariusz Naharnowicz',
  companyUrl: 'https://www.danaco-group.pl',
  copyrightYear: 2026,
  license: 'Apache-2.0',
  licenseName: 'Apache License, Version 2.0',
  supportEmail: 'support@danaco-group.pl',
  securityEmail: 'support@danaco-group.pl',
  url: (process.env.SITE_URL ?? deploymentModes[mode].url).replace(/\/+$/, ''),
  base: rawBase.endsWith('/') ? rawBase : `${rawBase}/`,
  mode,
} as const;

export const product = {
  version: '0.1.2',
  releaseDate: '2026-11-28',
  stability: 'pre-stable',
  platforms: ['Linux', 'Windows', 'macOS'],
} as const;

// The address of the repository is the one of Cargo.toml. Links into files point at sourceRef, the revision the documentation of the site is imported from.
const repositoryUrl = 'https://github.com/DanacoOfficial/danaco-nurt';
const sourceRef = 'release-0.1.2';

export const repository = {
  owner: 'DanacoOfficial',
  name: 'danaco-nurt',
  url: repositoryUrl,
  sourceRef,
  blob: `${repositoryUrl}/blob/${sourceRef}`,
  tree: `${repositoryUrl}/tree/${sourceRef}`,
  releases: `${repositoryUrl}/releases`,
  issues: `${repositoryUrl}/issues`,
  organisation: 'https://github.com/DanacoOfficial',
} as const;

export const installCommands = {
  windows: 'irm https://nurt.danaco-group.pl/install.ps1 | iex',
  unix: 'curl -fsSL https://nurt.danaco-group.pl/install.sh | sh',
} as const;
