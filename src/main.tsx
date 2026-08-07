import React from "react";
import ReactDOM from "react-dom/client";

import App from "./App";
import { Overlay } from "./pages/Overlay";
import { AppProvider } from "./lib/store";
import { ToastProvider } from "./components/ui";
import "./styles/global.css";

// StrictMode double-invokes effects in development, which would double every
// backend probe. The app is IO-heavy at startup, so it is left off.

/*
 * Two entries, one bundle.
 *
 * The launcher and the in-game overlay are the same build served from the same
 * local address, told apart by the fragment. That is deliberate: the overlay
 * inherits the stylesheet, the fonts and the motion of the launcher without a
 * second pipeline to keep in step, which is the only way it can plausibly look
 * like it belongs to the same program.
 *
 * The overlay does not mount the app's provider tree — no fog, no catalogue, no
 * settings poll. It is a window that appears over a game, and it should cost
 * nothing while the game is what matters.
 */
const isOverlay = window.location.hash.startsWith("#/overlay");

// The overlay's window is transparent; the page inside it was not. `body`
// carries the app's background, so a black rectangle was painted over the game
// and the rounded column sat on top of it with its corners showing.
if (isOverlay) document.documentElement.classList.add("overlay-window");

const root = ReactDOM.createRoot(document.getElementById("root") as HTMLElement);

root.render(
  <React.StrictMode>
    {isOverlay ? (
      <ToastProvider>
        <Overlay />
      </ToastProvider>
    ) : (
      <ToastProvider>
        <AppProvider>
          <App />
        </AppProvider>
      </ToastProvider>
    )}
  </React.StrictMode>,
);
