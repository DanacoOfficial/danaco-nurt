// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
'use strict';

// Validates nurt.tmLanguage.json and tokenizes the golden programs with a
// minimal TextMate interpreter (match rules, begin/end rules, includes and
// end-pattern back-references). Run with `node test/check.js`.

const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');

const GRAMMAR_FILE = path.resolve(__dirname, '..', 'nurt.tmLanguage.json');
const REPOSITORY_ROOT = path.resolve(__dirname, '..', '..', '..');
const GOLDEN_DIRS = [
  path.join(REPOSITORY_ROOT, 'spec', 'golden', 'ast'),
  path.join(REPOSITORY_ROOT, 'tests', 'd1', 'golden', 'ast'),
];
const SCOPE_SUFFIX = '.nurt';

const grammar = JSON.parse(fs.readFileSync(GRAMMAR_FILE, 'utf8'));

function escapeRegExp(text) {
  return text.replace(/[.*+?^${}()|[\]\\/]/g, '\\$&');
}

function resolveInclude(reference) {
  if (reference === '$self') {
    return { patterns: grammar.patterns };
  }
  assert.ok(reference.startsWith('#'), `unsupported include ${reference}`);
  const rule = grammar.repository[reference.slice(1)];
  assert.ok(rule, `unknown repository entry ${reference}`);
  return rule;
}

function expandPatterns(patterns) {
  const rules = [];
  for (const pattern of patterns || []) {
    if (pattern.include) {
      const target = resolveInclude(pattern.include);
      if (target.match || target.begin) {
        rules.push(target);
      } else {
        rules.push(...expandPatterns(target.patterns));
      }
    } else {
      rules.push(pattern);
    }
  }
  return rules;
}

function compile(source, flags = 'g') {
  return new RegExp(source, flags);
}

function validate() {
  assert.equal(grammar.scopeName, 'source.nurt');
  const visit = (rule, where) => {
    for (const key of ['match', 'begin', 'end']) {
      if (rule[key] !== undefined) {
        assert.doesNotThrow(
          () => compile(rule[key].replace(/\\(\d)/g, '$1')),
          `${where}.${key} does not compile`,
        );
      }
    }
    for (const nested of rule.patterns || []) {
      if (nested.include) {
        resolveInclude(nested.include);
      } else {
        visit(nested, `${where}.patterns`);
      }
    }
  };
  grammar.patterns.forEach((entry) => resolveInclude(entry.include));
  for (const [name, rule] of Object.entries(grammar.repository)) {
    visit(rule, name);
  }
}

function addCaptureTokens(tokens, match, captures, base) {
  const names = Object.keys(captures || {});
  if (names.length === 0) {
    return;
  }
  for (const key of names) {
    const index = Number(key);
    const text = match[index];
    if (text === undefined || text === '') {
      continue;
    }
    const start = index === 0 ? match.index : match.index + match[0].indexOf(text);
    tokens.push({
      start,
      end: start + text.length,
      text,
      scopes: [...base, captures[key].name],
    });
  }
}

function earliest(line, position, rules, endSource) {
  let best = null;
  if (endSource !== null) {
    const re = compile(endSource);
    re.lastIndex = position;
    const match = re.exec(line);
    if (match) {
      best = { kind: 'end', match };
    }
  }
  for (const rule of rules) {
    const re = compile(rule.match || rule.begin);
    re.lastIndex = position;
    const match = re.exec(line);
    if (!match) {
      continue;
    }
    if (best === null || match.index < best.match.index) {
      best = { kind: rule.match ? 'match' : 'begin', rule, match };
    }
  }
  return best;
}

function tokenize(text) {
  const tokens = [];
  const stack = [];
  const lines = text.replace(/\r\n/g, '\n').split('\n');
  const openAtLineStart = [];
  lines.forEach((line, lineNumber) => {
    openAtLineStart.push(stack.length > 0);
    let position = 0;
    let guard = 0;
    while (position <= line.length) {
      guard += 1;
      assert.ok(guard < 10000, `no progress on line ${lineNumber + 1}`);
      const top = stack.length ? stack[stack.length - 1] : null;
      const rules = expandPatterns(top ? top.rule.patterns : grammar.patterns);
      const found = earliest(line, position, rules, top ? top.endSource : null);
      if (!found) {
        break;
      }
      const { match } = found;
      const offset = (token) => ({ ...token, line: lineNumber + 1 });
      const scopes = stack.map((entry) => entry.rule.name).filter(Boolean);
      const emit = (list) => list.forEach((token) => tokens.push(offset(token)));
      if (found.kind === 'match') {
        const local = [];
        if (found.rule.name && !found.rule.captures) {
          local.push({
            start: match.index,
            end: match.index + match[0].length,
            text: match[0],
            scopes: [...scopes, found.rule.name],
          });
        }
        addCaptureTokens(local, match, found.rule.captures, scopes);
        emit(local);
      } else if (found.kind === 'begin') {
        const endSource = found.rule.end.replace(/\\(\d)/g, (_all, group) =>
          escapeRegExp(match[Number(group)] || ''),
        );
        const local = [];
        addCaptureTokens(local, match, found.rule.beginCaptures, [
          ...scopes,
          found.rule.name,
        ]);
        emit(local);
        stack.push({ rule: found.rule, endSource, start: match.index, line: lineNumber, text: match[0] });
      } else {
        const entry = stack.pop();
        const local = [];
        addCaptureTokens(local, match, entry.rule.endCaptures, [...scopes]);
        emit(local);
        const startLine = entry.line === lineNumber ? entry.start : 0;
        const spanText = entry.line === lineNumber ? line.slice(startLine, match.index + match[0].length) : null;
        tokens.push({
          line: lineNumber + 1,
          start: startLine,
          end: match.index + match[0].length,
          text: spanText,
          scopes: [...scopes],
          span: true,
        });
      }
      position = match.index + Math.max(match[0].length, found.kind === 'end' ? 0 : 1);
      if (match[0].length === 0) {
        position = match.index + 1;
      }
    }
    while (stack.length && stack[stack.length - 1].endSource.endsWith('$')) {
      stack.pop();
    }
  });
  return { tokens, open: stack.length, openAtLineStart };
}

const EXPECTATIONS = [
  ['find "/srv/app" glob: "**/*.go"', 'find', 'entity.name.function'],
  ['find "/srv/app" glob: "**/*.go"', 'glob', 'variable.parameter'],
  ['find "/srv/app" glob: "**/*.go"', '"', 'string.quoted.double'],
  ['git.status "/srv/app"', 'git.status', 'entity.name.function'],
  ['search /oldName\\(/ "/srv/app/src"', '/', 'string.regexp'],
  ['search /oldName\\(/i "/srv/app/src"', 'i', 'keyword.other.flags'],
  ['x |> filter size > 1mb', '1mb', 'constant.numeric.size'],
  ['x |> filter modified > ago(1h30m)', '1h30m', 'constant.numeric.duration'],
  ['lines "a" 2.. |> first 3', '2', 'constant.numeric'],
  ['processes |> filter cpu > 3.5', '3.5', 'constant.numeric'],
  ['let recent = find "/a"', 'let', 'keyword.control.let'],
  ['let recent = find "/a"', 'recent', 'variable.other.definition'],
  ['let recent = find "/a"', 'find', 'entity.name.function'],
  ['@recent |> count', '@recent', 'variable.other.reference'],
  ['@recent |> count', '|>', 'keyword.operator'],
  ['@recent |> count', 'count', 'entity.name.function'],
  ['x |> filter not hidden and size >= 1', 'not', 'keyword.operator.word'],
  ['x |> filter not hidden and size >= 1', '>=', 'keyword.operator'],
  ['x |> filter flag == true', 'true', 'keyword.constant.boolean'],
  ['x |> set key: null', 'null', 'keyword.constant.null'],
  ['replace "a" "b" in: ["/f"]', 'in', 'variable.parameter'],
  ['write "/f" "a\\n"', '\\n', 'constant.character.escape'],
  ['write "/f" "a\\q"', '\\q', 'invalid.illegal.escape'],
  ['x # note "quoted"', '# note "quoted"', 'comment.line.number-sign'],
  ['write "/f" "# not a comment"', '"', 'string.quoted.double'],
  ['insert r#"import "fmt""# position: after', 'r#"', 'string.quoted.other.raw'],
  ['x |> verify (run ["go"] cwd: "/a")', 'run', 'entity.name.function'],
  ['x |> filter (size >= 1)', 'size', null],
  ['x |> filter size / 2 > 1', '/', 'keyword.operator'],
];

function checkExpectations() {
  for (const [text, needle, expected] of EXPECTATIONS) {
    const { tokens } = tokenize(text);
    const hits = tokens.filter((token) => token.text === needle && !token.span);
    if (expected === null) {
      assert.equal(hits.length, 0, `${JSON.stringify(needle)} must stay unscoped in ${JSON.stringify(text)}`);
      continue;
    }
    assert.ok(hits.length > 0, `no token ${JSON.stringify(needle)} in ${JSON.stringify(text)}`);
    const scoped = hits.some((hit) => hit.scopes.some((scope) => scope === `${expected}${SCOPE_SUFFIX}`));
    assert.ok(
      scoped,
      `${JSON.stringify(needle)} in ${JSON.stringify(text)}: expected ${expected}${SCOPE_SUFFIX}, got ${hits
        .map((hit) => hit.scopes.join(' '))
        .join(' | ')}`,
    );
  }
}

function goldenFiles(directory) {
  if (!fs.existsSync(directory)) {
    return [];
  }
  const found = [];
  for (const entry of fs.readdirSync(directory, { withFileTypes: true })) {
    const full = path.join(directory, entry.name);
    if (entry.isDirectory()) {
      found.push(...goldenFiles(full));
    } else if (entry.name.endsWith('.nurt')) {
      found.push(full);
    }
  }
  return found.sort();
}

function checkGolden() {
  const files = GOLDEN_DIRS.flatMap(goldenFiles);
  assert.ok(files.length > 0, 'no golden programs found');
  for (const file of files) {
    const text = fs.readFileSync(file, 'utf8');
    const { tokens, open, openAtLineStart } = tokenize(text);
    assert.equal(open, 0, `${path.basename(file)}: unterminated rule at end of file`);
    const lines = text.replace(/\r\n/g, '\n').split('\n');
    const strings = tokens.filter((token) => token.scopes.some((scope) => scope === `punctuation.definition.string.begin${SCOPE_SUFFIX}`));
    const quotes = lines.join('\n').match(/"/g) || [];
    assert.ok(strings.length > 0 || quotes.length === 0, `${path.basename(file)}: no string scopes`);
    const invalid = tokens.filter((token) => token.scopes.some((scope) => scope.startsWith('invalid.')));
    assert.equal(invalid.length, 0, `${path.basename(file)}: invalid scope on ${invalid.map((t) => t.text)}`);
    lines.forEach((line, index) => {
      const first = /^[ \t]*([a-z][a-z0-9_.]*)(?:[ \t]|$)/.exec(line);
      if (!first || first[1] === 'let' || openAtLineStart[index]) {
        return;
      }
      const covered = tokens.some(
        (token) =>
          token.line === index + 1 &&
          token.text === first[1] &&
          token.scopes.includes(`entity.name.function${SCOPE_SUFFIX}`),
      );
      assert.ok(covered, `${path.basename(file)}:${index + 1}: command ${first[1]} not scoped`);
    });
  }
  return files.length;
}

validate();
checkExpectations();
const count = checkGolden();
process.stdout.write(`textmate grammar: ${EXPECTATIONS.length} expectations, ${count} golden programs\n`);
