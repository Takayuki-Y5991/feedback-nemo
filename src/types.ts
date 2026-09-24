export type Person = {
  id: number;
  name: string;
  isActive: boolean;
  createdAt: string;
};

export type Memo = {
  id: number;
  personId: number;
  personName: string;
  occurredAt: string;
  content: string;
  createdAt: string;
  updatedAt: string;
};

/** One page of memos plus the full match count, so the UI can render `page / pages`. */
export type MemoPage = {
  memos: Memo[];
  /** Number of memos matching the filters, ignoring `limit` / `offset`. */
  total: number;
};

export type ViewMode = "capture" | "history" | "settings";

export type Period = "all" | "this-quarter" | "last-quarter" | "last-3-months" | "custom";

export type MemoExport = {
  exportedAt: string;
  filter: { person: string | null; from: string | null; to: string | null; query: string | null };
  count: number;
  memos: { person: string; occurredAt: string; content: string; createdAt: string; updatedAt: string }[];
};
