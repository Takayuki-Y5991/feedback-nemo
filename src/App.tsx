import { useCallback, useEffect, useMemo, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { Archive, Check, ChevronDown, Clock3, History, LockKeyhole, Plus, Search, Settings, Sparkles, Trash2, UserRound, X } from "lucide-react";
import { addPerson, createMemo, deleteMemo, getMemos, getPeople, hideWindow, setViewMode } from "./api";
import type { Memo, Person, ViewMode } from "./types";

function toLocalInputValue(date = new Date()) {
  const offset = date.getTimezoneOffset();
  return new Date(date.getTime() - offset * 60_000).toISOString().slice(0, 16);
}

function formatDate(value: string) {
  return new Intl.DateTimeFormat("en", { month: "short", day: "numeric", hour: "2-digit", minute: "2-digit" }).format(new Date(value));
}

function beginWindowDrag(event: React.MouseEvent<HTMLElement>) {
  if (event.button !== 0 || (event.target as HTMLElement).closest("button")) return;
  if ("__TAURI_INTERNALS__" in window) void getCurrentWindow().startDragging();
}

function App() {
  const [view, setView] = useState<ViewMode>("capture");
  const [people, setPeople] = useState<Person[]>([]);
  const [memos, setMemos] = useState<Memo[]>([]);
  const [loading, setLoading] = useState(true);
  const [captureSession, setCaptureSession] = useState(0);

  const refreshPeople = useCallback(async () => setPeople(await getPeople()), []);
  const refreshMemos = useCallback(async () => setMemos(await getMemos()), []);

  useEffect(() => {
    Promise.all([refreshPeople(), refreshMemos()]).finally(() => setLoading(false));
    if (!("__TAURI_INTERNALS__" in window)) return;
    const unlisten = listen<ViewMode>("view-mode", (event) => {
      setView(event.payload);
      if (event.payload === "capture") setCaptureSession((value) => value + 1);
    });
    return () => { void unlisten.then((fn) => fn()); };
  }, [refreshMemos, refreshPeople]);

  const navigate = async (mode: ViewMode) => {
    setView(mode);
    await setViewMode(mode);
    if (mode === "history") await refreshMemos();
  };

  if (loading) return <div className="loading" role="status">Loading…</div>;

  return (
    <main className={`app app--${view}`}>
      {view === "capture" && <CaptureView key={captureSession} people={people} onSaved={refreshMemos} onNavigate={navigate} />}
      {view === "history" && <HistoryView people={people} memos={memos} onRefresh={refreshMemos} onNavigate={navigate} />}
      {view === "settings" && <SettingsView people={people} onRefresh={refreshPeople} onNavigate={navigate} />}
    </main>
  );
}

function CaptureView({ people, onSaved, onNavigate }: { people: Person[]; onSaved: () => Promise<void>; onNavigate: (mode: ViewMode) => void }) {
  const [personId, setPersonId] = useState(people[0]?.id ?? 0);
  const [occurredAt, setOccurredAt] = useState(toLocalInputValue());
  const [content, setContent] = useState(() => localStorage.getItem("feedback-memo-draft") ?? "");
  const [status, setStatus] = useState<"idle" | "saving" | "saved">("idle");
  const [error, setError] = useState("");
  useEffect(() => localStorage.setItem("feedback-memo-draft", content), [content]);

  const save = async () => {
    if (!personId) return setError("Add a teammate in Settings first.");
    if (!content.trim()) return setError("Write a short note before saving.");
    setError("");
    setStatus("saving");
    try {
      await createMemo(personId, new Date(occurredAt).toISOString(), content.trim());
      localStorage.removeItem("feedback-memo-draft");
      setContent("");
      setOccurredAt(toLocalInputValue());
      setStatus("saved");
      await onSaved();
      window.setTimeout(() => {
        setStatus("idle");
        window.requestAnimationFrame(() => document.getElementById("memo-content")?.focus());
      }, 900);
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
      setStatus("idle");
    }
  };

  return (
    <section className="capture-shell" onKeyDown={(event) => {
      if ((event.metaKey || event.ctrlKey) && event.key === "Enter") { event.preventDefault(); void save(); }
      if (event.key === "Escape") void hideWindow();
    }}>
      <header className="capture-header" data-tauri-drag-region onMouseDown={beginWindowDrag}>
        <div className="capture-brand" data-tauri-drag-region>
          <span className="app-glyph"><Sparkles size={17} strokeWidth={1.8} /></span>
          <div data-tauri-drag-region>
            <h1>Feedback Memo</h1>
          <p><span className="live-dot" /> Local &amp; private</p>
          </div>
        </div>
        <div className="header-actions">
          <button className="icon-button" aria-label="Open history" onClick={() => onNavigate("history")}><History size={19} /></button>
          <button className="icon-button" aria-label="Open settings" onClick={() => onNavigate("settings")}><Settings size={19} /></button>
          <button className="icon-button" aria-label="Close" onClick={() => void hideWindow()}><X size={19} /></button>
        </div>
      </header>

      <div className="capture-form">
        <div className="capture-intro">
          <p className="eyebrow">QUICK NOTE</p>
          <h2>Capture it while it’s fresh.</h2>
        </div>

        <div className="metadata-grid">
        {people.length === 0 ? (
          <button className="empty-people" autoFocus onClick={() => onNavigate("settings")}>
            <UserRound size={22} />
            <span><strong>Add a teammate</strong><small>Choose who your notes are about</small></span>
          </button>
        ) : (
          <label className="field compact-field">
            <span><UserRound size={14} /> Person</span>
            <span className="select-wrap">
              <select id="memo-person" autoFocus value={personId} onChange={(event) => setPersonId(Number(event.target.value))}>
                {people.filter((person) => person.isActive).map((person) => <option key={person.id} value={person.id}>{person.name}</option>)}
              </select>
              <ChevronDown size={16} aria-hidden="true" />
            </span>
          </label>
        )}

        <label className="field compact-field">
          <span><Clock3 size={14} /> Date</span>
          <input
            id="memo-occurred-at"
            type="date"
            value={occurredAt.slice(0, 10)}
            onChange={(event) => setOccurredAt(`${event.target.value}T${occurredAt.slice(11) || toLocalInputValue().slice(11)}`)}
            onKeyDown={(event) => {
              if (event.key !== "Tab") return;
              event.preventDefault();
              document.getElementById(event.shiftKey ? "memo-person" : "memo-content")?.focus();
            }}
          />
        </label>
        </div>

        <label className="field note-field">
          <span>Note</span>
          <textarea id="memo-content" rows={3} value={content} onChange={(event) => setContent(event.target.value)} placeholder="What happened?" />
        </label>

        {error && <p className="error" role="alert">{error}</p>}

        <div className="form-footer">
          <span className="local-note"><LockKeyhole size={13} /> Saved on this device</span>
          <button className={`primary-button ${status === "saved" ? "is-saved" : ""}`} disabled={status !== "idle"} onClick={() => void save()}>
            {status === "saved" ? <><Check size={18} /> Saved</> : status === "saving" ? "Saving…" : <>Save <span className="button-shortcut">⌘↵</span></>}
          </button>
        </div>
      </div>
    </section>
  );
}

function HistoryView({ people, memos, onRefresh, onNavigate }: { people: Person[]; memos: Memo[]; onRefresh: () => Promise<void>; onNavigate: (mode: ViewMode) => void }) {
  const [query, setQuery] = useState("");
  const [personId, setPersonId] = useState(0);
  const filtered = useMemo(() => memos.filter((memo) => (!personId || memo.personId === personId) && (!query || memo.content.toLowerCase().includes(query.toLowerCase()))), [memos, personId, query]);

  return (
    <section className="workspace">
      <aside className="sidebar">
        <div className="brand"><span className="brand-mark"><Archive size={18} /></span><strong>Feedback Memo</strong></div>
        <nav aria-label="Main navigation">
          <button onClick={() => onNavigate("capture")}><Plus size={18} /> New note</button>
          <button className="active" aria-current="page"><History size={18} /> History</button>
          <button onClick={() => onNavigate("settings")}><Settings size={18} /> Settings</button>
        </nav>
        <p className="privacy-note">All data stays<br />on this device.</p>
      </aside>
      <div className="content-pane">
        <header className="page-header" data-tauri-drag-region onMouseDown={beginWindowDrag}>
          <div><p className="eyebrow">REVIEW</p><h1>History</h1><p>Look back at the moments you captured.</p></div>
          <div className="page-actions"><button className="primary-button" onClick={() => onNavigate("capture")}><Plus size={18} /> New note</button><button className="icon-button" aria-label="Close" onClick={() => void hideWindow()}><X size={19} /></button></div>
        </header>
        <div className="filters">
          <label className="search-field"><Search size={18} /><span className="sr-only">Search notes</span><input value={query} onChange={(event) => setQuery(event.target.value)} placeholder="Search notes" /></label>
          <label><span className="sr-only">Filter by person</span><select value={personId} onChange={(event) => setPersonId(Number(event.target.value))}><option value={0}>Everyone</option>{people.map((person) => <option key={person.id} value={person.id}>{person.name}</option>)}</select></label>
        </div>
        <div className="memo-list" aria-live="polite">
          {filtered.length === 0 ? <div className="empty-state"><History size={28} /><h2>No notes yet</h2><p>Capture a small moment when you notice it.</p><button onClick={() => onNavigate("capture")}>Add your first note</button></div> : filtered.map((memo) => (
            <article className="memo-card" key={memo.id}>
              <div className="avatar" aria-hidden="true">{memo.personName.slice(0, 1)}</div>
              <div className="memo-body"><div className="memo-meta"><strong>{memo.personName}</strong><time dateTime={memo.occurredAt}>{formatDate(memo.occurredAt)}</time></div><p>{memo.content}</p></div>
              <button className="icon-button danger" aria-label={`Delete note about ${memo.personName}`} onClick={async () => { await deleteMemo(memo.id); await onRefresh(); }}><Trash2 size={17} /></button>
            </article>
          ))}
        </div>
      </div>
    </section>
  );
}

function SettingsView({ people, onRefresh, onNavigate }: { people: Person[]; onRefresh: () => Promise<void>; onNavigate: (mode: ViewMode) => void }) {
  const [name, setName] = useState("");
  const [error, setError] = useState("");
  const submit = async (event: React.FormEvent) => {
    event.preventDefault();
    if (!name.trim()) return setError("Enter a display name.");
    try { await addPerson(name.trim()); setName(""); setError(""); await onRefresh(); }
    catch (cause) { setError(cause instanceof Error ? cause.message : String(cause)); }
  };
  return (
    <section className="settings-page">
      <header className="simple-header" data-tauri-drag-region onMouseDown={beginWindowDrag}><button className="icon-button" aria-label="Back to quick note" onClick={() => onNavigate("capture")}><X size={20} /></button><div data-tauri-drag-region><p className="eyebrow">SETTINGS</p><h1>Teammates</h1></div></header>
      <div className="settings-content">
        <p className="lead">Add the people you work with. Their previous notes stay available when your team changes.</p>
        <form className="add-person" onSubmit={submit}>
          <label><span>Display name</span><input value={name} onChange={(event) => setName(event.target.value)} placeholder="e.g. Alex" /></label>
          <button className="primary-button" type="submit"><Plus size={18} /> Add</button>
        </form>
        {error && <p className="error" role="alert">{error}</p>}
        <div className="people-list">
          <h2>Teammates <span>{people.length}</span></h2>
          {people.length === 0 ? <p className="muted">No teammates added yet.</p> : people.map((person) => <div className="person-row" key={person.id}><span className="avatar">{person.name.slice(0, 1)}</span><strong>{person.name}</strong><span className="status-dot">Active</span></div>)}
        </div>
      </div>
    </section>
  );
}

export default App;
