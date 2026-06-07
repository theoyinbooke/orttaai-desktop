import React from "react";
import ReactDOM from "react-dom/client";
import { getCurrentWindow } from "@tauri-apps/api/window";
import App from "./App";
import Panel from "./Panel";

// Both windows load index.html; the floating panel renders a minimal overlay.
const isPanel = getCurrentWindow().label === "panel";
if (isPanel) document.body.classList.add("panel-window");

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>{isPanel ? <Panel /> : <App />}</React.StrictMode>,
);
