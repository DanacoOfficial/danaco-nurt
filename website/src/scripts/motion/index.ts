// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           website.src.scripts.motion
// Responsibility:   Starts the motion layer of the site when the document is marked for motion: entrances, counters, tilting cards and the terminal session. A reader who asks for reduced motion receives none of it.
// Dependencies:     website.src.scripts.motion.reveal, website.src.scripts.motion.counter, website.src.scripts.motion.tilt, website.src.scripts.motion.session

import { startCounters } from './counter';
import { startReveal } from './reveal';
import { startSessions } from './session';
import { startTilt } from './tilt';

export function startMotion(): void {
  const root = document.documentElement;
  if (!root.classList.contains('motion')) {
    return;
  }
  root.setAttribute('data-motion-ready', '');
  startReveal();
  startCounters();
  startTilt();
  startSessions();
}
