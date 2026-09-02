import { invoke } from "@tauri-apps/api/core";
import type { Memo, Person, ViewMode } from "./types";

const isTauri = () => "__TAURI_INTERNALS__" in window;

const fallbackPeople: Person[] = [
  { id: 1, name: "Alex", isActive: true, createdAt: new Date().toISOString() },
  { id: 2, name: "Jordan", isActive: true, createdAt: new Date().toISOString() },
];

let fallbackMemos: Memo[] = [];

export async function getPeople(): Promise<Person[]> {
  return isTauri() ? invoke("get_people") : fallbackPeople;
}

export async function addPerson(name: string): Promise<Person> {
  if (isTauri()) return invoke("add_person", { name });
  const person = { id: Date.now(), name, isActive: true, createdAt: new Date().toISOString() };
  fallbackPeople.push(person);
  return person;
}

export async function createMemo(personId: number, occurredAt: string, content: string): Promise<Memo> {
  if (isTauri()) return invoke("create_memo", { personId, occurredAt, content });
  const person = fallbackPeople.find((item) => item.id === personId)!;
  const now = new Date().toISOString();
  const memo = { id: Date.now(), personId, personName: person.name, occurredAt, content, createdAt: now, updatedAt: now };
  fallbackMemos = [memo, ...fallbackMemos];
  return memo;
}

export async function getMemos(personId?: number): Promise<Memo[]> {
  if (isTauri()) return invoke("get_memos", { personId: personId ?? null });
  return personId ? fallbackMemos.filter((memo) => memo.personId === personId) : fallbackMemos;
}

export async function deleteMemo(id: number): Promise<void> {
  if (isTauri()) return invoke("delete_memo", { id });
  fallbackMemos = fallbackMemos.filter((memo) => memo.id !== id);
}

export async function setViewMode(mode: ViewMode): Promise<void> {
  if (isTauri()) await invoke("set_view_mode", { mode });
}

export async function hideWindow(): Promise<void> {
  if (isTauri()) await invoke("hide_main_window");
}
