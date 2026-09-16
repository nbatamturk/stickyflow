import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { writeText } from "@tauri-apps/plugin-clipboard-manager";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { LogicalSize } from "@tauri-apps/api/dpi";
import { parseSnippetParts } from "./snippetBlocks";
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
  const [editing, setEditing] = useState(false);
  const [editTitle, setEditTitle] = useState("");
  const [editContent, setEditContent] = useState("");
  const [saving, setSaving] = useState(false);
  const [opacity, setOpacity] = useState(100);
  const [opacityOpen, setOpacityOpen] = useState(false);
  const [copied, setCopied] = useState(false);
  const [copiedBlock, setCopiedBlock] = useState<number | null>(null);
  const chipTitleRef = useRef<HTMLElement | null>(null);
  const titleInputRef = useRef<HTMLInputElement | null>(null);

  useEffect(() => {
    void loadNote();
  }, [noteId]);

  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;

    void listen<string>(
      "stickyflow-note-updated",
      (event) => {
        if (
          event.payload === noteId &&
          !editing
        ) {
          void loadNote();
        }
      },
    ).then((stop) => {
      if (disposed) {
        stop();
      } else {
        unlisten = stop;
      }
    });

    return () => {
      disposed = true;
      unlisten?.();
    };
  }, [noteId, editing]);

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
      const [loaded, loadedOpacity] = await Promise.all([
        invoke<Note>("get_note", { id: noteId }),
        invoke<number>("get_sticky_opacity", { id: noteId }),
      ]);

      setNote(loaded);
      setOpacity(loadedOpacity);
      setEditTitle(loaded.title);

      try {
        await getCurrentWindow().setTitle(
          loaded.title,
        );
      } catch {
        // Visible note content is already synchronized.
      }
      setEditContent(loaded.content);
      setEditing(false);
    } catch (cause) {
      setError(toMessage(cause));
    }
  }

  async function startQuickEdit() {
    if (!note || mode !== "expanded") {
      return;
    }

    setError("");
    setOpacityOpen(false);
    setEditTitle(note.title);
    setEditContent(note.content);

    try {
      const currentWindow = getCurrentWindow();

      // Sticky windows normally reject keyboard focus.
      // Editing temporarily opts this one window into focus.
      await currentWindow.setFocusable(true);
      setEditing(true);
      await currentWindow.setFocus();

      requestAnimationFrame(() => {
        titleInputRef.current?.focus();
        titleInputRef.current?.select();
      });
    } catch (cause) {
      setEditing(false);
      setError(toMessage(cause));

      try {
        await getCurrentWindow().setFocusable(false);
      } catch {
        // Preserve the original error.
      }
    }
  }

  async function returnToFocuslessMode() {
    try {
      await getCurrentWindow().setFocusable(false);
    } catch (cause) {
      setError(toMessage(cause));
    }
  }

  async function cancelQuickEdit() {
    if (!note) {
      return;
    }

    setEditTitle(note.title);
    setEditContent(note.content);
    setEditing(false);

    await returnToFocuslessMode();
  }

  async function saveQuickEdit() {
    if (!note || saving) {
      return;
    }

    const title = editTitle.trim() || "Untitled";

    setSaving(true);
    setError("");

    try {
      const updated = await invoke<Note>("update_note", {
        input: {
          id: note.id,
          title,
          content: editContent,
          color: note.color,
          noteType: note.noteType,
          pinned: note.pinned,
        },
      });

      setNote(updated);
      setEditTitle(updated.title);
      setEditContent(updated.content);
      setEditing(false);

      await returnToFocuslessMode();
    } catch (cause) {
      setError(toMessage(cause));
    } finally {
      setSaving(false);
    }
  }

  async function copyFullSnippet() {
    if (!note || note.noteType !== "snippet") {
      return;
    }

    setError("");

    try {
      // Preserve multiline/code formatting exactly.
      await writeText(note.content);

      setCopied(true);

      window.setTimeout(() => {
        setCopied(false);
      }, 1400);
    } catch (cause) {
      setError(toMessage(cause));
    }
  }

  async function copySnippetBlock(
    content: string,
    blockIndex: number,
  ) {
    setError("");

    try {
      await writeText(content);
      setCopiedBlock(blockIndex);

      window.setTimeout(() => {
        setCopiedBlock((current) =>
          current === blockIndex ? null : current,
        );
      }, 1400);
    } catch (cause) {
      setError(toMessage(cause));
    }
  }

  async function changeOpacity(nextOpacity: number) {
    const safeOpacity = Math.max(
      40,
      Math.min(100, nextOpacity),
    );

    // Update the expanded React sticky immediately.
    setOpacity(safeOpacity);

    try {
      const saved = await invoke<number>(
        "set_sticky_opacity",
        {
          id: noteId,
          opacity: safeOpacity,
        },
      );

      setOpacity(saved);
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
      <main
        className={`sticky-chip sticky-${note.color}`}
      >
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
    <main
      className={`sticky-shell sticky-${note.color}`}
      onKeyDown={(event) => {
        if (!editing) {
          return;
        }

        if (event.key === "Escape") {
          event.preventDefault();
          void cancelQuickEdit();
          return;
        }

        if (
          event.key === "Enter" &&
          (event.ctrlKey || event.metaKey)
        ) {
          event.preventDefault();
          void saveQuickEdit();
        }
      }}
    >
      <header className="sticky-header">
        <div className="sticky-title-block">
          <span>{note.noteType}</span>

          {editing ? (
            <input
              className="sticky-edit-title"
              maxLength={200}
              onChange={(event) => {
                const value = event.currentTarget.value;
                setEditTitle(value);
              }}
              ref={titleInputRef}
              type="text"
              value={editTitle}
            />
          ) : (
            <h1>{note.title}</h1>
          )}
        </div>

        <div className="sticky-actions">
          {editing ? (
            <>
              <button
                className="sticky-save-button"
                disabled={saving}
                onClick={() => void saveQuickEdit()}
                title="Save (Ctrl+Enter)"
                type="button"
              >
                {saving ? "Saving…" : "Save"}
              </button>

              <button
                disabled={saving}
                onClick={() => void cancelQuickEdit()}
                title="Cancel (Esc)"
                type="button"
              >
                Cancel
              </button>
            </>
          ) : (
            <>
              {note.noteType === "snippet" && (
                <button
                  aria-label="Copy full snippet"
                  className={`sticky-icon-action ${
                    copied ? "is-success" : ""
                  }`}
                  onClick={() => void copyFullSnippet()}
                  title={copied ? "Copied ✓" : "Copy full snippet"}
                  type="button"
                >
                  {copied ? "✓" : "⧉"}
                </button>
              )}

              <button
                aria-label="Quick Edit"
                className="sticky-icon-action"
                onClick={() => void startQuickEdit()}
                title="Quick Edit"
                type="button"
              >
                ✎
              </button>

              <button
                aria-label={`Opacity ${opacity}%`}
                className={`sticky-icon-action ${
                  opacityOpen ? "is-active" : ""
                }`}
                onClick={() =>
                  setOpacityOpen((current) => !current)
                }
                title={`Opacity: ${opacity}%`}
                type="button"
              >
                ◐
              </button>

              <button
                aria-label="Collapse to chip"
                className="sticky-icon-action"
                onClick={() => void switchMode(true)}
                title="Collapse to chip"
                type="button"
              >
                ▂
              </button>

              <button
                aria-label={
                  alwaysOnTop
                    ? "Disable always on top"
                    : "Enable always on top"
                }
                className={`sticky-icon-action ${
                  alwaysOnTop ? "is-active" : ""
                }`}
                onClick={() => void toggleAlwaysOnTop()}
                title={
                  alwaysOnTop
                    ? "Always on top: On"
                    : "Always on top: Off"
                }
                type="button"
              >
                ↑
              </button>

              <button
                aria-label="Close sticky"
                className="sticky-icon-action sticky-close-action"
                onClick={() => void closeWindow()}
                title="Close sticky"
                type="button"
              >
                ×
              </button>
            </>
          )}
        </div>
      </header>

      {copied && !editing && (
        <div className="sticky-copy-status">
          Copied ✓
        </div>
      )}

      {opacityOpen && !editing && (
        <div className="sticky-opacity-panel">
          <span>Opacity</span>

          <input
            aria-label="Sticky opacity"
            max={100}
            min={40}
            onChange={(event) => {
              const value = Number(event.currentTarget.value);
              void changeOpacity(value);
            }}
            step={5}
            type="range"
            value={opacity}
          />

          <strong>{opacity}%</strong>
        </div>
      )}

      {editing ? (
        <section className="sticky-quick-edit">
          <textarea
            className="sticky-edit-content"
            onChange={(event) => {
              const value = event.currentTarget.value;
              setEditContent(value);
            }}
            spellCheck={false}
            value={editContent}
          />

          <div className="sticky-edit-hint">
            Ctrl+Enter save · Esc cancel
          </div>
        </section>
      ) : (
        <section
          className={`sticky-content ${
            note.noteType === "snippet"
              ? "sticky-snippet-content"
              : ""
          }`}
        >
          {note.noteType === "snippet" ? (
            note.content ? (
              parseSnippetParts(note.content).map(
                (part, partIndex) =>
                  part.type === "code" ? (
                    <div
                      className="sticky-code-block"
                      key={`code-${part.index}-${partIndex}`}
                    >
                      <div className="sticky-code-header">
                        <span>
                          {part.language || "code"}
                        </span>

                        <button
                          onClick={() =>
                            void copySnippetBlock(
                              part.content,
                              part.index,
                            )
                          }
                          title="Copy this code block"
                          type="button"
                        >
                          {copiedBlock === part.index
                            ? "Copied ✓"
                            : "Copy"}
                        </button>
                      </div>

                      <pre>
                        <code>{part.content}</code>
                      </pre>
                    </div>
                  ) : (
                    <span
                      className="sticky-snippet-text"
                      key={`text-${partIndex}`}
                    >
                      {part.content}
                    </span>
                  ),
              )
            ) : (
              <em>Empty snippet</em>
            )
          ) : (
            note.content || <em>Empty note</em>
          )}
        </section>
      )}
    </main>
  );
}

function toMessage(cause: unknown) {
  return cause instanceof Error ? cause.message : String(cause);
}
