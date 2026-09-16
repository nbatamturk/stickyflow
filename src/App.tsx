import { FormEvent, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import {
  readText,
  writeText,
} from "@tauri-apps/plugin-clipboard-manager";
import { wrapCodeFence } from "./codeFence";
import "./App.css";
import "./SnippetTools.css";

type View = "loading" | "setup" | "locked" | "workspace";
type NoteType = "note" | "snippet" | "todo";

type SecurityStatus = {
  configured: boolean;
  enabled: boolean;
};

type ChipSettings = {
  autoWidth: boolean;
  fixedWidth: number;
  height: number;
  minWidth: number;
  maxWidth: number;
  fontSize: number;
};

type Note = {
  id: string;
  title: string;
  content: string;
  color: string;
  noteType: NoteType;
  pinned: boolean;
  createdAt: number;
  updatedAt: number;
};

type Draft = {
  id: string | null;
  title: string;
  content: string;
  color: string;
  noteType: NoteType;
  pinned: boolean;
};

const emptyDraft: Draft = {
  id: null,
  title: "",
  content: "",
  color: "sand",
  noteType: "note",
  pinned: false,
};

const defaultChipSettings: ChipSettings = {
  autoWidth: true,
  fixedWidth: 90,
  height: 26,
  minWidth: 60,
  maxWidth: 180,
  fontSize: 11,
};

function App() {
  const [view, setView] = useState<View>("loading");
  const [lockEnabled, setLockEnabled] = useState(false);
  const [password, setPassword] = useState("");
  const [confirmPassword, setConfirmPassword] = useState("");
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  const [notes, setNotes] = useState<Note[]>([]);
  const [notesLoading, setNotesLoading] = useState(false);
  const [draft, setDraft] = useState<Draft>(emptyDraft);
  const [editorOpen, setEditorOpen] = useState(false);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [chipSettings, setChipSettings] =
    useState<ChipSettings>(defaultChipSettings);
  const [settingsBusy, setSettingsBusy] = useState(false);
  const [settingsMessage, setSettingsMessage] = useState("");
  const [copiedNoteId, setCopiedNoteId] = useState<string | null>(null);
  const contentTextareaRef = useRef<HTMLTextAreaElement | null>(null);
  const lastCopiedTextRef = useRef<string | null>(null);

  useEffect(() => {
    void initializeSecurity();
  }, []);

  useEffect(() => {
    if (view === "workspace") {
      void loadNotes();
      void loadChipSettings();
    } else {
      setNotes([]);
      setDraft(emptyDraft);
      setEditorOpen(false);
      setSettingsOpen(false);
    }
  }, [view]);

  useEffect(() => {
    if (view !== "workspace") {
      return;
    }

    let disposed = false;
    let unlisten: (() => void) | undefined;

    void listen<string>(
      "stickyflow-note-updated",
      (event) => {
        void syncChangedNote(event.payload);
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
  }, [view]);

  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;

    void listen<string>(
      "stickyflow-clipboard-owned",
      (event) => {
        lastCopiedTextRef.current = event.payload;
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
  }, []);

  async function initializeSecurity() {
    try {
      const status = await invoke<SecurityStatus>("security_status");
      setLockEnabled(status.enabled);

      if (!status.configured) {
        setView("setup");
      } else if (status.enabled) {
        setView("locked");
      } else {
        setView("workspace");
      }
    } catch (cause) {
      setError(toMessage(cause));
      setView("setup");
    }
  }

  async function handleSetup(event: FormEvent) {
    event.preventDefault();
    setError("");

    if (password.length < 8) {
      setError("Password must be at least 8 characters.");
      return;
    }

    if (password !== confirmPassword) {
      setError("Passwords do not match.");
      return;
    }

    setBusy(true);
    try {
      await invoke("setup_password", { password });
      setLockEnabled(true);
      clearPasswordFields();
      setView("workspace");
    } catch (cause) {
      setError(toMessage(cause));
    } finally {
      setBusy(false);
    }
  }

  async function handleSkipLock() {
    setBusy(true);
    setError("");

    try {
      await invoke("skip_password_setup");
      setLockEnabled(false);
      clearPasswordFields();
      setView("workspace");
    } catch (cause) {
      setError(toMessage(cause));
    } finally {
      setBusy(false);
    }
  }

  async function handleUnlock(event: FormEvent) {
    event.preventDefault();
    setBusy(true);
    setError("");

    try {
      const verified = await invoke<boolean>("verify_password", { password });
      if (!verified) {
        setError("Incorrect password.");
        return;
      }

      clearPasswordFields();
      setView("workspace");
    } catch (cause) {
      setError(toMessage(cause));
    } finally {
      setBusy(false);
    }
  }

  async function clearOwnedClipboardIfUnchanged() {
    const ownedText = lastCopiedTextRef.current;
    lastCopiedTextRef.current = null;

    if (ownedText === null) {
      return;
    }

    try {
      const currentClipboard = await readText();

      if (currentClipboard === ownedText) {
        await writeText("");
      }
    } catch (cause) {
      // Clipboard cleanup must never prevent the security lock.
      console.warn("Could not clean StickyFlow clipboard content:", cause);
    }
  }

  async function handleLockNow() {
    setBusy(true);
    setError("");
    try {
      // Lock first. Clipboard cleanup is best-effort and must not delay
      // dropping the in-memory master key or closing sticky windows.
      await invoke("lock_session");
      await clearOwnedClipboardIfUnchanged();
      clearPasswordFields();
      setNotes([]);
      setDraft(emptyDraft);
      setEditorOpen(false);
      setView("locked");
    } catch (cause) {
      setError(toMessage(cause));
    } finally {
      setBusy(false);
    }
  }

  async function loadNotes() {
    setNotesLoading(true);
    setError("");
    try {
      const loaded = await invoke<Note[]>("list_notes");
      setNotes(loaded);
    } catch (cause) {
      setError(toMessage(cause));
    } finally {
      setNotesLoading(false);
    }
  }

  async function syncChangedNote(id: string) {
    try {
      const updated = await invoke<Note>(
        "get_note",
        { id },
      );

      setNotes((current) => {
        const exists = current.some(
          (note) => note.id === updated.id,
        );

        const next = exists
          ? current.map((note) =>
              note.id === updated.id
                ? updated
                : note,
            )
          : [updated, ...current];

        return sortNotes(next);
      });

      setDraft((current) =>
        current.id === updated.id
          ? {
              id: updated.id,
              title: updated.title,
              content: updated.content,
              color: updated.color,
              noteType: updated.noteType,
              pinned: updated.pinned,
            }
          : current,
      );
    } catch (cause) {
      console.warn(
        "Could not synchronize changed note:",
        cause,
      );
    }
  }

  function startNewNote() {
    setDraft(emptyDraft);
    setEditorOpen(true);
    setError("");
  }

  function editNote(note: Note) {
    setDraft({
      id: note.id,
      title: note.title,
      content: note.content,
      color: note.color,
      noteType: note.noteType,
      pinned: note.pinned,
    });
    setEditorOpen(true);
    setError("");
  }

  function insertCodeBlock() {
    const textarea = contentTextareaRef.current;

    if (!textarea || draft.noteType !== "snippet") {
      return;
    }

    const edit = wrapCodeFence(
      draft.content,
      textarea.selectionStart,
      textarea.selectionEnd,
    );

    setDraft((current) => ({
      ...current,
      content: edit.value,
    }));

    requestAnimationFrame(() => {
      textarea.focus();
      textarea.setSelectionRange(
        edit.selectionStart,
        edit.selectionEnd,
      );
    });
  }

  async function openSticky(note: Note) {
    setError("");

    try {
      await invoke("open_sticky_window", { id: note.id });
    } catch (cause) {
      setError(toMessage(cause));
    }
  }

  async function saveNote(event: FormEvent) {
    event.preventDefault();
    setBusy(true);
    setError("");

    const title = draft.title.trim() || "Untitled";

    try {
      if (draft.id) {
        const updated = await invoke<Note>("update_note", {
          input: {
            ...draft,
            title,
          },
        });
        setNotes((current) =>
          sortNotes(
            current.map((note) =>
              note.id === updated.id ? updated : note,
            ),
          ),
        );
      } else {
        const created = await invoke<Note>("create_note", {
          input: {
            title,
            content: draft.content,
            color: draft.color,
            noteType: draft.noteType,
          },
        });
        setNotes((current) => sortNotes([created, ...current]));
      }

      setDraft(emptyDraft);
      setEditorOpen(false);
    } catch (cause) {
      setError(toMessage(cause));
    } finally {
      setBusy(false);
    }
  }

  async function deleteNote(note: Note) {
    setBusy(true);
    setError("");
    try {
      await invoke("delete_note", { id: note.id });
      setNotes((current) => current.filter((item) => item.id !== note.id));
      if (draft.id === note.id) {
        setDraft(emptyDraft);
        setEditorOpen(false);
      }
    } catch (cause) {
      setError(toMessage(cause));
    } finally {
      setBusy(false);
    }
  }

  async function togglePinned(note: Note) {
    setError("");
    try {
      const updated = await invoke<Note>("update_note", {
        input: {
          id: note.id,
          title: note.title,
          content: note.content,
          color: note.color,
          noteType: note.noteType,
          pinned: !note.pinned,
        },
      });
      setNotes((current) =>
        sortNotes(
          current.map((item) =>
            item.id === updated.id ? updated : item,
          ),
        ),
      );
    } catch (cause) {
      setError(toMessage(cause));
    }
  }

  async function copySnippet(note: Note) {
    if (
      view !== "workspace" ||
      note.noteType !== "snippet"
    ) {
      return;
    }

    setError("");

    try {
      // Intentionally copy the exact stored content:
      // no trim, no newline conversion.
      await writeText(note.content);
      lastCopiedTextRef.current = note.content;

      setCopiedNoteId(note.id);

      window.setTimeout(() => {
        setCopiedNoteId((current) =>
          current === note.id ? null : current,
        );
      }, 1400);
    } catch (cause) {
      setError(toMessage(cause));
    }
  }

  async function loadChipSettings() {
    try {
      const loaded =
        await invoke<ChipSettings>("get_chip_settings");

      setChipSettings(loaded);
    } catch (cause) {
      setError(toMessage(cause));
    }
  }

  async function saveChipSettings(event: FormEvent) {
    event.preventDefault();

    setSettingsBusy(true);
    setSettingsMessage("");
    setError("");

    try {
      const saved =
        await invoke<ChipSettings>("save_chip_settings", {
          input: chipSettings,
        });

      setChipSettings(saved);
      setSettingsMessage("Chip settings saved.");
    } catch (cause) {
      setError(toMessage(cause));
    } finally {
      setSettingsBusy(false);
    }
  }

  async function resetChipSettings() {
    setSettingsBusy(true);
    setSettingsMessage("");
    setError("");

    try {
      const reset =
        await invoke<ChipSettings>("reset_chip_settings");

      setChipSettings(reset);
      setSettingsMessage("Defaults restored.");
    } catch (cause) {
      setError(toMessage(cause));
    } finally {
      setSettingsBusy(false);
    }
  }

  function clearPasswordFields() {
    setPassword("");
    setConfirmPassword("");
  }

  if (view === "loading") {
    return (
      <main className="screen center-screen">
        <div className="brand-mark">SF</div>
        <p className="muted">Starting StickyFlow…</p>
      </main>
    );
  }

  if (view === "setup") {
    return (
      <main className="screen center-screen">
        <section className="auth-card">
          <div className="brand-mark">SF</div>
          <div className="auth-copy">
            <p className="eyebrow">WELCOME TO STICKYFLOW</p>
            <h1>Protect your notes</h1>
            <p className="muted">
              Add an optional application password. Your master encryption key stays in the Rust
              backend and is never exposed to the note UI.
            </p>
          </div>

          <form className="auth-form" onSubmit={handleSetup}>
            <label>
              Password
              <input
                autoFocus
                autoComplete="new-password"
                minLength={8}
                onChange={(event) => setPassword(event.currentTarget.value)}
                placeholder="At least 8 characters"
                type="password"
                value={password}
              />
            </label>

            <label>
              Confirm password
              <input
                autoComplete="new-password"
                minLength={8}
                onChange={(event) => setConfirmPassword(event.currentTarget.value)}
                placeholder="Repeat password"
                type="password"
                value={confirmPassword}
              />
            </label>

            {error && <p className="error-message">{error}</p>}

            <button className="primary-button" disabled={busy} type="submit">
              {busy ? "Saving…" : "Enable lock"}
            </button>
            <button className="ghost-button" disabled={busy} onClick={handleSkipLock} type="button">
              Continue without password
            </button>
          </form>

          <p className="security-note">
            Notes are encrypted at rest in both modes. Without a password, the local encryption key
            is protected by your OS user account and file permissions rather than a separate password.
          </p>
        </section>
      </main>
    );
  }

  if (view === "locked") {
    return (
      <main className="screen center-screen">
        <section className="auth-card compact-card">
          <div className="lock-icon" aria-hidden="true">⌁</div>
          <div className="auth-copy">
            <p className="eyebrow">STICKYFLOW LOCKED</p>
            <h1>Unlock your notes</h1>
            <p className="muted">The decryption key is not loaded until your password is verified.</p>
          </div>

          <form className="auth-form" onSubmit={handleUnlock}>
            <label>
              Password
              <input
                autoFocus
                autoComplete="current-password"
                onChange={(event) => setPassword(event.currentTarget.value)}
                placeholder="Enter your password"
                type="password"
                value={password}
              />
            </label>

            {error && <p className="error-message">{error}</p>}

            <button className="primary-button" disabled={busy || password.length === 0} type="submit">
              {busy ? "Checking…" : "Unlock"}
            </button>
          </form>
        </section>
      </main>
    );
  }

  return (
    <main className="workspace">
      <header className="topbar">
        <div>
          <p className="eyebrow">LOCAL-FIRST DESKTOP NOTES</p>
          <h1>StickyFlow</h1>
        </div>
        <div className="topbar-actions">
          <span className="status-pill">{lockEnabled ? "Encrypted + locked" : "Encrypted local mode"}</span>
          <button
            className="ghost-button small-button"
            onClick={() => {
              setSettingsOpen((current) => !current);
              setSettingsMessage("");
            }}
            type="button"
          >
            Settings
          </button>
          {lockEnabled && (
            <button className="ghost-button small-button" disabled={busy} onClick={handleLockNow} type="button">
              Lock now
            </button>
          )}
        </div>
      </header>

      {settingsOpen && (
        <section className="chip-settings-panel">
          <div className="section-heading">
            <div>
              <p className="eyebrow">STICKY APPEARANCE</p>
              <h3>Compact chip</h3>
            </div>
            <button
              className="icon-button"
              onClick={() => setSettingsOpen(false)}
              type="button"
            >
              Close
            </button>
          </div>

          <form
            className="chip-settings-form"
            onSubmit={saveChipSettings}
          >
            <label className="chip-setting-toggle">
              <input
                checked={chipSettings.autoWidth}
                onChange={(event) => {
                  const checked = event.currentTarget.checked;
                  setChipSettings((current) => ({
                    ...current,
                    autoWidth: checked,
                  }));
                }}
                type="checkbox"
              />
              Auto width from title
            </label>

            <div className="chip-settings-grid">
              <label>
                Fixed width
                <div className="px-input">
                  <input
                    disabled={chipSettings.autoWidth}
                    max={640}
                    min={48}
                    onChange={(event) => {
                      const value = Number(event.currentTarget.value);
                      setChipSettings((current) => ({
                        ...current,
                        fixedWidth: value,
                      }));
                    }}
                    type="number"
                    value={chipSettings.fixedWidth}
                  />
                  <span>px</span>
                </div>
              </label>

              <label>
                Height
                <div className="px-input">
                  <input
                    max={64}
                    min={20}
                    onChange={(event) => {
                      const value = Number(event.currentTarget.value);
                      setChipSettings((current) => ({
                        ...current,
                        height: value,
                      }));
                    }}
                    type="number"
                    value={chipSettings.height}
                  />
                  <span>px</span>
                </div>
              </label>

              <label>
                Min width
                <div className="px-input">
                  <input
                    max={480}
                    min={48}
                    onChange={(event) => {
                      const value = Number(event.currentTarget.value);
                      setChipSettings((current) => ({
                        ...current,
                        minWidth: value,
                      }));
                    }}
                    type="number"
                    value={chipSettings.minWidth}
                  />
                  <span>px</span>
                </div>
              </label>

              <label>
                Max width
                <div className="px-input">
                  <input
                    max={640}
                    min={48}
                    onChange={(event) => {
                      const value = Number(event.currentTarget.value);
                      setChipSettings((current) => ({
                        ...current,
                        maxWidth: value,
                      }));
                    }}
                    type="number"
                    value={chipSettings.maxWidth}
                  />
                  <span>px</span>
                </div>
              </label>

              <label>
                Font size
                <div className="px-input">
                  <input
                    max={24}
                    min={8}
                    onChange={(event) => {
                      const value = Number(event.currentTarget.value);
                      setChipSettings((current) => ({
                        ...current,
                        fontSize: value,
                      }));
                    }}
                    type="number"
                    value={chipSettings.fontSize}
                  />
                  <span>px</span>
                </div>
              </label>
            </div>

            <p className="muted chip-settings-hint">
              Auto width measures the rendered GTK title.
              Turn it off to force every compact chip to the
              exact Fixed width.
            </p>

            {settingsMessage && (
              <p className="settings-success">
                {settingsMessage}
              </p>
            )}

            <div className="chip-settings-actions">
              <button
                className="primary-button"
                disabled={settingsBusy}
                type="submit"
              >
                {settingsBusy ? "Saving…" : "Save settings"}
              </button>

              <button
                className="ghost-button"
                disabled={settingsBusy}
                onClick={() => void resetChipSettings()}
                type="button"
              >
                Reset defaults
              </button>
            </div>
          </form>
        </section>
      )}

      <section className="hero-panel notes-hero">
        <div>
          <p className="eyebrow">ENCRYPTED SQLITE</p>
          <h2>Your notes are now real.</h2>
          <p className="muted">
            Titles and contents are AES-256-GCM encrypted before they are written to SQLite.
          </p>
        </div>
        <button className="primary-button" onClick={startNewNote} type="button">
          + New note
        </button>
      </section>

      {error && <p className="workspace-error error-message">{error}</p>}

      <section className="notes-layout">
        <div className="notes-panel">
          <div className="section-heading">
            <div>
              <p className="eyebrow">CONTROL CENTER</p>
              <h3>Notes</h3>
            </div>
            <span className="note-count">{notes.length}</span>
          </div>

          {notesLoading ? (
            <p className="empty-state">Decrypting notes…</p>
          ) : notes.length === 0 ? (
            <div className="empty-state">
              <strong>No notes yet.</strong>
              <span>Create the first encrypted note.</span>
            </div>
          ) : (
            <div className="note-list">
              {notes.map((note) => (
                <article
                  className={`note-card note-${note.color} ${
                    note.noteType === "snippet" ? "snippet-card" : ""
                  }`}
                  key={note.id}
                >
                  <button className="note-main" onClick={() => editNote(note)} type="button">
                    <div className="note-card-topline">
                      <span className="note-type">{note.noteType}</span>
                      {note.pinned && <span title="Pinned">●</span>}
                    </div>
                    <h4>{note.title}</h4>
                    <p>{note.content || "Empty note"}</p>
                    <time>{formatDate(note.updatedAt)}</time>
                  </button>
                  <div className="note-actions">
                    {note.noteType === "snippet" && (
                      <button
                        className="icon-button"
                        onClick={() => void copySnippet(note)}
                        title="Copy full snippet"
                        type="button"
                      >
                        {copiedNoteId === note.id
                          ? "Copied ✓"
                          : "Copy"}
                      </button>
                    )}

                    <button className="icon-button" onClick={() => void openSticky(note)} type="button">
                      Sticky
                    </button>
                    <button className="icon-button" onClick={() => void togglePinned(note)} type="button">
                      {note.pinned ? "Unpin" : "Pin"}
                    </button>
                    <button className="icon-button danger-button" onClick={() => void deleteNote(note)} type="button">
                      Delete
                    </button>
                  </div>
                </article>
              ))}
            </div>
          )}
        </div>

        <aside className={`editor-panel ${editorOpen ? "editor-open" : ""}`}>
          {editorOpen ? (
            <form className="note-editor" onSubmit={saveNote}>
              <div className="section-heading">
                <div>
                  <p className="eyebrow">{draft.id ? "EDIT NOTE" : "NEW NOTE"}</p>
                  <h3>{draft.id ? "Update note" : "Create note"}</h3>
                </div>
                <button
                  className="icon-button"
                  onClick={() => {
                    setEditorOpen(false);
                    setDraft(emptyDraft);
                  }}
                  type="button"
                >
                  Close
                </button>
              </div>

              <label>
                Title
                <input
                  autoFocus
                  maxLength={200}
                  onChange={(event) => {
                    const value = event.currentTarget.value;
                    setDraft((current) => ({ ...current, title: value }));
                  }}
                  placeholder="e.g. CPMS commands"
                  value={draft.title}
                />
              </label>

              <label>
                Type
                <select
                  onChange={(event) => {
                    const value = event.currentTarget.value as NoteType;
                    setDraft((current) => ({ ...current, noteType: value }));
                  }}
                  value={draft.noteType}
                >
                  <option value="note">Note</option>
                  <option value="snippet">Snippet</option>
                  <option value="todo">Todo</option>
                </select>
              </label>

              <label>
                <div className="snippet-editor-toolbar">
                  <span>Content</span>
                  {draft.noteType === "snippet" && (
                    <button
                      className="snippet-code-button"
                      onClick={insertCodeBlock}
                      title="Wrap selection in a fenced code block"
                      type="button"
                    >
                      &lt;/&gt; Code block
                    </button>
                  )}
                </div>
                <textarea
                  onChange={(event) => {
                    const value = event.currentTarget.value;
                    setDraft((current) => ({ ...current, content: value }));
                  }}
                  placeholder="Write or paste anything…"
                  ref={contentTextareaRef}
                  rows={12}
                  value={draft.content}
                />
              </label>

              <fieldset className="color-fieldset">
                <legend>Color</legend>
                <div className="color-options">
                  {[
                    ["sand", "Sand"],
                    ["yellow", "Yellow"],
                    ["green", "Green"],
                    ["blue", "Blue"],
                    ["pink", "Pink"],
                  ].map(([value, label]) => (
                    <button
                      aria-pressed={draft.color === value}
                      className={`color-chip note-${value}`}
                      key={value}
                      onClick={() => setDraft((current) => ({ ...current, color: value }))}
                      type="button"
                    >
                      {label}
                    </button>
                  ))}
                </div>
              </fieldset>

              <button className="primary-button" disabled={busy} type="submit">
                {busy ? "Saving…" : draft.id ? "Save changes" : "Create note"}
              </button>
            </form>
          ) : (
            <div className="editor-placeholder">
              <span className="feature-icon">N</span>
              <h3>Select a note</h3>
              <p className="muted">Open an existing note or create a new one to edit its encrypted contents.</p>
            </div>
          )}
        </aside>
      </section>
    </main>
  );
}

function sortNotes(notes: Note[]) {
  return [...notes].sort(
    (a, b) =>
      Number(b.pinned) - Number(a.pinned) ||
      b.updatedAt - a.updatedAt,
  );
}

function formatDate(timestamp: number) {
  return new Intl.DateTimeFormat(undefined, {
    day: "2-digit",
    month: "short",
    hour: "2-digit",
    minute: "2-digit",
  }).format(new Date(timestamp));
}

function toMessage(cause: unknown) {
  return cause instanceof Error ? cause.message : String(cause);
}

export default App;
