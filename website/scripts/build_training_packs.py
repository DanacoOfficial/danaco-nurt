# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# https://github.com/DanacoOfficial | support@danaco-group.pl
#
# Module:           website.scripts.build_training_packs
# Responsibility:   Builds the training packages of the website in website/training from the repository: the pair data set (candidates of nurt-bench, each program checked by the engine), the chat and tool-call forms of the pairs, the tool definitions of every format, and the documentation corpus. Writes manifest.json with the counts, the sizes and the SHA-256 sums.
# Dependencies:     the executables nurt and nurt-bench of the same release, the directories bench/, docs/ and spec/ of the repository

"""Builds the training packages of the website from the repository.

Usage: python3 scripts/build_training_packs.py --nurt <nurt> --nurt-bench <nurt-bench>
"""

import argparse
import datetime
import hashlib
import json
import pathlib
import re
import shutil
import subprocess
import sys
import tempfile
import tomllib

WEBSITE = pathlib.Path(__file__).resolve().parent.parent
REPOSITORY = WEBSITE.parent
TARGET = WEBSITE / 'training'
CHECK_BASE = '/home/user/app'
TOOL_FORMATS = ('anthropic', 'openai', 'openai-strict', 'openai-responses', 'openai-responses-strict', 'gemini', 'mcp', 'json-schema')
TOOL_LANGUAGES = ('en', 'pl')
CORPUS_SOURCES = (
    ('docs/site', 'guide'),
    ('docs/guides', 'guides'),
    ('docs/cookbook', 'cookbook'),
    ('docs/commands', 'commands'),
    ('docs/groups', 'groups'),
    ('docs/cheatsheets', 'cheatsheets'),
    ('docs/reference', 'reference'),
    ('docs/integrators', 'integrators'),
    ('spec', 'specification'),
)
SYSTEM_MESSAGE = 'You write programs in the language of Danaco Nurt (`nurt`), a command shell where a program is a pipeline of typed records. Answer with the program only, in the canonical notation.'
INPUT_PROMPTS = {
    'nl': '{input}',
    'bash': 'Write the Danaco Nurt program that does the same as this bash command:\n\n{input}',
    'pwsh': 'Write the Danaco Nurt program that does the same as this PowerShell command:\n\n{input}',
}
HEADER_COMMENT = re.compile(r'^<!--.*?-->\s*$', re.M)
FRONT_MATTER = re.compile(r'\A---\n.*?\n---\n', re.S)
INITIAL_TREE = re.compile(r'^```[^\n]*fs-before[^\n]*\n.*?^```\n', re.S | re.M)


def write_lines(path: pathlib.Path, records: list[dict]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open('w', encoding='utf-8') as handle:
        for record in records:
            handle.write(json.dumps(record, ensure_ascii=False, separators=(',', ':')) + '\n')


def write_json(path: pathlib.Path, value: object) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, ensure_ascii=False, indent=2) + '\n', encoding='utf-8')


def habit_tables(directory: pathlib.Path) -> list[pathlib.Path]:
    """Writes the corrected entries of the cheat sheets as habit tables of nurt-bench."""
    tables = []
    for name, shell in (('bash', 'bash'), ('powershell', 'pwsh')):
        source = tomllib.loads((REPOSITORY / 'docs/cheatsheets' / f'{name}.toml').read_text(encoding='utf-8'))
        lines = ['format = "nurt-bench-habits/1"', '']
        for entry in source['entry']:
            if entry.get('outcome') != 'corrected':
                continue
            lines += [
                '[[pair]]',
                f'shell = {json.dumps(shell)}',
                f'habit = {json.dumps(entry["habit"])}',
                f'canonical = {json.dumps(entry["canonical"])}',
                f'rule = {json.dumps(entry["id"])}',
                '',
            ]
        table = directory / f'{name}.toml'
        table.write_text('\n'.join(lines), encoding='utf-8')
        tables.append(table)
    return tables


def command_pages(directory: pathlib.Path) -> pathlib.Path:
    """Copies the command pages without the initial trees of their examples: the programs of this build are checked by the engine, not run on a tree."""
    target = directory / 'commands'
    target.mkdir()
    for file in sorted((REPOSITORY / 'docs/commands').glob('*.md')):
        target.joinpath(file.name).write_text(INITIAL_TREE.sub('', file.read_text(encoding='utf-8')), encoding='utf-8')
    return target


def candidates(nurt_bench: str, work: pathlib.Path) -> list[dict]:
    """Runs the data set generator of nurt-bench and returns its candidates."""
    command = [nurt_bench, 'pairs', '--tasks', 'bench/tasks', '--fixtures', 'bench/fixtures', '--docs', str(command_pages(work)), '--list-only', '--out', str(work / 'pairs')]
    for table in habit_tables(work):
        command += ['--habits', str(table)]
    subprocess.run(command, cwd=REPOSITORY, check=True, capture_output=True, text=True)
    produced = work / 'pairs'
    text = (produced / 'candidates.jsonl').read_text(encoding='utf-8')
    return [json.loads(line) for line in text.splitlines() if line.strip()]


class Checker:
    """Checks programs with the engine through the serve protocol of nurt."""

    def __init__(self, nurt: str) -> None:
        self.process = subprocess.Popen([nurt, 'serve', '--stdio'], stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True, encoding='utf-8')
        self.sequence = 0
        self.engine = self.call('initialize', {'protocol': 'nurt-rpc/1'})['result']

    def call(self, method: str, params: dict | None = None) -> dict:
        self.sequence += 1
        message = {'jsonrpc': '2.0', 'id': self.sequence, 'method': method}
        if params is not None:
            message['params'] = params
        assert self.process.stdin and self.process.stdout
        self.process.stdin.write(json.dumps(message) + '\n')
        self.process.stdin.flush()
        while True:
            answer = json.loads(self.process.stdout.readline())
            if answer.get('id') == self.sequence:
                return answer

    def check(self, program: str) -> tuple[bool, str, str]:
        """Returns whether the program binds, the reason of a rejection and the revision of the catalog."""
        answer = self.call('check', {'nurt': '1.0', 'program': program, 'options': {'base': CHECK_BASE}})
        result = answer.get('result') or {}
        if 'bound' in result:
            return True, '', result['bound'].get('catalog_rev', '')
        error = (result.get('report') or {}).get('error') or answer.get('error') or {}
        return False, f'{error.get("code", "E_UNKNOWN")}: {error.get("message", "")}'.strip(), ''

    def close(self) -> None:
        self.call('shutdown')
        self.process.wait(timeout=10)


def build_pairs(records: list[dict], checker: Checker) -> tuple[list[dict], list[dict], str]:
    accepted, rejected, catalog = [], [], ''
    for record in records:
        binds, reason, revision = checker.check(record['program'])
        catalog = catalog or revision
        if binds:
            accepted.append({**record, 'validation': {'checked': True, 'executed': False, 'status': 'bound'}})
        else:
            rejected.append({**record, 'validation': {'checked': True, 'executed': False, 'rejected': reason}})
    return accepted, rejected, catalog


def in_validation_split(identifier: str) -> bool:
    """One pair in ten goes to the validation split, chosen by the identifier of the pair."""
    return int(identifier[:2], 16) < 26


def chat_records(pairs: list[dict], system_message: str) -> tuple[list[dict], list[dict]]:
    train, validation = [], []
    for pair in pairs:
        record = {
            'messages': [
                {'role': 'system', 'content': system_message},
                {'role': 'user', 'content': INPUT_PROMPTS[pair['input_kind']].format(input=pair['input'])},
                {'role': 'assistant', 'content': pair['program']},
            ],
        }
        (validation if in_validation_split(pair['id']) else train).append(record)
    return train, validation


def tool_call_records(pairs: list[dict]) -> tuple[list[dict], list[dict]]:
    train, validation = [], []
    for pair in pairs:
        record = {
            'messages': [
                {'role': 'user', 'content': INPUT_PROMPTS[pair['input_kind']].format(input=pair['input'])},
                {
                    'role': 'assistant',
                    'tool_calls': [
                        {
                            'id': f'call_{pair["id"]}',
                            'type': 'function',
                            'function': {'name': 'nurt', 'arguments': json.dumps({'program': pair['program']}, ensure_ascii=False)},
                        },
                    ],
                },
            ],
        }
        (validation if in_validation_split(pair['id']) else train).append(record)
    return train, validation


def build_tools(nurt: str, directory: pathlib.Path) -> None:
    for language in TOOL_LANGUAGES:
        for name in TOOL_FORMATS:
            text = subprocess.run([nurt, 'schema', 'tool', '--format', name, '--language', language], check=True, capture_output=True, text=True, cwd=tempfile.gettempdir()).stdout
            target = directory / language / f'nurt-tool.{name}.json'
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_text(text, encoding='utf-8')


def corpus_records() -> list[dict]:
    records = []
    for directory, section in CORPUS_SOURCES:
        for file in sorted((REPOSITORY / directory).glob('*.md')):
            if file.name.startswith('_'):
                continue
            text = FRONT_MATTER.sub('', file.read_text(encoding='utf-8'))
            text = HEADER_COMMENT.sub('', text).strip()
            heading = re.search(r'^# (.+)$', text, re.M)
            records.append({
                'id': f'{section}/{file.stem}',
                'path': str(file.relative_to(REPOSITORY)),
                'section': section,
                'title': heading.group(1).strip('` ') if heading else file.stem,
                'text': text,
            })
    return records


def describe(directory: pathlib.Path) -> list[dict]:
    files = []
    for file in sorted(directory.rglob('*')):
        if not file.is_file():
            continue
        data = file.read_bytes()
        entry = {'path': str(file.relative_to(TARGET)), 'bytes': len(data), 'sha256': hashlib.sha256(data).hexdigest()}
        if file.suffix == '.jsonl':
            entry['records'] = data.count(b'\n')
        files.append(entry)
    return files


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--nurt', required=True, help='the executable nurt of the release')
    parser.add_argument('--nurt-bench', required=True, help='the executable nurt-bench of the release')
    parser.add_argument('--date', default=datetime.date.today().isoformat(), help='the date written into the manifest')
    arguments = parser.parse_args()

    version = subprocess.run([arguments.nurt, '--version'], check=True, capture_output=True, text=True).stdout.splitlines()[0].split()[-1]
    for name in ('pairs', 'chat', 'tool-calls', 'tools', 'corpus'):
        shutil.rmtree(TARGET / name, ignore_errors=True)

    with tempfile.TemporaryDirectory() as temporary:
        records = candidates(arguments.nurt_bench, pathlib.Path(temporary))
    checker = Checker(arguments.nurt)
    accepted, rejected, catalog = build_pairs(records, checker)
    checker.close()

    sources = sorted({record['source'] for record in records})
    summary = {
        'format': 'nurt-bench-pair/2',
        'nurt': version,
        'catalog_rev': catalog,
        'validation': 'check',
        'sources': [
            {
                'source': source,
                'candidates': sum(1 for record in records if record['source'] == source),
                'accepted': sum(1 for record in accepted if record['source'] == source),
                'rejected': sum(1 for record in rejected if record['source'] == source),
            }
            for source in sources
        ],
        'accepted': len(accepted),
        'rejected': len(rejected),
    }
    write_lines(TARGET / 'pairs/pairs.jsonl', accepted)
    write_lines(TARGET / 'pairs/rejected.jsonl', rejected)
    write_json(TARGET / 'pairs/summary.json', summary)
    shutil.copy(REPOSITORY / 'bench/data/DATASHEET.md', TARGET / 'pairs/DATASHEET.md')
    shutil.copy(REPOSITORY / 'bench/data/LICENSE', TARGET / 'pairs/LICENSE')
    shutil.copy(REPOSITORY / 'bench/protocol/schema/nurt-bench-pair-2.schema.json', TARGET / 'pairs/nurt-bench-pair-2.schema.json')

    for name, builder in (('chat', lambda: chat_records(accepted, SYSTEM_MESSAGE)), ('tool-calls', lambda: tool_call_records(accepted))):
        train, validation = builder()
        write_lines(TARGET / name / 'train.jsonl', train)
        write_lines(TARGET / name / 'validation.jsonl', validation)
        shutil.copy(REPOSITORY / 'bench/data/LICENSE', TARGET / name / 'LICENSE')
    tool = subprocess.run([arguments.nurt, 'schema', 'tool', '--format', 'openai'], check=True, capture_output=True, text=True, cwd=tempfile.gettempdir()).stdout
    write_json(TARGET / 'tool-calls/tools.json', [json.loads(tool)])

    build_tools(arguments.nurt, TARGET / 'tools')
    shutil.copy(REPOSITORY / 'LICENSE', TARGET / 'tools/LICENSE')

    write_lines(TARGET / 'corpus/docs.jsonl', corpus_records())
    shutil.copy(REPOSITORY / 'LICENSE', TARGET / 'corpus/LICENSE')

    licenses = {'pairs': 'CC-BY-4.0', 'chat': 'CC-BY-4.0', 'tool-calls': 'CC-BY-4.0', 'tools': 'Apache-2.0', 'corpus': 'Apache-2.0'}
    manifest = {
        'format': 'nurt-training-packs/1',
        'nurt': version,
        'catalog_rev': catalog,
        'date': arguments.date,
        'packs': [{'name': name, 'license': licenses[name], 'files': describe(TARGET / name)} for name in licenses],
        'pairs': summary,
    }
    write_json(TARGET / 'manifest.json', manifest)
    for source in summary['sources']:
        print(f'{source["source"]}: {source["candidates"]} candidates, {source["accepted"]} accepted, {source["rejected"]} rejected')
    print(f'accepted pairs: {summary["accepted"]}, corpus documents: {len(corpus_records())}')
    return 0


if __name__ == '__main__':
    sys.exit(main())
