import React from "react";
import ReactDOM from "react-dom/client";
import { getCurrentWindow } from "@tauri-apps/api/window";
import "@fontsource-variable/bricolage-grotesque";
import "@fontsource-variable/jetbrains-mono";
import App from "./App";
import Panel from "./Panel";
import { applyTheme, storedTheme } from "./ui";
import "./App.css";

// Apply the saved theme before first paint to avoid a flash.
applyTheme(storedTheme());

// Both windows load index.html; the floating panel renders a minimal overlay.
const isPanel = getCurrentWindow().label === "panel";
if (isPanel) document.body.classList.add("panel-window");

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>{isPanel ? <Panel /> : <App />}</React.StrictMode>,
);
