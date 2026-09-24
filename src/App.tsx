import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { Archive, Check, ChevronDown, ChevronLeft, ChevronRight, ClipboardCopy, Clock3, Download, History, LockKeyhole, Plus, Search, Settings, Sparkles, Trash2, UserRound, X } from "lucide-react";
import { addPerson, copyToClipboard, createMemo, deleteMemo, exportMemos, getPeople, hideWindow, saveExport, searchMemos, setViewMode } from "./api";
import type { MemoPage, Period, Person, ViewMode } from "./types";

const ADD_TEAMMATE_OPTION = -1;

/** Rows per History page. The database does the slicing; this is the only place the size lives. */
const PAGE_SIZE = 20;

const SEARCH_DEBOUNCE_MS = 260;

const PERIOD_OPTIONS: { value: Period; label: string }[] = [
  { value: "all", label: "All time" },
  { value: "this-quarter", label: "This quarter" },
  { value: "last-quarter", label: "Last quarter" },
  { value: "last-3-months", label: "Last 3 months" },
  { value: "custom", label: "Custom range…" },
];

// ASSUMPTION: calendar quarters (Jan–Mar, Apr–Jun, Jul–Sep, Oct–Dec). The organisation's fiscal
// start month is not a setting yet — when it becomes one, this constant is the only thing to wire up.
const QUARTER_START_MONTH = 0;

function toLocalInputValue(date = new Date()) {
  const offset = date.getTimezoneOffset();
  return new Date(date.getTime() - offset * 60_000).toISOString().slice(0, 16);
}

// Local midnight of (y, m, d + offsetDays) as an instant, so boundaries follow the user's clock.
function localDayStart(year: number, month: number, day: number, offsetDays = 0) {
  return new Date(year, month, day + offsetDays).toISOString();
}

function dateInputStart(value: string, offsetDays = 0) {
  const [year, month, day] = value.split("-").map(Number);
  return localDayStart(year, month - 1, day, offsetDays);
}

/** Pure: period + custom inputs -> inclusive `from` / exclusive `to` instants (null = unbounded). */
function periodRange(period: Period, customFrom: string, customTo: string, today = new Date()): { from: string | null; to: string | null } {
  const [year, month, day] = [today.getFullYear(), today.getMonth(), today.getDate()];
  const quarterStart = month - ((month - QUARTER_START_MONTH + 12) % 3);
  switch (period) {
    case "this-quarter":
      return { from: localDayStart(year, quarterStart, 1), to: localDayStart(year, quarterStart + 3, 1) };
    case "last-quarter":
      return { from: localDayStart(year, quarterStart - 3, 1), to: localDayStart(year, quarterStart, 1) };
    case "last-3-months":
      // Rolling window: the same day three months back, through the end of today.
      return { from: localDayStart(year, month - 3, day), to: localDayStart(year, month, day, 1) };
    case "custom":
      // `to` is the start of the day AFTER the picked one, so the last day is fully included.
      return { from: customFrom ? dateInputStart(customFrom) : null, to: customTo ? dateInputStart(customTo, 1) : null };
    default:
      return { from: null, to: null };
  }
}

function exportFileName(personName?: string) {
  const slug = personName?.toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-|-$/g, "");
  return `feedback-memo-${slug || "all"}-${toLocalInputValue().slice(0, 10)}.json`;
}

function formatDate(value: string) {
  return new Intl.DateTimeFormat("en", { month: "short", day: "numeric", hour: "2-digit", minute: "2-digit" }).format(new Date(value));
}

function beginWindowDrag(event: React.MouseEvent<HTMLElement>) {
  if (event.button !== 0 || (event.target as HTMLElement).closest("button")) return;
  if ("__TAURI_INTERNALS__" in window) void getCurrentWindow().startDragging();
}

function focusById(id: string) {
  window.requestAnimationFrame(() => document.getElementById(id)?.focus());
}

function App() {
  const [view, setView] = useState<ViewMode>("capture");
  const [people, setPeople] = useState<Person[]>([]);
  const [loading, setLoading] = useState(true);
  const [captureSession, setCaptureSession] = useState(0);

  const refreshPeople = useCallback(async () => setPeople(await getPeople()), []);

  useEffect(() => {
    refreshPeople().finally(() => setLoading(false));
    if (!("__TAURI_INTERNALS__" in window)) return;
    const unlisten = listen<ViewMode>("view-mode", (event) => {
      setView(event.payload);
      if (event.payload === "capture") setCaptureSession((value) => value + 1);
    });
    return () => { void unlisten.then((fn) => fn()); };
  }, [refreshPeople]);

  const navigate = async (mode: ViewMode) => {
    setView(mode);
    await setViewMode(mode);
  };

  if (loading) return <div className="loading" role="status">Loading…</div>;

  // Memos are no longer hoisted here: History pages them straight from the database. Views are
  // mounted conditionally, so leaving and returning to History remounts it and re-fetches page 1 —
  // that is what makes a note saved in Capture show up on the way back.
  return (
    <main className={`app app--${view}`}>
      {view === "capture" && <CaptureView key={captureSession} people={people} onPeopleChanged={refreshPeople} onNavigate={navigate} />}
      {view === "history" && <HistoryView people={people} onNavigate={navigate} />}
      {view === "settings" && <SettingsView people={people} onRefresh={refreshPeople} onNavigate={navigate} />}
    </main>
  );
}

function AddTeammateModal({ onClose, onAdded }: { onClose: () => void; onAdded: (person: Person) => void | Promise<void> }) {
  const [name, setName] = useState("");
  const [error, setError] = useState("");
  const [saving, setSaving] = useState(false);
  const inputRef = useRef<HTMLInputElement>(null);

  useEffect(() => { inputRef.current?.focus(); }, []);

  const submit = async (event: React.FormEvent) => {
    event.preventDefault();
    if (saving) return;
    const trimmed = name.trim();
    if (!trimmed) return setError("Enter a display name.");
    setError("");
    setSaving(true);
    try {
      await onAdded(await addPerson(trimmed));
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
      setSaving(false);
    }
  };

  return (
    <div
      className="modal-overlay"
      onKeyDown={(event) => {
        // The modal owns Escape and ⌘↵ so the panel behind it never hides or saves.
        if (event.key === "Escape") { event.preventDefault(); event.stopPropagation(); onClose(); return; }
        if ((event.metaKey || event.ctrlKey) && event.key === "Enter") { event.preventDefault(); event.stopPropagation(); void submit(event); }
      }}
    >
      <div className="modal" role="dialog" aria-modal="true" aria-labelledby="add-teammate-title">
        <header className="modal-header">
          <h2 id="add-teammate-title">Add teammate</h2>
          <button type="button" className="icon-button" aria-label="Close" onClick={onClose}><X size={18} /></button>
        </header>
        <form className="modal-body" onSubmit={submit}>
          <label className="field">
            <span><UserRound size={14} /> Display name</span>
            <input ref={inputRef} value={name} onChange={(event) => { setName(event.target.value); if (error) setError(""); }} placeholder="e.g. Alex" />
          </label>
          {error && <p className="error" role="alert">{error}</p>}
          <div className="modal-actions">
            <button type="button" className="ghost-button" onClick={onClose}>Cancel</button>
            <button type="submit" className="primary-button" disabled={saving}>{saving ? "Adding…" : <><Plus size={16} /> Add</>}</button>
          </div>
        </form>
      </div>
    </div>
  );
}

function CaptureView({ people, onPeopleChanged, onNavigate }: { people: Person[]; onPeopleChanged: () => Promise<void>; onNavigate: (mode: ViewMode) => void }) {
  const [personId, setPersonId] = useState(people[0]?.id ?? 0);
  const [occurredAt, setOccurredAt] = useState(toLocalInputValue());
  const [content, setContent] = useState(() => localStorage.getItem("feedback-memo-draft") ?? "");
  const [status, setStatus] = useState<"idle" | "saving" | "saved">("idle");
  const [error, setError] = useState("");
  const [adding, setAdding] = useState(false);
  useEffect(() => localStorage.setItem("feedback-memo-draft", content), [content]);

  const save = async () => {
    if (!personId) return setError("Add a teammate first.");
    if (!content.trim()) return setError("Write a short note before saving.");
    setError("");
    setStatus("saving");
    try {
      await createMemo(personId, new Date(occurredAt).toISOString(), content.trim());
      localStorage.removeItem("feedback-memo-draft");
      setContent("");
      setOccurredAt(toLocalInputValue());
      setStatus("saved");
      window.setTimeout(() => {
        setStatus("idle");
        focusById("memo-content");
      }, 900);
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
      setStatus("idle");
    }
  };

  return (
    <section className="capture-shell" onKeyDown={(event) => {
      if (adding) return;
      if ((event.metaKey || event.ctrlKey) && event.key === "Enter") { event.preventDefault(); void save(); }
      if (event.key === "Escape") void hideWindow();
    }}>
      <header className="capture-header" onMouseDown={beginWindowDrag}>
        <div className="capture-brand">
          <span className="app-glyph"><Sparkles size={17} strokeWidth={1.8} /></span>
          <div>
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
          <button className="empty-people" autoFocus onClick={() => setAdding(true)}>
            <UserRound size={22} />
            <span><strong>Add a teammate</strong><small>Choose who your notes are about</small></span>
          </button>
        ) : (
          <label className="field compact-field">
            <span><UserRound size={14} /> Person</span>
            <span className="select-wrap">
              <select
                id="memo-person"
                autoFocus
                value={personId}
                onChange={(event) => {
                  const value = Number(event.target.value);
                  // The sentinel opens the modal and is never committed as personId.
                  if (value === ADD_TEAMMATE_OPTION) return setAdding(true);
                  setPersonId(value);
                }}
              >
                {people.filter((person) => person.isActive).map((person) => <option key={person.id} value={person.id}>{person.name}</option>)}
                <option value={ADD_TEAMMATE_OPTION}>+ Add teammate</option>
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

      {adding && (
        <AddTeammateModal
          onClose={() => { setAdding(false); focusById(people.length === 0 ? "memo-content" : "memo-person"); }}
          onAdded={async (person) => {
            await onPeopleChanged();
            setPersonId(person.id);
            setError("");
            setAdding(false);
            focusById("memo-content");
          }}
        />
      )}
    </section>
  );
}

function HistoryView({ people, onNavigate }: { people: Person[]; onNavigate: (mode: ViewMode) => void }) {
  const [query, setQuery] = useState("");
  const [search, setSearch] = useState("");
  const [personId, setPersonId] = useState(0);
  const [period, setPeriod] = useState<Period>("all");
  const [customFrom, setCustomFrom] = useState("");
  const [customTo, setCustomTo] = useState("");
  const [page, setPage] = useState(1);
  const [reloads, setReloads] = useState(0);
  const [result, setResult] = useState<MemoPage>({ memos: [], total: 0 });
  const [fetching, setFetching] = useState(true);
  const [status, setStatus] = useState<"idle" | "saving" | "copying" | "saved" | "copied">("idle");
  const [error, setError] = useState("");
  const rootRef = useRef<HTMLElement>(null);
  const paneRef = useRef<HTMLDivElement>(null);

  const invertedRange = period === "custom" && !!customFrom && !!customTo && customFrom > customTo;
  const range = useMemo(() => (invertedRange ? { from: null, to: null } : periodRange(period, customFrom, customTo)), [invertedRange, period, customFrom, customTo]);

  useEffect(() => { rootRef.current?.focus(); }, []);

  // Debounce only the trip to the database — `query` itself stays controlled and instant.
  useEffect(() => {
    const timer = window.setTimeout(() => setSearch(query.trim()), SEARCH_DEBOUNCE_MS);
    return () => window.clearTimeout(timer);
  }, [query]);

  // Any filter change invalidates the page number: page 4 of a one-page result is an empty list.
  // Adjusting during render (rather than in an effect) means the fetch below never runs with a
  // stale page, so no wasted request and no flash of the wrong page.
  const filterKey = `${personId}|${period}|${range.from}|${range.to}|${search}`;
  const [lastFilterKey, setLastFilterKey] = useState(filterKey);
  if (lastFilterKey !== filterKey) {
    setLastFilterKey(filterKey);
    setPage(1);
  }

  useEffect(() => {
    // `ignore` is flipped by the cleanup before the next effect runs, so a slow response from an
    // abandoned filter combination can never overwrite a newer one.
    let ignore = false;
    setFetching(true);
    searchMemos(personId || null, range.from, range.to, search || null, PAGE_SIZE, (page - 1) * PAGE_SIZE)
      .then((next) => {
        if (ignore) return;
        // Deleting the last row of the last page leaves us past the end: step back instead of
        // rendering nothing. The re-run of this effect fetches the page we land on.
        const lastPage = Math.max(1, Math.ceil(next.total / PAGE_SIZE));
        if (page > lastPage) return setPage(lastPage);
        setResult(next);
        setError("");
        setFetching(false);
      })
      .catch((cause) => {
        if (ignore) return;
        setError(cause instanceof Error ? cause.message : String(cause));
        setFetching(false);
      });
    return () => { ignore = true; };
  }, [personId, range.from, range.to, search, page, reloads]);

  const { memos, total } = result;
  const pageCount = Math.max(1, Math.ceil(total / PAGE_SIZE));
  const hasFilters = personId !== 0 || period !== "all" || search !== "";
  const canExport = status === "idle" && !invertedRange && total > 0;

  const goToPage = (next: number) => {
    setPage(next);
    paneRef.current?.scrollTo({ top: 0 });
  };

  const clearFilters = () => {
    setQuery("");
    setSearch("");
    setPersonId(0);
    setPeriod("all");
    setCustomFrom("");
    setCustomTo("");
  };

  const runExport = async (mode: "save" | "copy") => {
    if (!canExport) return;
    setError("");
    setStatus(mode === "save" ? "saving" : "copying");
    try {
      // Deliberately unpaged: the export covers every match, not the visible page.
      const json = await exportMemos(personId || null, range.from, range.to, search || null);
      if (mode === "save") {
        // A cancelled save panel resolves to null — that is not an error, just go back to idle.
        if (await saveExport(exportFileName(people.find((person) => person.id === personId)?.name), json) === null) return setStatus("idle");
      } else {
        await copyToClipboard(json);
      }
      setStatus(mode === "save" ? "saved" : "copied");
      window.setTimeout(() => setStatus("idle"), 1500);
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
      setStatus("idle");
    }
  };

  return (
    <section className="workspace" ref={rootRef} tabIndex={-1} onKeyDown={(event) => { if (event.key === "Escape") void hideWindow(); }}>
      <aside className="sidebar">
        <div className="brand"><span className="brand-mark"><Archive size={18} /></span><strong>Feedback Memo</strong></div>
        <nav aria-label="Main navigation">
          <button onClick={() => onNavigate("capture")}><Plus size={18} /> New note</button>
          <button className="active" aria-current="page"><History size={18} /> History</button>
          <button onClick={() => onNavigate("settings")}><Settings size={18} /> Settings</button>
        </nav>
        <p className="privacy-note">All data stays<br />on this device.</p>
      </aside>
      <div className="content-pane" ref={paneRef}>
        <header className="page-header" onMouseDown={beginWindowDrag}>
          <div><p className="eyebrow">REVIEW</p><h1>History</h1><p>Look back at the moments you captured.</p></div>
          <div className="page-actions"><button className="primary-button" onClick={() => onNavigate("capture")}><Plus size={18} /> New note</button><button className="icon-button" aria-label="Close" onClick={() => void hideWindow()}><X size={19} /></button></div>
        </header>
        <div className="filter-bar">
          <div className="filters">
            <label className="search-field"><Search size={18} /><span className="sr-only">Search notes</span><input value={query} onChange={(event) => setQuery(event.target.value)} placeholder="Search notes" /></label>
            <label><span className="sr-only">Filter by person</span><select value={personId} onChange={(event) => setPersonId(Number(event.target.value))}><option value={0}>Everyone</option>{people.map((person) => <option key={person.id} value={person.id}>{person.name}</option>)}</select></label>
            <label><span className="sr-only">Filter by period</span><select value={period} onChange={(event) => setPeriod(event.target.value as Period)}>{PERIOD_OPTIONS.map((option) => <option key={option.value} value={option.value}>{option.label}</option>)}</select></label>
            <div className="export-actions">
              <button className="ghost-button" disabled={!canExport} aria-label="Export filtered notes as JSON" onClick={() => void runExport("save")}>
                {status === "saved" ? <><Check size={15} /> Saved</> : status === "saving" ? "Saving…" : <><Download size={15} /> Export</>}
              </button>
              <button className="ghost-button" disabled={!canExport} aria-label="Copy filtered notes as JSON" onClick={() => void runExport("copy")}>
                {status === "copied" ? <><Check size={15} /> Copied</> : status === "copying" ? "Copying…" : <><ClipboardCopy size={15} /> Copy</>}
              </button>
            </div>
          </div>
          {period === "custom" && (
            <div className="range-fields">
              <label className="field"><span>From</span><input type="date" value={customFrom} onChange={(event) => setCustomFrom(event.target.value)} /></label>
              <label className="field"><span>To</span><input type="date" value={customTo} onChange={(event) => setCustomTo(event.target.value)} /></label>
            </div>
          )}
          {invertedRange && <p className="error" role="alert">“From” must be on or before “To”.</p>}
          {error && <p className="error" role="alert">{error}</p>}
          {pageCount > 1 && <p className="filter-note">Export and Copy cover all {total} matching notes, not just this page.</p>}
        </div>
        <div className={`memo-list${fetching ? " is-loading" : ""}`} aria-live="polite" aria-busy={fetching}>
          {memos.length > 0 ? memos.map((memo) => (
            <article className="memo-card" key={memo.id}>
              <div className="avatar" aria-hidden="true">{memo.personName.slice(0, 1)}</div>
              <div className="memo-body"><div className="memo-meta"><strong>{memo.personName}</strong><time dateTime={memo.occurredAt}>{formatDate(memo.occurredAt)}</time></div><p>{memo.content}</p></div>
              <button className="icon-button danger" aria-label={`Delete note about ${memo.personName}`} onClick={async () => {
                try {
                  await deleteMemo(memo.id);
                  setReloads((value) => value + 1);
                } catch (cause) {
                  setError(cause instanceof Error ? cause.message : String(cause));
                }
              }}><Trash2 size={17} /></button>
            </article>
          )) : fetching ? (
            // Never claim "no notes" while the answer is still in flight.
            <p className="list-status">Loading notes…</p>
          ) : hasFilters ? (
            <div className="empty-state"><Search size={28} /><h2>No notes match these filters</h2><p>Try another person, period, or search term.</p><button onClick={clearFilters}>Clear filters</button></div>
          ) : (
            <div className="empty-state"><History size={28} /><h2>No notes yet</h2><p>Capture a small moment when you notice it.</p><button onClick={() => onNavigate("capture")}>Add your first note</button></div>
          )}
        </div>
        {total > 0 && (
          <footer className="pager">
            <span className="pager-count">{total === 1 ? "1 note" : `${total} notes`}</span>
            {pageCount > 1 && (
              <div className="pager-nav">
                <button className="ghost-button" disabled={page <= 1} onClick={() => goToPage(page - 1)}><ChevronLeft size={15} /> Prev</button>
                <span className="pager-position">{page} / {pageCount}</span>
                <button className="ghost-button" disabled={page >= pageCount} onClick={() => goToPage(page + 1)}>Next <ChevronRight size={15} /></button>
              </div>
            )}
          </footer>
        )}
      </div>
    </section>
  );
}

function SettingsView({ people, onRefresh, onNavigate }: { people: Person[]; onRefresh: () => Promise<void>; onNavigate: (mode: ViewMode) => void }) {
  const [adding, setAdding] = useState(false);
  const rootRef = useRef<HTMLElement>(null);
  const focusRoot = () => window.requestAnimationFrame(() => rootRef.current?.focus());

  useEffect(() => { rootRef.current?.focus(); }, []);

  return (
    <section className="settings-page" ref={rootRef} tabIndex={-1} onKeyDown={(event) => {
      if (adding) return;
      if (event.key === "Escape") void hideWindow();
    }}>
      <header className="simple-header" onMouseDown={beginWindowDrag}>
        <button className="icon-button" aria-label="Back to quick note" onClick={() => onNavigate("capture")}><X size={20} /></button>
        <div><p className="eyebrow">SETTINGS</p><h1>Teammates</h1></div>
      </header>
      <div className="settings-content">
        <div className="people-list">
          <div className="people-list-head">
            <h2>Teammates <span>{people.length}</span></h2>
            <button className="primary-button" onClick={() => setAdding(true)}><Plus size={18} /> Add teammate</button>
          </div>
          {people.length === 0 ? <p className="muted">No teammates added yet.</p> : people.map((person) => <div className="person-row" key={person.id}><span className="avatar">{person.name.slice(0, 1)}</span><strong>{person.name}</strong><span className="status-dot">Active</span></div>)}
        </div>
      </div>

      {adding && (
        <AddTeammateModal
          onClose={() => { setAdding(false); focusRoot(); }}
          onAdded={async () => { await onRefresh(); setAdding(false); focusRoot(); }}
        />
      )}
    </section>
  );
}

export default App;
