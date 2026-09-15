import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import StickyWindow from "./StickyWindow";
import "./Notes.css";

const stickyNoteId = new URLSearchParams(window.location.search).get("sticky");

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    {stickyNoteId ? <StickyWindow noteId={stickyNoteId} /> : <App />}
  </React.StrictMode>,
);
