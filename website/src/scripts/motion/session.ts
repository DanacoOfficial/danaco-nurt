// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           website.src.scripts.motion.session
// Responsibility:   Plays the terminal session of the home page: the input lines are typed character by character, the output lines appear one by one, and the play starts when the window enters the view and stops while it is out of view. The button of the window plays the session again.
// Dependencies:     none.

const typingDelay = 26;
const outputDelay = 70;
const lineStartDelay = 520;
const pauseDelay = 1500;

interface TypedSegment {
  element: HTMLElement;
  text: string;
}

function wait(milliseconds: number, signal: AbortSignal): Promise<void> {
  return new Promise((resolve, reject) => {
    const timer = window.setTimeout(resolve, milliseconds);
    signal.addEventListener('abort', () => {
      window.clearTimeout(timer);
      reject(new DOMException('stopped', 'AbortError'));
    });
  });
}

class SessionPlayer {
  private readonly lines: HTMLElement[];
  private readonly texts: Map<HTMLElement, string>;
  private readonly replay: HTMLButtonElement | null;
  private controller: AbortController | null = null;
  private played = false;

  constructor(private readonly root: HTMLElement) {
    this.lines = Array.from(root.querySelectorAll<HTMLElement>('.session__line'));
    this.texts = new Map();
    for (const segment of root.querySelectorAll<HTMLElement>('[data-tone]')) {
      this.texts.set(segment, segment.textContent ?? '');
    }
    this.replay = root.querySelector<HTMLButtonElement>('[data-session-replay]');
    this.replay?.addEventListener('click', () => this.play());
  }

  /** Marks the step of the explanation beside the window that the shown line illustrates. */
  private mark(line: HTMLElement): void {
    const step = line.dataset.step;
    if (step === undefined) {
      return;
    }
    const scope = this.root.closest('section') ?? document;
    scope.querySelectorAll<HTMLElement>('.how__point').forEach((point, index) => {
      point.classList.toggle('is-active', String(index) === step);
    });
  }

  private reset(): void {
    this.root.classList.add('is-playing');
    for (const line of this.lines) {
      line.classList.remove('is-shown', 'is-typing');
    }
    for (const [segment, text] of this.texts) {
      if (segment.closest('.session__line--input') && segment.dataset.tone === 'input') {
        segment.textContent = '';
      } else {
        segment.textContent = text;
      }
    }
  }

  private finish(): void {
    for (const [segment, text] of this.texts) {
      segment.textContent = text;
    }
    for (const line of this.lines) {
      line.classList.add('is-shown');
      line.classList.remove('is-typing');
    }
    this.root.classList.remove('is-playing');
    if (this.replay) {
      this.replay.hidden = false;
    }
  }

  private async run(signal: AbortSignal): Promise<void> {
    this.reset();
    for (const line of this.lines) {
      const kind = line.dataset.kind;
      if (kind === 'pause') {
        line.classList.add('is-shown');
        await wait(pauseDelay, signal);
        continue;
      }
      this.mark(line);
      if (kind === 'input') {
        line.classList.add('is-shown', 'is-typing');
        await wait(lineStartDelay, signal);
        const typed: TypedSegment[] = Array.from(line.querySelectorAll<HTMLElement>('[data-tone="input"]')).map((element) => ({ element, text: this.texts.get(element) ?? '' }));
        for (const segment of typed) {
          for (let index = 1; index <= segment.text.length; index += 1) {
            segment.element.textContent = segment.text.slice(0, index);
            await wait(typingDelay + (Math.random() * typingDelay) / 2, signal);
          }
        }
        await wait(260, signal);
        line.classList.remove('is-typing');
        continue;
      }
      line.classList.add('is-shown');
      await wait(outputDelay, signal);
    }
    this.finish();
  }

  play(): void {
    this.controller?.abort();
    const controller = new AbortController();
    this.controller = controller;
    this.played = true;
    if (this.replay) {
      this.replay.hidden = true;
    }
    this.run(controller.signal).catch(() => undefined);
  }

  stop(): void {
    if (this.controller) {
      this.controller.abort();
      this.controller = null;
      this.finish();
    }
  }

  get hasPlayed(): boolean {
    return this.played;
  }
}

export function startSessions(): void {
  const roots = Array.from(document.querySelectorAll<HTMLElement>('[data-session]'));
  if (roots.length === 0 || !('IntersectionObserver' in window)) {
    return;
  }
  for (const root of roots) {
    const player = new SessionPlayer(root);
    const observer = new IntersectionObserver(
      (entries) => {
        for (const entry of entries) {
          if (entry.isIntersecting && !player.hasPlayed) {
            player.play();
          } else if (!entry.isIntersecting) {
            player.stop();
          }
        }
      },
      { threshold: 0.35 },
    );
    observer.observe(root);
  }
}
