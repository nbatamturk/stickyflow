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

  useEffect(() => {
    const currentWindow = getCurrentWindow();
    let unlistenMoved: (() => void) | undefined;
    let unlistenResized: (() => void) | undefined;
    let saveTimer: ReturnType<typeof setTimeout> | undefined;
    let disposed = false;

    const scheduleSave = () => {
      if (saveTimer) {
        clearTimeout(saveTimer);
      }

      saveTimer = setTimeout(() => {
        void invoke("save_sticky_window_state");
      }, 150);
    };

    void (async () => {
      const moved = await currentWindow.onMoved(scheduleSave);
      if (disposed) {
        moved();
      } else {
        unlistenMoved = moved;
      }

      const resized = await currentWindow.onResized(scheduleSave);
      if (disposed) {
        resized();
      } else {
        unlistenResized = resized;
      }
    })();

    return () => {
      disposed = true;
      if (saveTimer) {
        clearTimeout(saveTimer);
      }
      unlistenMoved?.();
      unlistenResized?.();
    };
  }, []);


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
    try {
      await invoke("close_sticky_window", { id: noteId });
    } catch (cause) {
      setError(toMessage(cause));
    }
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
