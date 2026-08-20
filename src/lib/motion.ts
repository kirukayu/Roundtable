import { useEffect, useRef, useState } from "react";

/**
 * The handful of scroll and pointer readings that are not worth a Motion value.
 *
 * Reveals and parallax live in the components now — Motion's `whileInView` and
 * `useScroll` do both better than a hand-rolled observer could. What is left
 * here writes CSS variables or a boolean, and stays out of React's render path.
 */

/** True while the page is scrolled past `after` pixels. */
export function useScrolled(after = 40) {
  const [past, setPast] = useState(false);

  useEffect(() => {
    let frame = 0;
    const onScroll = () => {
      cancelAnimationFrame(frame);
      frame = requestAnimationFrame(() => setPast(window.scrollY > after));
    };
    onScroll();
    window.addEventListener("scroll", onScroll, { passive: true });
    return () => {
      cancelAnimationFrame(frame);
      window.removeEventListener("scroll", onScroll);
    };
  }, [after]);

  return past;
}

/**
 * Writes how far the page has been read into a CSS variable on the element.
 *
 * A number rather than a width, so the rail can be scaled on the compositor
 * instead of being laid out again on every frame.
 */
export function useProgress<T extends HTMLElement>() {
  const ref = useRef<T>(null);

  useEffect(() => {
    const element = ref.current;
    if (!element) return;

    let frame = 0;
    const update = () => {
      cancelAnimationFrame(frame);
      frame = requestAnimationFrame(() => {
        const scrollable = document.documentElement.scrollHeight - window.innerHeight;
        const ratio = scrollable > 0 ? window.scrollY / scrollable : 0;
        element.style.setProperty("--p", String(Math.min(1, Math.max(0, ratio))));
      });
    };

    update();
    window.addEventListener("scroll", update, { passive: true });
    window.addEventListener("resize", update);
    return () => {
      cancelAnimationFrame(frame);
      window.removeEventListener("scroll", update);
      window.removeEventListener("resize", update);
    };
  }, []);

  return ref;
}

/**
 * Tracks the pointer inside an element as two percentage variables.
 *
 * Used for the faint sheen that follows the cursor across a poster. It only
 * listens while the pointer is actually over the element.
 */
export function usePointer<T extends HTMLElement>() {
  const ref = useRef<T>(null);

  useEffect(() => {
    const element = ref.current;
    if (!element) return;
    if (document.documentElement.dataset.reduceMotion === "true") return;

    let frame = 0;
    const onMove = (event: PointerEvent) => {
      cancelAnimationFrame(frame);
      frame = requestAnimationFrame(() => {
        const box = element.getBoundingClientRect();
        element.style.setProperty("--mx", `${((event.clientX - box.left) / box.width) * 100}%`);
        element.style.setProperty("--my", `${((event.clientY - box.top) / box.height) * 100}%`);
      });
    };

    const onLeave = () => {
      cancelAnimationFrame(frame);
      element.style.setProperty("--mx", "50%");
      element.style.setProperty("--my", "50%");
    };

    element.addEventListener("pointermove", onMove);
    element.addEventListener("pointerleave", onLeave);
    return () => {
      cancelAnimationFrame(frame);
      element.removeEventListener("pointermove", onMove);
      element.removeEventListener("pointerleave", onLeave);
    };
  }, []);

  return ref;
}
