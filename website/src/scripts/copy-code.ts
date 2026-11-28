// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           website.src.scripts.copy-code
// Responsibility:   Adds a copy button to every code block of a page and copies the text of the block to the clipboard.
// Dependencies:     none.

function label(name: 'copy' | 'copied' | 'failed'): string {
  const root = document.documentElement;
  const map = { copy: root.dataset.labelCopy, copied: root.dataset.labelCopied, failed: root.dataset.labelCopyFailed };
  return map[name] ?? name;
}

function attach(block: HTMLElement): void {
  if (block.querySelector('.copy-button')) {
    return;
  }
  const button = document.createElement('button');
  button.type = 'button';
  button.className = 'copy-button';
  button.textContent = label('copy');
  button.addEventListener('click', async () => {
    const code = block.querySelector('code');
    const text = (code ?? block).textContent ?? '';
    try {
      await navigator.clipboard.writeText(text.replace(/\n$/, ''));
      button.textContent = label('copied');
    } catch (error) {
      button.textContent = label('failed');
    }
    window.setTimeout(() => {
      button.textContent = label('copy');
    }, 1800);
  });
  block.append(button);
}

document.querySelectorAll<HTMLElement>('.prose pre, .code-block pre, .terminal pre').forEach((pre) => {
  pre.classList.add('has-copy');
  attach(pre);
});
