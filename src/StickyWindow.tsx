import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import "./StickyWindow.css";

type Note = {
  id: string;
  title: string;
  content: string;
  color: string;
  noteType: "note" | "snippet" | "todo";
  pinned: boolean;
  createdAt: number;
  updatedAt: number;
};

type Props = {
  noteId: string;
};

export default function StickyWindow({ noteId }: Props) {
  const [note, setNote] = useState<Note | null>(null);
  const [error, setError] = useState("");
  const [alwaysOnTop, setAlwaysOnTop] = useState(true);

  useEffect(() => {
    void loadNote();
  }, [noteId]);

  async function loadNote() {
    try {
      setError("");
      const loaded = await invoke<Note>("get_note", { id: noteId });
      setNote(loaded);
    } catch (cause) {
      setError(toMessage(cause));
    }
  }

  async function toggleAlwaysOnTop() {
    try {
      const next = !alwaysOnTop;
      await getCurrentWindow().setAlwaysOnTop(next);
      setAlwaysOnTop(next);
    } catch (cause) {
      setError(toMessage(cause));
    }
  }

  async function closeWindow() {
    await getCurrentWindow().close();
  }

  if (error) {
    return (
      <main className="sticky-shell sticky-error">
        <strong>StickyFlow</strong>
        <p>{error}</p>
        <button onClick={() => void closeWindow()} type="button">
          Close
        </button>
      </main>
    );
  }

  if (!note) {
    return <main className="sticky-shell">Decrypting note…</main>;
  }

  return (
    <main className={`sticky-shell sticky-${note.color}`}>
      <header className="sticky-header">
        <div>
          <span>{note.noteType}</span>
          <h1>{note.title}</h1>
        </div>

        <div className="sticky-actions">
          <button onClick={() => void toggleAlwaysOnTop()} type="button">
            {alwaysOnTop ? "Top ✓" : "Top"}
          </button>
          <button onClick={() => void closeWindow()} type="button">
            ×
          </button>
        </div>
      </header>

      <section className="sticky-content">
        {note.content || <em>Empty note</em>}
      </section>
    </main>
  );
}

function toMessage(cause: unknown) {
  return cause instanceof Error ? cause.message : String(cause);
}
