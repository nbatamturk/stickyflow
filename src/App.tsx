import { FormEvent, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import "./App.css";

type View = "loading" | "setup" | "locked" | "workspace";

type SecurityStatus = {
  configured: boolean;
  enabled: boolean;
};

function App() {
  const [view, setView] = useState<View>("loading");
  const [lockEnabled, setLockEnabled] = useState(false);
  const [password, setPassword] = useState("");
  const [confirmPassword, setConfirmPassword] = useState("");
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    void initializeSecurity();
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

  function handleLockNow() {
    clearPasswordFields();
    setError("");
    setView("locked");
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
              Add an optional application password. Only an Argon2id password hash is stored;
              your plaintext password is never written to disk.
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
            Note encryption will be added separately; this step protects application startup.
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
            <p className="muted">Your note windows stay hidden until the application is unlocked.</p>
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
          <span className="status-pill">{lockEnabled ? "Lock enabled" : "Local mode"}</span>
          {lockEnabled && (
            <button className="ghost-button small-button" onClick={handleLockNow} type="button">
              Lock now
            </button>
          )}
        </div>
      </header>

      <section className="hero-panel">
        <div>
          <p className="eyebrow">V0.1 FOUNDATION</p>
          <h2>The secure shell is running.</h2>
          <p className="muted">
            Next: encrypted SQLite storage, real sticky windows, snippets and one-click copy.
          </p>
        </div>
        <button className="primary-button mock-button" disabled type="button">
          + New note
        </button>
      </section>

      <section className="card-grid" aria-label="Planned StickyFlow features">
        <article className="feature-card">
          <span className="feature-icon">N</span>
          <h3>Notes</h3>
          <p>Fast autosaved notes that can live as independent desktop windows.</p>
        </article>
        <article className="feature-card">
          <span className="feature-icon">&lt;/&gt;</span>
          <h3>Snippets</h3>
          <p>Reusable commands and text blocks with one-click clipboard actions.</p>
        </article>
        <article className="feature-card">
          <span className="feature-icon">✓</span>
          <h3>Todos</h3>
          <p>Local tasks designed for future provider-based synchronization.</p>
        </article>
      </section>
    </main>
  );
}

function toMessage(cause: unknown) {
  return cause instanceof Error ? cause.message : String(cause);
}

export default App;
