// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           website.src.data.section-pages.types
// Responsibility:   The shape of the content of the landing pages of the sections: the shared parts of a block and the content of each of the six pages.
// Dependencies:     website.config.icons

import type { IconName } from '../../../config/icons';

/** The heading of a block: the eyebrow, the title and an optional lead. */
export interface Heading {
  readonly eyebrow: string;
  readonly title: string;
  readonly lead?: string;
}

/** A numbered point: a title and one sentence. */
export interface Point {
  readonly title: string;
  readonly text: string;
}

/** A program shown in a terminal window with the sentence that explains it. */
export interface Program {
  readonly title: string;
  readonly code: string;
  readonly caption: string;
}

/** A framed cell of a map: an icon, a name, text in the code face and a note. */
export interface Cell {
  readonly icon: IconName;
  readonly title: string;
  readonly code?: string;
  readonly note: string;
}

/** A framed panel for one audience or one subject: its points and the page it leads to. */
export interface Panel {
  readonly icon: IconName;
  readonly title: string;
  readonly text?: string;
  readonly points?: readonly string[];
  readonly action: string;
  readonly path: string;
}

/** A step of a flow: its name in the code face, what happens and the effect in the world. */
export interface FlowStep {
  readonly index: string;
  readonly name: string;
  readonly text: string;
  readonly effect?: string;
  readonly change?: boolean;
}

/** A table of evidence: the column names and the rows; a cell may hold code between backticks. */
export interface Evidence {
  readonly columns: readonly string[];
  readonly rows: readonly (readonly string[])[];
}

/** A group of pages of the section under one heading; the titles and the descriptions come from the pages. */
export interface PageGroup {
  readonly title: string;
  readonly slugs: readonly string[];
}

export interface ProductPage {
  readonly program: Heading & { readonly points: readonly Point[]; readonly programs: readonly Program[] };
  readonly numbers: Heading;
  readonly map: Heading & { readonly cells: readonly Cell[]; readonly action: string };
  readonly audiences: Heading & { readonly panels: readonly Panel[] };
}

export interface AiPage {
  readonly numbers: Heading;
  readonly audiences: Heading & { readonly panels: readonly Panel[] };
  readonly call: Heading & { readonly steps: readonly FlowStep[] };
  readonly routes: Heading & { readonly table: Evidence };
  readonly data: Heading & { readonly panels: readonly Panel[] };
}

export interface QualityPage {
  readonly numbers: Heading;
  readonly checks: Heading & { readonly table: Evidence; readonly action: string };
  readonly areas: Heading & { readonly cells: readonly (Cell & { readonly slug: string })[] };
}

export interface ArchitecturePage {
  readonly flow: Heading & { readonly steps: readonly FlowStep[]; readonly action: string };
  readonly entries: Heading & {
    readonly items: readonly { readonly command: string; readonly name: string; readonly note: string }[];
    readonly core: { readonly name: string; readonly text: string };
  };
  readonly layers: Heading & { readonly table: Evidence; readonly action: string };
}

export interface DesignPage {
  readonly idea: Heading & { readonly points: readonly Point[]; readonly imageAlt: string; readonly imageCaption: string; readonly action: string };
  readonly colours: Heading & {
    readonly swatches: readonly { readonly name: string; readonly value: string; readonly role: string }[];
    readonly action: string;
  };
  readonly type: Heading & { readonly mono: string; readonly note: string };
  readonly files: Heading & { readonly panels: readonly Panel[] };
}

export interface ProjectPage {
  readonly release: Heading & {
    readonly version: string;
    readonly date: string;
    readonly text: string;
    readonly actions: readonly { readonly label: string; readonly path: string }[];
    readonly timeline: readonly { readonly label: string; readonly title: string; readonly text: string }[];
  };
  readonly numbers: Heading;
  readonly systems: Heading & { readonly table: Evidence };
  readonly pages: Heading & { readonly groups: readonly PageGroup[] };
}

export interface SectionPagesContent {
  readonly common: { readonly indexEyebrow: string; readonly indexTitle: string };
  readonly product: ProductPage;
  readonly ai: AiPage;
  readonly quality: QualityPage;
  readonly architecture: ArchitecturePage;
  readonly design: DesignPage;
  readonly project: ProjectPage;
}
