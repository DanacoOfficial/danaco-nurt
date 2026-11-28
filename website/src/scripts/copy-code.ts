// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           website.src.scripts.copy-code
// Responsibility:   Adds a copy button to every code block of a page and copies the text of the block to the clipboard: through the clipboard interface of the browser and, where the browser refuses it, through a selection in a hidden field.
// Dependencies:     none.

function label(name: 'copy' | 'copied' | 'failed'): string {
  const root = document.documentElement;
  const map = { copy: root.dataset.labelCopy, copied: root.dataset.labelCopied, failed: root.dataset.labelCopyFailed };
  return map[name] ?? name;
}

/** Copies a text through a selection in a field outside the view; the way for a browser that refuses the clipboard interface. */
function copyBySelection(text: string): boolean {
  const field = document.createElement('textarea');
  field.value = text;
  field.setAttribute('readonly', '');
  field.style.position = 'fixed';
  field.style.top = '-100vh';
  document.body.append(field);
  field.select();
  let copied = false;
  try {
    copied = document.execCommand('copy');
  } catch {
    copied = false;
  }
  field.remove();
  return copied;
}

/** The text of a block without the label of its copy button. */
function blockText(block: HTMLElement): string {
  const code = block.querySelector('code');
  if (code) {
    return code.textContent ?? '';
  }
  const copy = block.cloneNode(true) as HTMLElement;
  copy.querySelectorAll('.copy-button').forEach((button) => button.remove());
  return copy.textContent ?? '';
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
    const text = blockText(block).replace(/\n$/, '');
    let copied = false;
    try {
      await navigator.clipboard.writeText(text);
      copied = true;
    } catch {
      copied = copyBySelection(text);
    }
    button.textContent = label(copied ? 'copied' : 'failed');
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
