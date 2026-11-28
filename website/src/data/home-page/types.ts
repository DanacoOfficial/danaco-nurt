// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           website.src.data.home-page.types
// Responsibility:   Shape of the content of the home page; the English and the Polish content have this shape.
// Dependencies:     website.config.icons

import type { IconName } from '../../../config/icons';

export interface HomeLink {
  readonly label: string;
  readonly path: string;
}

export interface HomeCard {
  readonly icon: IconName;
  readonly title: string;
  readonly text: string;
  readonly path: string;
}

export interface HomeContent {
  readonly hero: { readonly eyebrow: string; readonly title: string; readonly lead: string; readonly note: string; readonly sceneLabel: string; readonly sceneCaption: string };
  readonly pillars: { readonly eyebrow: string; readonly title: string; readonly lead: string; readonly cards: readonly HomeCard[] };
  readonly statement: { readonly eyebrow: string; readonly quote: string; readonly role: string };
  readonly how: {
    readonly eyebrow: string;
    readonly title: string;
    readonly lead: string;
    readonly programTitle: string;
    readonly program: string;
    readonly sessionTitle: string;
    readonly sessionReplay: string;
    readonly sessionCaption: string;
    readonly points: readonly { readonly title: string; readonly text: string }[];
  };
  readonly numbers: { readonly eyebrow: string; readonly title: string; readonly lead: string };
  readonly ai: { readonly eyebrow: string; readonly title: string; readonly lead: string; readonly cards: readonly HomeCard[]; readonly action: string };
  readonly design: { readonly eyebrow: string; readonly title: string; readonly lead: string; readonly cards: readonly HomeCard[] };
  readonly quality: { readonly eyebrow: string; readonly title: string; readonly lead: string; readonly cards: readonly HomeCard[] };
  readonly docs: { readonly eyebrow: string; readonly title: string; readonly lead: string; readonly cards: readonly HomeCard[] };
  readonly cta: { readonly title: string; readonly text: string; readonly primary: string; readonly secondary: string };
}
