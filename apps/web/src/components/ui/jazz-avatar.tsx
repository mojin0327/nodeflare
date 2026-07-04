'use client';

import Jazzicon from 'react-jazzicon';

// FNV-1a hash → unsigned 32-bit int, used as the deterministic Jazzicon seed
// (react-jazzicon seeds a Mersenne-Twister with a number).
function numFromSeed(seed: string): number {
  let h = 2166136261;
  for (let i = 0; i < seed.length; i++) {
    h ^= seed.charCodeAt(i);
    h = Math.imul(h, 16777619);
  }
  return h >>> 0;
}

/** Jazzicon avatar keyed deterministically off a string seed (e.g. server id). */
export function JazzAvatar({ seed, diameter }: { seed: string; diameter: number }) {
  return <Jazzicon diameter={diameter} seed={numFromSeed(seed)} />;
}
