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

export type ViewMode = "capture" | "history" | "settings";
