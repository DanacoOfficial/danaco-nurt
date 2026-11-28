// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           website.src.data.section-pages.en
// Responsibility:   The English content of the landing pages of the sections. Every statement summarises a page of the section; figures that the build counts come from website.src.data.facts.
// Dependencies:     website.src.data.section-pages.types

import type { SectionPagesContent } from './types';

export const sectionPagesEn: SectionPagesContent = {
  common: { indexEyebrow: 'All pages', indexTitle: 'Every page of this section' },

  product: {
    program: {
      eyebrow: 'What it is',
      title: 'A shell where a program is a pipeline of typed records',
      lead: 'One tool for files, text, structured documents, source code, version control, processes and the operating system, delivered as one executable per operating system.',
      points: [
        { title: 'Checked before it runs', text: 'Names, parameters and types are validated against the command catalog before any effect takes place. An error carries a stable code, a position and a correction.' },
        { title: 'Undo is part of the language', text: 'File changes are recorded in a journal. An operation is reviewed, undone and redone with commands of the language.' },
        { title: 'A report of every run', text: 'Each run returns a report of its effects in text, JSON or Markdown form, with a schema published in the specification.' },
        { title: 'Operations built into the program', text: 'File, text, archive, document, code, version control, network and system operations are implemented in the program and do not invoke external tools.' },
      ],
      programs: [
        {
          title: 'nurt: the 20 largest Go files changed during the last day',
          code: 'find "/home/user/project" glob: "**/*.go"\n  |> filter size > 1mb and modified > ago(24h)\n  |> sort size order: desc\n  |> first 20',
          caption: 'Every stage takes records and returns records, so filter and sort address a field by its name.',
        },
        {
          title: 'nurt: a replacement that states how many files it expects',
          code: 'search /oldName\\(/ "/srv/app/src" glob: "**/*.go"\n  |> assert paths: 12\n  |> replace "newName("',
          caption: 'The program stops with E_ASSERT before any change unless the search found exactly 12 files.',
        },
        {
          title: 'nurt: review the journal and take an operation back',
          code: 'history limit: 5\nundo 18',
          caption: 'The journal lists the operations; undo restores the state before operation 18.',
        },
      ],
    },
    numbers: { eyebrow: 'In the box', title: 'One executable, the whole catalog' },
    map: {
      eyebrow: 'What the commands work on',
      title: 'Eleven areas of work, one syntax',
      lead: 'The command catalog is grouped by the object of the work. Every command has a page in the documentation.',
      action: 'All capabilities',
      cells: [
        { icon: 'folder-tree', title: 'Files and directories', code: 'list find info read lines write copy move remove rename create attributes', note: 'Every change goes through the journal, so it can be undone.' },
        { icon: 'file-pen', title: 'Text', code: 'search replace insert edit diff patch hash convert', note: 'Literal text, regular expressions or syntax patterns; edit applies a list of changes to one file, all or none.' },
        { icon: 'table', title: 'Records', code: 'filter select sort first last skip count group summarize distinct', note: 'Work on the records produced by any source command.' },
        { icon: 'braces', title: 'Structured documents', code: 'set query sql', note: 'JSON, YAML, TOML, XML, INI, CSV and more by key path; set preserves the layout of the document.' },
        { icon: 'file-code', title: 'Source code', code: 'code.symbols code.definition code.references code.rename', note: 'Thirteen commands that list symbols, find definitions and references and rename by syntax.' },
        { icon: 'git-branch', title: 'Version control', code: 'git.status git.diff git.commit git.rebase git.push', note: 'Twenty-two commands that read and change repositories inside the program.' },
        { icon: 'archive', title: 'Archives', code: 'archive.list archive.extract archive.create compress decompress', note: 'Archives and compressed files.' },
        { icon: 'globe', title: 'Network', code: 'http.request http.download network.ping network.resolve', note: 'A download writes a file and can be undone; a changing request is listed as irreversible in the report.' },
        { icon: 'activity', title: 'Processes', code: 'run jobs retry stop processes ports suspend resume', note: 'run starts a program without a shell and continues it as a background job.' },
        { icon: 'server', title: 'System administration', code: 'services start restart enable disable windows.cim', note: 'Thirty-four commands: services, accounts, groups, mounts, disks, the firewall, routes, DNS and the clock.' },
        { icon: 'calendar', title: 'Scheduling', code: 'schedule.create schedule.list schedule.run schedule.history', note: 'One syntax replaces cron, systemd timers, launchd jobs and the Windows Task Scheduler.' },
      ],
    },
    audiences: {
      eyebrow: 'Who it is for',
      title: 'Three ways into the product',
      panels: [
        {
          icon: 'terminal',
          title: 'People at a terminal',
          points: ['A mistaken remove or replace can be taken back.', 'A program that cannot be valid is refused before it touches a file.', 'Seven concrete tasks with real programs from the cookbook.'],
          action: 'Use cases',
          path: '/product/use-cases/',
        },
        {
          icon: 'bot',
          title: 'AI agents and their platforms',
          points: ['The same engine as a tool through the Model Context Protocol, a JSON-RPC server or a library.', 'Every call returns a report that a program can read.', 'Policy and privilege control before a program runs.'],
          action: 'Nurt for AI agents',
          path: '/product/ai-agents/',
        },
        {
          icon: 'route',
          title: 'Teams moving off bash and PowerShell',
          points: ['The documentation maps the habits of both shells to their forms in Nurt.', 'nurt migrate translates conditions, loops, functions and script parameters.', 'A table of paired commands and cheat sheets for each platform.'],
          action: 'Migration',
          path: '/product/migration/',
        },
      ],
    },
  },

  ai: {
    numbers: { eyebrow: 'What ships', title: 'Data and definitions that come with the tool' },
    audiences: {
      eyebrow: 'Two audiences',
      title: 'One tool for a model to learn and for an agent engine to run',
      lead: 'Danaco Nurt gives a model one tool in place of the many tools of a shell. The engine checks a program, performs the effect, records it and reports it.',
      panels: [
        {
          icon: 'cpu',
          title: 'For model labs',
          text: 'What a lab that trains or evaluates models receives.',
          points: ['The tool definition in eight formats, in English and Polish.', 'Stable error codes with a position and a correction that a model applies.', 'Five training packages checked by the engine and a held-out task set.', 'Versioned interfaces that a trained model targets.'],
          action: 'For model labs',
          path: '/ai/model-labs/',
        },
        {
          icon: 'bot',
          title: 'For agent engines',
          text: 'What the builder of an agent engine or an agent platform receives.',
          points: ['Four integration routes that take the same request and return the same response.', 'Sessions, events and cancellation.', 'Policy and privilege control, multi-agent work and remote use.', 'Integration examples and a setup action for continuous integration.'],
          action: 'For agent engines',
          path: '/ai/agent-engines/',
        },
      ],
    },
    call: {
      eyebrow: 'One tool call',
      title: 'What happens when a model calls the tool',
      lead: 'The same four things happen whichever route delivers the program.',
      steps: [
        { index: '1', name: 'check', text: 'The program is parsed and bound to the command catalog before it does anything. An error returns a stable code, the position in the program and a correction.', effect: 'no effect' },
        { index: '2', name: 'run', text: 'Stages exchange typed records. The same program text runs on Linux, Windows and macOS.', effect: 'change', change: true },
        { index: '3', name: 'report', text: 'The run returns a nurt-report/1 report in text, JSON, Markdown or compact form, paged and bounded in size; next returns the rest of a long result.', effect: 'no effect' },
        { index: '4', name: 'undo', text: 'File changes are in the journal, so a mistake of the model is reviewed and taken back with a command of the language.', effect: 'rollback' },
      ],
    },
    routes: {
      eyebrow: 'Integration',
      title: 'Four routes to the same engine',
      lead: 'Every route takes a nurt-request/1 request and returns a nurt-response/1 response with a nurt-report/1 effect report.',
      table: {
        columns: ['Route', 'Started with', 'Used when'],
        rows: [
          ['Tool call', '`nurt exec "<program>"`', 'a host or a toolkit starts one process per call'],
          ['Model Context Protocol server', '`nurt mcp` or `nurt mcp --http <address>`', 'an MCP client starts it; no integration code is written'],
          ['JSON-RPC server', '`nurt serve --stdio`', 'an engine runs its own agent loop; one process per session or workspace'],
          ['C interface and bindings', '`libnurt`, `nurt.h` and `bindings/`', 'short calls without effects in the host process; bindings for Python, TypeScript, Go, Java and .NET'],
        ],
      },
    },
    data: {
      eyebrow: 'For training and evaluation',
      title: 'Training packages and a benchmark of tasks',
      panels: [
        {
          icon: 'database',
          title: 'Training packages',
          points: ['pairs: 1,457 accepted pairs of a task and a program, and 629 rejected programs with the error code of the engine.', 'chat and tool-calls: the accepted pairs as conversations, 1,309 for training and 148 for validation.', 'tools: the tool definition in 8 formats and 2 languages.', 'corpus: 251 documents of the documentation and the specification.'],
          action: 'Training packages',
          path: '/ai/training-packages/',
        },
        {
          icon: 'gauge',
          title: 'The benchmark of tasks',
          points: ['nurt-bench compares Danaco Nurt with bash and PowerShell 7 as the tool of an AI agent.', '720 tasks in 12 domains: 648 public and 72 held out.', 'An oracle decides the result of every task.', 'A protocol and commands that a lab runs on its own models.'],
          action: 'The benchmark',
          path: '/ai/benchmark/',
        },
      ],
    },
  },

  quality: {
    numbers: { eyebrow: 'Measured', title: 'Results of the test runs' },
    checks: {
      eyebrow: 'What is checked',
      title: 'Every change passes the same checks',
      lead: 'The workflows format and lint the code, build and test the workspace on Linux, macOS and Windows, measure line coverage, build the documentation and evaluate the dependency policy.',
      action: 'The full map of checks',
      table: {
        columns: ['Check', 'What it verifies', 'Result'],
        rows: [
          ['Workspace tests', 'the behaviour of the crates, unit and integration level', '13,965 passed (version 0.1.1, Linux, 27 November 2026)'],
          ['Windows 11 tests', 'the same suite on Windows', '13,455 passed, none failed (version 0.1.1)'],
          ['Differential tests against git', 'the `git.*` commands produce the state that `git` produces, and their undo restores the initial state', '775 of 775 agree (version 0.1.1)'],
          ['Conformance of the bindings', 'Go, Node, .NET, Java and Python call the engine as the specification states', 'the bindings pass; Python 136 of 136 (version 0.1.1)'],
          ['Clean-system scenarios', 'the release archives work on a minimal Debian 13', '19 scenarios passed; `gnu` and `musl` archives pass (version 0.1.1)'],
          ['Line coverage', 'the share of lines that the tests execute', 'the job requires 80 percent for the workspace and 90 percent for six critical crates'],
          ['Lint and format', 'formatting, compiler and clippy warnings', '`fmt` and `clippy -D warnings` clean on Linux and Windows (version 0.1.1)'],
          ['Dependency policy', 'licences, bans, sources and advisories', '`cargo deny` without errors; `cargo audit` exit code 0 (version 0.1.1)'],
          ['Packaging tests', 'installers, packages and manifests', '174 of 174 (version 0.1.1)'],
        ],
      },
    },
    areas: {
      eyebrow: 'Seven areas',
      title: 'Where each kind of evidence is described',
      cells: [
        { slug: 'testing', icon: 'flask-conical', title: 'Testing', note: 'The suites, the platforms, the privileged and volume tests, aarch64 and the scenarios on release archives.' },
        { slug: 'conformance', icon: 'check-check', title: 'Conformance', note: 'The comparison with git, the specification, the conformance suite of the bindings and the compatibility program.' },
        { slug: 'performance', icon: 'gauge', title: 'Performance', note: 'Measured changes of 0.1.2 and 0.1.1, the limits that tests enforce and the scale tests.' },
        { slug: 'reliability', icon: 'hard-drive', title: 'Reliability', note: 'What the journal guarantees after a crash and on a full disk, and which effects are marked irreversible.' },
        { slug: 'security', icon: 'shield-check', title: 'Security', note: 'The security model, the policy of privilege, how a vulnerability is reported and the checks in continuous integration.' },
        { slug: 'supply-chain', icon: 'package-check', title: 'Supply chain and releases', note: 'The dependency policy, how archives are built, checksummed and signed, and how to verify them.' },
        { slug: 'engineering-standards', icon: 'list-checks', title: 'Engineering standards', note: 'The code style, the commit convention, the licence headers and the checks that enforce them.' },
      ],
    },
  },

  architecture: {
    flow: {
      eyebrow: 'One direction',
      title: 'A program passes through the engine in nine steps',
      lead: 'Text is parsed into a syntax tree, the tree is bound to the command catalog and checked, a plan is made, the executor runs the stages, commands change the world through the journal and a report describes the effects.',
      action: 'The engine, step by step',
      steps: [
        { index: '0', name: 'parse', text: 'Text is corrected and parsed into the surface tree.', effect: 'none' },
        { index: '1', name: 'bind', text: 'The tree is bound to the catalog; static checks of every stage.', effect: 'none' },
        { index: '2', name: 'read', text: 'Sources and transforms produce records.', effect: 'reads only' },
        { index: '3', name: 'check', text: 'Checks before the effect (assert).', effect: 'none' },
        { index: '4', name: 'plan', text: 'The effect stage computes its targets; preview ends here.', effect: 'reads only' },
        { index: '5', name: 'apply', text: 'The effect stage changes the world through a journal transaction.', effect: 'change', change: true },
        { index: '5a', name: 'after', text: 'Stages after the effect read the records of the effect.', effect: 'none' },
        { index: '6', name: 'verify', text: 'The verify pipeline runs; failure rolls the effect back.', effect: 'rollback' },
        { index: '7', name: 'report', text: 'The effect report is built.', effect: 'none' },
      ],
    },
    entries: {
      eyebrow: 'Entry points',
      title: 'Five ways in, one engine',
      lead: 'The entry points do not implement the language themselves. A request that arrives through any of them is the same document, nurt-request/1.',
      items: [
        { command: 'nurt exec', name: 'Command line program', note: 'The nurt executable and the Rust interface of the product.' },
        { command: 'nurt mcp', name: 'Model Context Protocol server', note: 'MCP tools and resources, definitions of the nurt tool.' },
        { command: 'nurt serve', name: 'Protocol server', note: 'The serve protocol nurt-rpc/1 for hosts that run their own loop.' },
        { command: 'nurt lsp', name: 'Language server', note: 'Diagnostics, completion and formatting of .nurt files.' },
        { command: 'libnurt', name: 'C interface and bindings', note: 'The C ABI, nurt.h and the bindings for Go, Node.js, .NET, Java and Python.' },
      ],
      core: { name: 'Engine · nurt-core', text: 'Every entry point calls the same Rust facade, Engine of the crate nurt-core.' },
    },
    layers: {
      eyebrow: 'The workspace',
      title: 'The layers and their crates',
      lead: 'Danaco Nurt is a Cargo workspace. The allowed dependencies between its crates are data in tables/crate-graph.toml, and a test fails when a crate declares a dependency that the table does not allow.',
      action: 'Every crate with its description',
      table: {
        columns: ['Layer', 'Role', 'Crates'],
        rows: [
          ['Language front end', 'lexer, parsers of the three notation variants, canonical printer, notation correction, help, migration of shell scripts', '`nurt-syntax` `nurt-correct` `nurt-help` `nurt-migrate`'],
          ['Catalog, checker and planner', 'command registry, binding of syntax trees to the catalog, execution planning', '`nurt-catalog` `nurt-bind` `nurt-plan`'],
          ['Executor', 'streaming pipeline executor and record stages', '`nurt-exec`'],
          ['Journal and file system', 'operation journal, free space for the journal, platform file system layer', '`nurt-journal` `nurt-space` `nurt-fs`'],
          ['Report', 'effect report model and renderers', '`nurt-report`'],
          ['Engine and service', 'engine facade, background service', '`nurt-core` `nurt-service`'],
          ['Command line program', 'the `nurt` executable and the Rust interface of the product', '`nurt-cli` `nurt`'],
          ['Model Context Protocol server', 'MCP tools and resources, definitions of the `nurt` tool', '`nurt-mcp` `nurt-tooldef`'],
          ['Language server', 'diagnostics, completion and formatting of `.nurt` files', '`nurt-langserver`'],
          ['C interface and bindings', 'the C ABI and the Node.js addon', '`nurt-ffi` `nurt-node`'],
        ],
      },
    },
  },

  design: {
    idea: {
      eyebrow: 'The idea',
      title: 'Back in silver, forward in gold',
      lead: 'The mark consists of two identical chevrons. The gold chevron is the silver one turned by 180 degrees, so both are one drawing used twice.',
      imageAlt: 'The mark of Danaco Nurt: a silver chevron pointing back and a gold chevron pointing forward.',
      imageCaption: 'Render of the mark, production identity 1.0.0.',
      action: 'The design of Nurt',
      points: [
        { title: 'Silver: a step back', text: 'The left chevron is silver and points back: the undo.' },
        { title: 'Gold: a step forward', text: 'The right chevron is gold and points forward: the change.' },
        { title: 'The stream between them', text: 'The channel between the parallel arms is the stream, 5.5 units wide along the whole ribbon, half of the stroke of 11 units.' },
        { title: 'One angle', text: 'The arms of both chevrons have a slope of 3 : 4 (36.87 degrees), the angle of every diagonal of the identity.' },
      ],
    },
    colours: {
      eyebrow: 'Colour',
      title: 'Six colours carry the identity',
      lead: 'Each colour has one role. On graphite, silver has a contrast of 14.9 : 1 and gold 8.1 : 1.',
      action: 'The mark, colour and type',
      swatches: [
        { name: 'Graphite', value: '#0B0C0E', role: 'main ground' },
        { name: 'Silver', value: '#DDE1E6', role: 'step back' },
        { name: 'Gold', value: '#C9A15A', role: 'step forward' },
        { name: 'Ink', value: '#2B2F36', role: 'ink on light' },
        { name: 'Gold deep', value: '#A3722A', role: 'gold on light, marks and lines' },
        { name: 'Paper', value: '#F6F7F8', role: 'light ground' },
      ],
    },
    type: {
      eyebrow: 'Type',
      title: 'The company recedes, the product advances',
      lead: 'The name is set in Geist, a neutral geometric grotesque under the SIL Open Font License. Danaco is set in weight 400 and Nurt in weight 700, as the chevrons do.',
      mono: 'find "/home/user/app" |> first 5',
      note: 'Geist Mono is the face of the terminal and of code.',
    },
    files: {
      eyebrow: 'Three pages',
      title: 'From the idea to the files',
      panels: [
        { icon: 'eye', title: 'The design of Nurt', text: 'The name, the mark of two chevrons, the angle of the stream, the type, the grounds and where the identity appears in the product.', action: 'Read', path: '/design/overview/' },
        { icon: 'sliders-horizontal', title: 'The mark, colour and type', text: 'The construction of the mark on a grid of 96 by 96 units, the drawings for small sizes, the colour palette with contrast and the type.', action: 'Read', path: '/design/mark-and-colour/' },
        { icon: 'download', title: 'Identity files', text: 'The identity book and the vector and raster files of the mark to download.', action: 'Open', path: '/design/identity/' },
      ],
    },
  },

  project: {
    release: {
      eyebrow: 'Current release',
      title: 'Version 0.1.2',
      version: '0.1.2',
      date: '28 November 2026',
      text: 'A setup wizard for Windows, search and replace with a regular expression in time proportional to the matches, a static check that accepts every valid literal and a journal that is active on the data volume of macOS.',
      actions: [
        { label: 'Releases', path: '/project/releases/' },
        { label: 'Current state', path: '/project/status/' },
      ],
      timeline: [
        { label: '2026-11-28', title: 'Version 0.1.2', text: 'The setup wizard for Windows, faster search and replace, the journal on the data volume of macOS.' },
        { label: '2026-11-27', title: 'Version 0.1.1', text: 'Control constructs, administration commands, privilege elevation, the command next and a faster journal for many small files.' },
        { label: '2026-11-22', title: 'Version 0.1.0', text: 'The first release: the engine, the language, the command catalog, the journal, the command line program, the Model Context Protocol server, the language server and the C interface.' },
      ],
    },
    numbers: { eyebrow: 'The repository', title: 'What the project consists of' },
    systems: {
      eyebrow: 'Platforms',
      title: 'Three operating systems, one executable each',
      lead: 'nurt on Linux and macOS, nurt.exe on Windows. The executable has no run-time dependencies beyond the system itself.',
      table: {
        columns: ['System', 'Architectures', 'Archives of release 0.1.2'],
        rows: [
          ['Linux', 'x86-64, aarch64', 'glibc and static musl'],
          ['Windows', 'x86-64, aarch64', 'MSVC, as an archive and as a setup wizard'],
          ['macOS', 'x86-64, aarch64', 'build from source'],
        ],
      },
    },
    pages: {
      eyebrow: 'The project',
      title: 'Releases, rules and people',
      groups: [
        { title: 'The release', slugs: ['releases', 'status', 'versioning'] },
        { title: 'Taking part', slugs: ['contributing', 'support'] },
        { title: 'Terms and people', slugs: ['license', 'about'] },
      ],
    },
  },
};
