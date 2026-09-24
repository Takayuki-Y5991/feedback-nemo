import { invoke } from "@tauri-apps/api/core";
import type { Memo, MemoExport, MemoPage, Person, ViewMode } from "./types";

const isTauri = () => "__TAURI_INTERNALS__" in window;

const fallbackPeople: Person[] = [
  { id: 1, name: "Alex", isActive: true, createdAt: new Date().toISOString() },
  { id: 2, name: "Jordan", isActive: true, createdAt: new Date().toISOString() },
];

let fallbackMemos: Memo[] = [];

// SQLite hands out distinct row ids; `Date.now()` does not when two rows land in the same
// millisecond, and colliding ids make a single delete remove several rows in the preview.
let fallbackId = 1_000;
const nextFallbackId = () => ++fallbackId;

export async function getPeople(): Promise<Person[]> {
  return isTauri() ? invoke("get_people") : fallbackPeople;
}

export async function addPerson(name: string): Promise<Person> {
  if (isTauri()) return invoke("add_person", { name });
  const person = { id: nextFallbackId(), name, isActive: true, createdAt: new Date().toISOString() };
  fallbackPeople.push(person);
  return person;
}

export async function createMemo(personId: number, occurredAt: string, content: string): Promise<Memo> {
  if (isTauri()) return invoke("create_memo", { personId, occurredAt, content });
  const person = fallbackPeople.find((item) => item.id === personId);
  if (!person) throw new Error("Choose an active teammate.");
  const now = new Date().toISOString();
  const memo = { id: nextFallbackId(), personId, personName: person.name, occurredAt, content, createdAt: now, updatedAt: now };
  fallbackMemos = [memo, ...fallbackMemos];
  return memo;
}

/** Browser-preview stand-in for the SQL `WHERE` clause: same filters, newest first. */
function fallbackMatches(personId: number | null, from: string | null, to: string | null, query: string | null) {
  const needle = query?.trim().toLowerCase();
  return fallbackMemos
    .filter((memo) =>
      (!personId || memo.personId === personId) &&
      (!from || memo.occurredAt >= from) &&
      (!to || memo.occurredAt < to) &&
      (!needle || memo.content.toLowerCase().includes(needle)))
    .sort((a, b) => b.occurredAt.localeCompare(a.occurredAt));
}

/**
 * One page of matches plus the total match count. `from` is inclusive, `to` is
 * exclusive, both ISO instants (or null for "unbounded"); `query` is a content search.
 */
export async function searchMemos(
  personId: number | null,
  from: string | null,
  to: string | null,
  query: string | null,
  limit: number,
  offset: number,
): Promise<MemoPage> {
  if (isTauri()) return invoke("search_memos", { personId, from, to, query, limit, offset });
  const selected = fallbackMatches(personId, from, to, query);
  return { memos: selected.slice(offset, offset + limit), total: selected.length };
}

export async function deleteMemo(id: number): Promise<void> {
  if (isTauri()) return invoke("delete_memo", { id });
  fallbackMemos = fallbackMemos.filter((memo) => memo.id !== id);
}

// Same filters as the list, but never paged: an export covers every match.
export async function exportMemos(personId: number | null, from: string | null, to: string | null, query: string | null): Promise<string> {
  if (isTauri()) return invoke("export_memos", { personId, from, to, query });
  const selected = fallbackMatches(personId, from, to, query);
  const document: MemoExport = {
    exportedAt: new Date().toISOString(),
    filter: { person: fallbackPeople.find((person) => person.id === personId)?.name ?? null, from, to, query: query?.trim() || null },
    count: selected.length,
    memos: selected.map((memo) => ({ person: memo.personName, occurredAt: memo.occurredAt, content: memo.content, createdAt: memo.createdAt, updatedAt: memo.updatedAt })),
  };
  return JSON.stringify(document, null, 2);
}

export async function saveExport(defaultFileName: string, contents: string): Promise<string | null> {
  if (isTauri()) return invoke("save_export", { defaultFileName, contents });
  const url = URL.createObjectURL(new Blob([contents], { type: "application/json" }));
  const link = Object.assign(window.document.createElement("a"), { href: url, download: defaultFileName });
  link.click();
  URL.revokeObjectURL(url);
  return defaultFileName;
}

export async function copyToClipboard(contents: string): Promise<void> {
  if (isTauri()) return invoke("copy_to_clipboard", { contents });
  await navigator.clipboard.writeText(contents);
}

export async function setViewMode(mode: ViewMode): Promise<void> {
  if (isTauri()) await invoke("set_view_mode", { mode });
}

export async function hideWindow(): Promise<void> {
  if (isTauri()) await invoke("hide_main_window");
}
