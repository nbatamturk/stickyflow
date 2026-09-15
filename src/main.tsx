import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import StickyWindow from "./StickyWindow";
import "./Notes.css";

const params = new URLSearchParams(window.location.search);
const stickyNoteId = params.get("sticky");
const stickyMode =
  params.get("mode") === "chip" ? "chip" : "expanded";

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    {stickyNoteId ? (
      <StickyWindow noteId={stickyNoteId} mode={stickyMode} />
    ) : (
      <App />
    )}
  </React.StrictMode>,
);
