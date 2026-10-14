"use client";

import gsap from "gsap";
import { ScrollTrigger } from "gsap/ScrollTrigger";
import { useGSAP } from "@gsap/react";

gsap.registerPlugin(useGSAP, ScrollTrigger);

// One motion vocabulary for the whole page. Shell changes settle like the
// app's critically damped springs: fast start, long soft landing, no overshoot.
gsap.defaults({ ease: "power3.out", duration: 0.7 });
export const SHELL_EASE = "power4.out";
export const SHELL_DURATION = 0.85;

export const MOTION_OK = "(prefers-reduced-motion: no-preference)";

export { gsap, ScrollTrigger, useGSAP };
