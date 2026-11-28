// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           website.src.data.session
// Responsibility:   The terminal session played on the home page: a change of three files and its undo, recorded with nurt 0.1.2 in an interactive session. The text is the output of the program; the directory is shortened to /home/user/app, the number of the journal entry is that of a fresh journal, and the differences of a.go and c.go are left out.
// Dependencies:     none.

export type SessionTone = 'prompt' | 'indicator' | 'input' | 'plain' | 'status' | 'dim' | 'removed' | 'added' | 'heading';

export interface SessionSegment {
  readonly text: string;
  readonly tone: SessionTone;
}

export interface SessionLine {
  readonly kind: 'input' | 'output' | 'pause';
  readonly segments: readonly SessionSegment[];
  /** The step of the explanation beside the window that this line illustrates: 0 check, 1 run, 2 report, 3 undo. */
  readonly step?: number;
}

const prompt: SessionSegment[] = [
  { text: 'nurt', tone: 'prompt' },
  { text: '〉 ', tone: 'indicator' },
];

const input = (text: string): SessionLine => ({ kind: 'input', segments: [...prompt, { text, tone: 'input' }] });
const output = (...segments: SessionSegment[]): SessionLine => ({ kind: 'output', segments });
const plain = (text: string): SessionSegment => ({ text, tone: 'plain' });
const dim = (text: string): SessionSegment => ({ text, tone: 'dim' });

export const sessionVersion = '0.1.2';

export const sessionLines: readonly SessionLine[] = [
  { ...input('replace /oldName\\(/ "newName(" in: ["/home/user/app/src"] glob: "**/*.go"'), step: 0 },
  { ...output(plain('op 18  replace  '), { text: 'ok', tone: 'status' }), step: 1 },
  { ...output(dim('targets '), plain('3 files · replacements 5 · lines +3 −3 · 183b → 183b')), step: 2 },
  output(plain('  /home/user/app/src/a.go  1 repl  55b → 55b  '), dim('10fd934a → a03340fa')),
  output(plain('  /home/user/app/src/b.go  2 repl  66b → 66b  '), dim('66ead61f → 314f7aa1')),
  output(plain('  /home/user/app/src/c.go  2 repl  62b → 62b  '), dim('d3b76e47 → 92f4081f')),
  output({ text: 'diff (3 hunks)', tone: 'heading' }),
  output(dim('  /home/user/app/src/b.go:1')),
  output({ text: '    -     return oldName(3) + oldName(4)', tone: 'removed' }),
  output({ text: '    +     return newName(3) + newName(4)', tone: 'added' }),
  { kind: 'pause', segments: [] },
  { ...input('undo'), step: 3 },
  output(plain('undo  '), { text: 'ok', tone: 'status' }),
  output(dim('targets '), plain('3 files · 183b → 183b')),
  output(plain('  /home/user/app/src/c.go    62b → 62b  '), dim('92f4081f → d3b76e47')),
  output(plain('  /home/user/app/src/b.go    66b → 66b  '), dim('314f7aa1 → 66ead61f')),
  output(plain('  /home/user/app/src/a.go    55b → 55b  '), dim('a03340fa → 10fd934a')),
];
