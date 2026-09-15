import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { LogicalSize } from "@tauri-apps/api/dpi";
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
  mode: "expanded" | "chip";
};

export default function StickyWindow({ noteId, mode }: Props) {
  const [note, setNote] = useState<Note | null>(null);
  const [error, setError] = useState("");
  const [alwaysOnTop, setAlwaysOnTop] = useState(true);
  const chipTitleRef = useRef<HTMLElement | null>(null);

  useEffect(() => {
    void loadNote();
  }, [noteId]);

  useEffect(() => {
    const currentWindow = getCurrentWindow();
    let unlistenMoved: (() => void) | undefined;
    let unlistenResized: (() => void) | undefined;
    let timer: ReturnType<typeof setTimeout> | undefined;
    let disposed = false;

    const saveGeometry = () => {
      if (timer) {
        clearTimeout(timer);
      }

      timer = setTimeout(() => {
        void invoke("save_sticky_geometry", {
          id: noteId,
          mode,
        });
      }, 250);
    };

    void (async () => {
      const moved = await currentWindow.onMoved(saveGeometry);

      if (disposed) {
        moved();
      } else {
        unlistenMoved = moved;
      }

      if (mode === "expanded") {
        const resized = await currentWindow.onResized(saveGeometry);

        if (disposed) {
          resized();
        } else {
          unlistenResized = resized;
        }
      }
    })();

    return () => {
      disposed = true;

      if (timer) {
        clearTimeout(timer);
      }

      unlistenMoved?.();
      unlistenResized?.();
    };
  }, [noteId, mode]);

  useLayoutEffect(() => {
    if (mode !== "chip" || !note || !chipTitleRef.current) {
      return;
    }

    const frame = requestAnimationFrame(() => {
      const titleWidth =
        chipTitleRef.current?.getBoundingClientRect().width ?? 0;

      // drag handle + paddings + chevron + borders
      const chromeWidth = 40;

      const width = Math.max(
        64,
        Math.min(200, Math.ceil(titleWidth + chromeWidth)),
      );

      void (async () => {
        const size = new LogicalSize(width, 28);

        // On Linux/Wry the embedded webview can retain its own default
        // 200x200 bounds. Shrink the webview first, then its native window.
        await getCurrentWebview().setSize(size);
        await getCurrentWindow().setSize(size);
      })();
    });

    return () => cancelAnimationFrame(frame);
  }, [mode, note?.title]);

  async function loadNote() {
    try {
      setError("");
      const loaded = await invoke<Note>("get_note", { id: noteId });
      setNote(loaded);
    } catch (cause) {
      setError(toMessage(cause));
    }
  }

  async function switchMode(compact: boolean) {
    try {
      await invoke("set_sticky_compact", {
        id: noteId,
        compact,
      });
    } catch (cause) {
      setError(toMessage(cause));
    }
  }

  async function startDragging() {
    try {
      await getCurrentWindow().startDragging();
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
      </main>
    );
  }

  if (!note) {
    return <main className="sticky-shell">Decrypting note…</main>;
  }

  if (mode === "chip") {
    return (
      <main className={`sticky-chip sticky-${note.color}`}>
        <button
          className="sticky-chip-drag"
          onMouseDown={() => void startDragging()}
          title="Move sticky"
          type="button"
        >
          ⋮
        </button>

        <button
          className="sticky-chip-open"
          onClick={() => void switchMode(false)}
          title={`Open ${note.title}`}
          type="button"
        >
          <strong ref={chipTitleRef}>{note.title}</strong>
          <span>›</span>
        </button>
      </main>
    );
  }

  return (
    <main className={`sticky-shell sticky-${note.color}`}>
      <header className="sticky-header">
        <div>
          <span>{note.noteType}</span>
          <h1>{note.title}</h1>
        </div>

        <div className="sticky-actions">
          <button
            onClick={() => void switchMode(true)}
            title="Collapse"
            type="button"
          >
            ▂
          </button>

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
