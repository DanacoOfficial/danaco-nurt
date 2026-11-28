// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           website.src.data.home-page.en
// Responsibility:   Content of the home page in English, the source language.
// Dependencies:     website.src.data.home-page.types

import type { HomeContent } from './types';

export const homeEn: HomeContent = {
  hero: {
    eyebrow: 'Command shell and language',
    title: 'Every change can be undone.',
    lead: 'Danaco Nurt is a command shell with its own language for files, text, structured documents, source code, version control, processes and the operating system. Programs are pipelines of typed records, checked before they run and described by a report of their effects. File changes go to a journal and can be reviewed and undone.',
    note: 'Built for people and AI agents. One tool for Linux, Windows and macOS.',
    sceneLabel: 'The mark of Danaco Nurt in brushed silver and gold: the chevron back and the chevron forward',
    sceneCaption: 'Back and forward: a change and its undo',
  },
  pillars: {
    eyebrow: 'Why Nurt',
    title: 'A shell that answers for what it does',
    cards: [
      { icon: 'undo-2', title: 'Undo is part of the language', text: 'File changes are recorded in a journal. Reviewing an operation, undoing it and redoing it are commands of the language.', path: '/product/journal-and-undo/', shell: false, program: 'history limit: 5\nundo 18' },
      { icon: 'scan-search', title: 'Checked before it runs', text: 'Names, parameters and types are validated against the command catalog before any effect takes place. Errors carry a stable code, a position and a correction.', path: '/product/language/', shell: true, program: 'nurt check -e \'remove "/home/user/app/build"\'' },
      { icon: 'braces', title: 'Records, not text', text: 'Stages exchange records with named fields (files, lines, matches, processes) instead of unstructured text, so a pipeline does not depend on how a tool formats its output.', path: '/product/language/', shell: false, program: 'find "/home/user/app" glob: "**/*.go"\n  |> filter size > 1mb\n  |> sort size order: desc\n  |> first 5' },
      { icon: 'file-text', title: 'A report of every run', text: 'Each run returns a report of its effects in text, JSON or Markdown form. The schema of the report is published in the specification.', path: '/product/effect-reports/', shell: false, program: 'replace /oldName\\(/ "newName(" in: ["/home/user/app/src"] glob: "**/*.go"' },
      { icon: 'bot', title: 'Built for AI agents', text: 'The same engine runs as a command line program, as a Model Context Protocol server, as a JSON-RPC server, as a language server and as a C library.', path: '/product/ai-agents/', shell: true, program: 'nurt mcp --stdio\nnurt serve --stdio' },
      { icon: 'boxes', title: 'One executable, no tools to install', text: 'File, text, archive, document, code, version control, network and system operations are implemented in the program and do not invoke external tools.', path: '/product/platforms/', shell: false, program: 'git.status "/home/user/app"\narchive.list "/home/user/dl/data.zip"\nhttp.request "https://example.com/health" |> assert count: 1' },
    ],
  },
  statement: {
    eyebrow: 'The creator',
    quote: 'I did not want another shell. I wanted a language made for the age of AI. A language that gives full awareness of changes before they are made, full transparency after they are made, and full control over undoing each of them. When I did not find such a solution, I designed it from scratch.',
    role: 'creator of Danaco Nurt',
  },
  how: {
    eyebrow: 'How it works',
    title: 'Write a pipeline. Read the report. Undo if you must.',
    lead: 'A program is a pipeline of stages joined by the pipe operator. Every stage takes records and returns records.',
    programTitle: 'nurt: list the 20 largest Go files changed during the last day',
    program: 'find "/home/user/project" glob: "**/*.go"\n  |> filter size > 1mb and modified > ago(24h)\n  |> sort size order: desc\n  |> first 20',
    sessionTitle: 'nurt 0.1.2: change three files, then undo',
    sessionReplay: 'Play again',
    sessionCaption: 'From a session of nurt 0.1.2: a replacement in three files and its undo.',
    points: [
      { title: 'Check', text: 'The program is parsed and checked against the catalog. A mistake in a name, a parameter or a type stops the program before anything changes.' },
      { title: 'Run', text: 'Stages exchange typed records. The operations are implemented in the program, on all three operating systems.' },
      { title: 'Report', text: 'The run returns a structured report of its effects, in text, JSON or Markdown.' },
      { title: 'Undo', text: 'File changes are in the journal. Reviewing, undoing and redoing an operation are commands of the language.' },
    ],
  },
  numbers: {
    eyebrow: 'In numbers',
    title: 'Danaco Nurt in numbers',
    lead: 'The size of the command catalog and of the workspace, and the results of the tests and of the comparison with git.',
  },
  ai: {
    eyebrow: 'For AI labs and agent engines',
    title: 'One tool for a model to learn and for an agent engine to run',
    lead: 'Danaco Nurt gives an agent one tool with a published definition, programs checked before they run, structured reports and a journal that takes a change back. Model labs receive training packages and a benchmark of tasks.',
    action: 'What AI labs and agent engines get',
    cards: [
      { icon: 'bot', title: 'For agent engines', text: 'A tool call, a Model Context Protocol server, a JSON-RPC server and a C library with bindings, with policy and privilege control.', path: '/ai/agent-engines/' },
      { icon: 'cpu', title: 'For model labs', text: 'The tool definition in eight formats in English and Polish, stable error codes with corrections and versioned interfaces.', path: '/ai/model-labs/' },
      { icon: 'database', title: 'Training packages', text: 'Pairs of a task and a program checked by the engine, chat and tool-call forms, tool definitions and a documentation corpus.', path: '/ai/training-packages/' },
      { icon: 'gauge', title: 'The benchmark of tasks', text: '720 tasks in 12 domains that compare Danaco Nurt with bash and PowerShell as the tool of an agent.', path: '/ai/benchmark/' },
    ],
  },
  design: {
    eyebrow: 'Design',
    title: 'Back in silver, forward in gold',
    lead: 'The mark of Danaco Nurt is two chevrons on the angle of the stream: a change and its undo. The identity is published with its files.',
    cards: [
      { icon: 'layers', title: 'The design of Nurt', text: 'The idea of the mark, its construction, the colour, the typeface and the place of the identity in the product.', path: '/design/overview/' },
      { icon: 'pen-line', title: 'Mark, colour and type', text: 'The grid, the angle and the tones of the mark, the palette with contrast ratios, the typeface of the name and the variants for every ground.', path: '/design/mark-and-colour/' },
      { icon: 'download', title: 'Identity files', text: 'The guidelines, the vector and raster files, the icons, the print specification and the colour tokens to download.', path: '/design/identity/' },
    ],
  },
  quality: {
    eyebrow: 'Quality',
    title: 'Tested, compared and measured',
    lead: 'The quality pages state what is tested and compared, how, and with which result.',
    cards: [
      { icon: 'flask-conical', title: 'Testing', text: 'The test suite of the workspace, the platforms it runs on and the scenarios on a clean system.', path: '/quality/testing/' },
      { icon: 'scale', title: 'Conformance', text: 'Comparison with git, the specification and the conformance cases of the language bindings.', path: '/quality/conformance/' },
      { icon: 'gauge', title: 'Performance', text: 'Journal costs, the limits of a run and the benchmark of tasks.', path: '/quality/performance/' },
      { icon: 'shield-check', title: 'Security and supply chain', text: 'Reporting, advisories, dependency policy, signed commits and signed release checksums.', path: '/quality/supply-chain/' },
    ],
  },
  docs: {
    eyebrow: 'Documentation',
    title: 'A page for every command',
    lead: 'The guide, the cookbook of ready programs, the reference of every command and the specification of the language.',
    cards: [
      { icon: 'rocket', title: 'Guide', text: 'Installation, first steps, the language, the command line, the effect report, the journal and configuration.', path: '/docs/guide/' },
      { icon: 'book-open', title: 'Cookbook', text: 'Ready programs for common tasks, from editing many files to administration.', path: '/docs/cookbook/' },
      { icon: 'terminal', title: 'Commands', text: 'One reference page for every command of the catalog.', path: '/docs/commands/' },
      { icon: 'library', title: 'Specification', text: 'The language, the syntax tree, the catalog, the report, the journal, errors and configuration.', path: '/docs/specification/' },
    ],
  },
  cta: {
    title: 'Install it, change a file, take the change back',
    text: 'Install the executable, run a program, read the report and undo the change.',
    primary: 'Install Danaco Nurt',
    secondary: 'Read the first steps',
  },
};
