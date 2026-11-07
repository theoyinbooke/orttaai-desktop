import React from "react";
import ReactDOM from "react-dom/client";
import { getCurrentWindow } from "@tauri-apps/api/window";
import "@fontsource-variable/inter";
import "@fontsource-variable/source-serif-4";
import "@fontsource-variable/jetbrains-mono";
import App from "./App";
import Panel from "./Panel";
import SetupWindow from "./views/Setup";
import { applyTheme, storedTheme } from "./ui";
import "./App.css";

// Apply the saved theme before first paint to avoid a flash.
applyTheme(storedTheme());

// Every window loads index.html: the floating panel renders a minimal overlay,
// and the setup window is the first-run wizard on its own.
const label = getCurrentWindow().label;
const isPanel = label === "panel";
if (isPanel) document.body.classList.add("panel-window");

// No webview "Back / Reload / Inspect Element" menu in the shipped app; text
// fields keep theirs for copy and paste.
if (!import.meta.env.DEV) {
  document.addEventListener("contextmenu", (e) => {
    const el = e.target as HTMLElement | null;
    if (!el?.closest?.("input, textarea, [contenteditable='true']")) e.preventDefault();
  });
}

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>{isPanel ? <Panel /> : label === "setup" ? <SetupWindow /> : <App />}</React.StrictMode>,
);
