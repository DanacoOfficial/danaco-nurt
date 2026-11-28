// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           website.src.lib.docs.frontmatter
// Responsibility:   Reading of a documentation file: the YAML front matter (with a line-by-line reading for files whose front matter is not strict YAML), the title, the description and the body without the header comments.
// Dependencies:     yaml

import { parse } from 'yaml';

export interface SourceFile {
  readonly front: Record<string, unknown>;
  readonly title: string;
  readonly description: string;
  readonly body: string;
}

const FRONT_MATTER = /^---\r?\n([\s\S]*?)\r?\n---\r?\n?/;
const LEADING_COMMENTS = /^(?:\s*<!--[\s\S]*?-->\s*)+/;
const FIRST_HEADING = /^#\s+(.+?)\s*#*\s*$/m;

function parseLenient(block: string): Record<string, unknown> {
  const result: Record<string, unknown> = {};
  for (const line of block.split(/\r?\n/)) {
    const match = /^([A-Za-z_][\w-]*):\s*(.*)$/.exec(line);
    if (!match) {
      continue;
    }
    const value = match[2].trim();
    if (value.startsWith('[') && value.endsWith(']')) {
      result[match[1]] = value
        .slice(1, -1)
        .split(',')
        .map((item) => item.trim())
        .filter((item) => item !== '');
    } else if (/^-?\d+$/.test(value)) {
      result[match[1]] = Number(value);
    } else {
      result[match[1]] = value.replace(/^["']|["']$/g, '');
    }
  }
  return result;
}

function stripFrontMatter(text: string): { front: Record<string, unknown>; rest: string } {
  const match = FRONT_MATTER.exec(text);
  if (!match) {
    return { front: {}, rest: text };
  }
  let parsed: unknown;
  try {
    parsed = parse(match[1]);
  } catch (error) {
    parsed = parseLenient(match[1]);
  }
  const front = parsed && typeof parsed === 'object' ? (parsed as Record<string, unknown>) : {};
  return { front, rest: text.slice(match[0].length) };
}

function firstParagraph(body: string): string {
  let inFence = false;
  const lines: string[] = [];
  for (const line of body.split(/\r?\n/)) {
    if (line.trimStart().startsWith('```')) {
      inFence = !inFence;
      continue;
    }
    if (inFence || line.startsWith('#') || line.startsWith('|') || line.startsWith('<!--')) {
      if (lines.length > 0) {
        break;
      }
      continue;
    }
    if (line.trim() === '') {
      if (lines.length > 0) {
        break;
      }
      continue;
    }
    lines.push(line.trim());
  }
  const text = lines
    .join(' ')
    .replace(/\[([^\]]+)\]\([^)]*\)/g, '$1')
    .replace(/[`*_]/g, '')
    .trim();
  return text.length > 220 ? `${text.slice(0, 217).trimEnd()}...` : text;
}

export function readSource(text: string, stem: string): SourceFile {
  const { front, rest } = stripFrontMatter(text);
  const withoutComments = rest.replace(LEADING_COMMENTS, '');
  const heading = FIRST_HEADING.exec(withoutComments);
  const title =
    (typeof front.title === 'string' && front.title) ||
    (heading ? heading[1].replace(/[`]/g, '') : '') ||
    (typeof front.name === 'string' && front.name) ||
    stem;
  const body = heading && withoutComments.trimStart().startsWith(heading[0]) ? withoutComments.replace(heading[0], '').replace(/^\s+/, '') : withoutComments;
  const description =
    (typeof front.description === 'string' && front.description) || (typeof front.summary === 'string' && typeof front.command === 'string' ? `${front.command}: ${front.summary}` : '') || firstParagraph(body);
  return { front, title, description, body };
}
