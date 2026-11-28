// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           website.src.content.config
// Responsibility:   Content collections: the authored pages in two languages, the imported documentation and the translations of the imported documentation.
// Dependencies:     astro:content, astro/loaders, website.config.sections, website.src.lib.docs.loader

import { defineCollection, z } from 'astro:content';
import { glob } from 'astro/loaders';
import { sectionKeys } from '../config/sections';
import { docsLoader } from './lib/docs/loader';

const pages = defineCollection({
  loader: glob({ base: './src/content/pages', pattern: '**/*.mdx' }),
  schema: z.object({
    title: z.string().min(3),
    description: z.string().min(40).max(220),
    section: z.enum(sectionKeys),
    order: z.number().int().min(1),
    eyebrow: z.string().optional(),
    lead: z.string().optional(),
    updated: z.coerce.date().optional(),
    toc: z.boolean().default(true),
    related: z.array(z.string()).default([]),
    keywords: z.array(z.string()).default([]),
  }),
});

const docs = defineCollection({
  loader: docsLoader(),
  schema: z.object({
    title: z.string(),
    description: z.string(),
    section: z.string(),
    sectionOrder: z.number(),
    order: z.number().optional(),
    repositoryPath: z.string(),
    stem: z.string(),
    commands: z.array(z.string()).optional(),
    summary: z.string().optional(),
    group: z.string().optional(),
    since: z.string().optional(),
  }),
});

const docsTranslations = defineCollection({
  loader: glob({ base: './src/content/docs-translations', pattern: '**/*.md' }),
  schema: z.object({
    title: z.string(),
    description: z.string(),
    source: z.string(),
    sourceDigest: z.string().optional(),
  }),
});

export const collections = { pages, docs, docsTranslations };
