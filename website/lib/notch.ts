import type { Shot } from "./site";

/** Width of a stage that holds every shot, CSS px. */
export const stageWidth = (shots: Shot[]) => Math.max(...shots.map((s) => s.w));
export const stageHeight = (shots: Shot[]) => Math.max(...shots.map((s) => s.h));

/** Top-centre placement of a shot inside a stage, in % so the stage can scale. */
export function placement(shot: Shot, stageW: number) {
  return {
    left: `${((stageW - shot.w) / 2 / stageW) * 100}%`,
    width: `${(shot.w / stageW) * 100}%`,
  };
}

const pct = (v: number, of: number) => `${((v / of) * 100).toFixed(3)}%`;

/**
 * clip-path that shows only the part of `shot` covered by `target`'s notch
 * body (both top-centred on the same stage). Tweening between these strings
 * is how one real capture grows or shrinks into another, anchored at the top
 * edge like the app's own shell. Every string has the same shape, so GSAP can
 * interpolate it from whatever value is on screen (interruptible).
 */
export function clipToward(shot: Shot, target: Shot, stageW: number) {
  const shotLeft = (stageW - shot.w) / 2;
  const targetLeft = (stageW - target.w) / 2;
  const l = Math.min(Math.max(targetLeft + target.body[0] - shotLeft, 0), shot.w);
  const r = Math.min(Math.max(shotLeft + shot.w - (targetLeft + target.body[2]), 0), shot.w);
  const b = Math.min(Math.max(shot.h - target.body[3], 0), shot.h);
  const rad = target.radius;
  return `inset(0% ${pct(r, shot.w)} ${pct(b, shot.h)} ${pct(l, shot.w)} round 0px 0px ${rad}px ${rad}px)`;
}

/** The whole shot, shadow included. */
export const clipFull = () => "inset(0% 0.000% 0.000% 0.000% round 0px 0px 0px 0px)";
