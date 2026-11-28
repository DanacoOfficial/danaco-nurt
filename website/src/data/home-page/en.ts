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
    lead: 'The design follows from one promise: you can see what a program will do, what it did, and take it back.',
    cards: [
      { icon: 'undo-2', title: 'Undo is part of the language', text: 'File changes are recorded in a journal. Review an operation, undo it, redo it. Changes outside the file system, such as a service change, are recorded as irreversible.', path: '/product/journal-and-undo/' },
      { icon: 'scan-search', title: 'Checked before it runs', text: 'Names, parameters and types are validated against the command catalog before any effect takes place. Errors carry a stable code, a position and a correction.', path: '/product/language/' },
      { icon: 'braces', title: 'Records, not text', text: 'Stages exchange records with named fields (files, lines, matches, processes) instead of unstructured text, so a pipeline does not depend on how a tool formats its output.', path: '/product/language/' },
      { icon: 'file-text', title: 'A report of every run', text: 'Each run returns a report of its effects in text, JSON or Markdown form. The schema of the report is published in the specification.', path: '/product/effect-reports/' },
      { icon: 'bot', title: 'Built for AI agents', text: 'The same engine runs as a command line program, as a Model Context Protocol server, as a JSON-RPC server, as a language server and as a C library.', path: '/product/ai-agents/' },
      { icon: 'boxes', title: 'One executable, no tools to install', text: 'File, text, archive, document, code, version control, network and system operations are implemented in the program and do not invoke external tools.', path: '/product/platforms/' },
    ],
  },
  how: {
    eyebrow: 'How it works',
    title: 'Write a pipeline. Read the report. Undo if you must.',
    lead: 'A program is a pipeline of stages joined by the pipe operator. Every stage takes records and returns records.',
    programTitle: 'nurt: list the 20 largest Go files changed during the last day',
    program: 'find "/home/user/project" glob: "**/*.go"\n  |> filter size > 1mb and modified > ago(24h)\n  |> sort size order: desc\n  |> first 20',
    sessionTitle: 'nurt 0.1.2: change three files, then undo',
    sessionReplay: 'Play again',
    sessionCaption: 'Output of nurt 0.1.2 in an interactive session; the directory is shortened and two of the three differences are left out.',
    points: [
      { title: 'Check', text: 'The program is parsed and checked against the catalog. A mistake in a name, a parameter or a type stops the program before anything changes.' },
      { title: 'Run', text: 'Stages exchange typed records. The operations are implemented in the program, on all three operating systems.' },
      { title: 'Report', text: 'The run returns a structured report of its effects, in text, JSON or Markdown.' },
      { title: 'Undo', text: 'File changes are in the journal. Reviewing, undoing and redoing an operation are commands of the language.' },
    ],
  },
  numbers: {
    eyebrow: 'In numbers',
    title: 'What is in the box, and how it was checked',
    lead: 'Every number carries its source. Counted values are read from the repository when the site is built; measured values name the run that produced them.',
  },
  quality: {
    eyebrow: 'Quality',
    title: 'Quality is shown, not claimed',
    lead: 'The quality pages state what is tested, how, with which result, and what is still open.',
    cards: [
      { icon: 'flask-conical', title: 'Testing', text: 'The test suite of the workspace, the platforms it runs on and the scenarios on a clean system.', path: '/quality/testing/' },
      { icon: 'scale', title: 'Conformance', text: 'Comparison with git, the specification and the conformance cases of the language bindings.', path: '/quality/conformance/' },
      { icon: 'gauge', title: 'Performance', text: 'Journal costs, the limits of a run and the benchmark of tasks.', path: '/quality/performance/' },
      { icon: 'shield-check', title: 'Security and supply chain', text: 'Reporting, advisories, dependency policy, signed commits and signed release checksums.', path: '/quality/supply-chain/' },
    ],
  },
  docs: {
    eyebrow: 'Documentation',
    title: 'Published from the repository',
    lead: 'The documentation is the one in the repository, imported when the site is built, so the site and the source never disagree.',
    cards: [
      { icon: 'rocket', title: 'Guide', text: 'Installation, first steps, the language, the command line, the effect report, the journal and configuration.', path: '/docs/guide/' },
      { icon: 'book-open', title: 'Cookbook', text: 'Ready programs for common tasks, from editing many files to administration.', path: '/docs/cookbook/' },
      { icon: 'terminal', title: 'Commands', text: 'One reference page for every command of the catalog.', path: '/docs/commands/' },
      { icon: 'library', title: 'Specification', text: 'The language, the syntax tree, the catalog, the report, the journal, errors and configuration.', path: '/docs/specification/' },
    ],
  },
  cta: {
    title: 'Try it on a directory you can afford to change',
    text: 'Install the executable, run a program, read the report and undo the change.',
    primary: 'Install Danaco Nurt',
    secondary: 'Read the first steps',
  },
};
