import Lenis from "lenis";
import { useEffect } from "react";

/**
 * Inertial scrolling.
 *
 * The wheel moves a target and the page eases toward it every frame, which is
 * what separates a site that feels expensive from one that jumps a notch per
 * click. Lenis still scrolls the real window, so anchors, the scrollbar and
 * every scroll listener on the page keep working.
 */
let current: Lenis | null = null;

export function useSmoothScroll(enabled = true) {
  useEffect(() => {
    if (!enabled) return;
    if (window.matchMedia("(prefers-reduced-motion: reduce)").matches) return;

    const lenis = new Lenis({
      // Long enough to glide, short enough that the page still obeys you.
      duration: 1.25,
      easing: (t) => Math.min(1, 1.001 - Math.pow(2, -10 * t)),
      wheelMultiplier: 0.95,
      touchMultiplier: 1.7,
      smoothWheel: true,
    });

    current = lenis;
    document.documentElement.classList.add("lenis");

    let frame = 0;
    const tick = (time: number) => {
      lenis.raf(time);
      frame = requestAnimationFrame(tick);
    };
    frame = requestAnimationFrame(tick);

    return () => {
      cancelAnimationFrame(frame);
      lenis.destroy();
      document.documentElement.classList.remove("lenis");
      current = null;
    };
  }, [enabled]);
}

/** Eases to an element or an offset, through Lenis when it is running. */
export function glideTo(target: HTMLElement | number, offset = 0) {
  if (current) {
    current.scrollTo(target, { offset, duration: 1.5 });
    return;
  }
  if (typeof target === "number") {
    window.scrollTo({ top: target + offset, behavior: "smooth" });
  } else {
    target.scrollIntoView({ behavior: "smooth", block: "start" });
  }
}

/** Jumps without easing. Used when a whole screen is replaced. */
export function jumpToTop() {
  current?.scrollTo(0, { immediate: true });
  window.scrollTo({ top: 0, behavior: "instant" as ScrollBehavior });
}
