import React from "react";
import ReactDOM from "react-dom/client";

import App from "./App";
import { AppProvider } from "./lib/store";
import { ToastProvider } from "./components/ui";
import "./styles/global.css";

// StrictMode double-invokes effects in development, which would double every
// backend probe. The app is IO-heavy at startup, so it is left off.

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <ToastProvider>
      <AppProvider>
        <App />
      </AppProvider>
    </ToastProvider>
  </React.StrictMode>,
);
