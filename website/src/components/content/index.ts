// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           website.src.components.content.index
// Responsibility:   The components that authors of MDX pages can use without importing them; the page templates pass this map to the MDX content.
// Dependencies:     none.

import Badge from './Badge.astro';
import BrandImage from './BrandImage.astro';
import Callout from './Callout.astro';
import Card from './Card.astro';
import CardGrid from './CardGrid.astro';
import Fact from './Fact.astro';
import Facts from './Facts.astro';
import Stat from './Stat.astro';
import Stats from './Stats.astro';
import Step from './Step.astro';
import Steps from './Steps.astro';
import Terminal from './Terminal.astro';
import Timeline from './Timeline.astro';

import CommandCatalog from '../docs/CommandCatalog.astro';
import Downloads from './Downloads.astro';

export const mdxComponents = {
  Badge,
  CommandCatalog,
  Downloads,
  BrandImage,
  Callout,
  Card,
  CardGrid,
  Fact,
  Facts,
  Stat,
  Stats,
  Step,
  Steps,
  Terminal,
  Timeline,
};
