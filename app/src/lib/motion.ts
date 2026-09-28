// Motion tokens for `motion/react`, mirroring the CSS duration and easing tokens.
import type { Transition } from "motion/react";

export const ease = {
  standard: [0.2, 0, 0, 1] as const,
  outExpo: [0.16, 1, 0.3, 1] as const,
};

export const transitions = {
  fast: { duration: 0.14, ease: ease.standard },
  base: { duration: 0.22, ease: ease.standard },
  slow: { duration: 0.36, ease: ease.outExpo },
  spring: { type: "spring", stiffness: 520, damping: 40, mass: 0.9 },
  softSpring: { type: "spring", stiffness: 300, damping: 32 },
} satisfies Record<string, Transition>;

/** Page enter: a short rise and fade. */
export const pageVariants = {
  initial: { opacity: 0, y: 6 },
  animate: { opacity: 1, y: 0, transition: transitions.slow },
};

/** List items entering and leaving. */
export const listItemVariants = {
  initial: { opacity: 0, y: 8, scale: 0.985 },
  animate: { opacity: 1, y: 0, scale: 1, transition: transitions.slow },
  exit: { opacity: 0, scale: 0.97, transition: transitions.fast },
};
