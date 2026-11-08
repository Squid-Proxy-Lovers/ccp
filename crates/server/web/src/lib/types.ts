// SPDX-License-Identifier: AGPL-3.0-or-later
// Shapes returned by the CCP HTTP admin API (`/v1/admin/*`).

export interface SessionMetadata {
  session_name: string;
  session_id: number;
  description: string;
  owner: string;
  labels: string[];
  visibility: string;
  purpose: string;
}

export interface SessionStats {
  session: SessionMetadata;
  shelves: number;
  books: number;
  entries: number;
  is_active: boolean;
}

export interface AgentStatus {
  session_name: string;
  team: string;
  agent_name: string;
  worker_id: string;
  /** Free text set by the agent; often JSON. */
  status: string;
  /** Unix milliseconds, as a string. */
  updated_at: string;
  /** Unix milliseconds, as a string. */
  expires_at: string;
}

export interface Instructions {
  content: string;
  /** SQLite UTC datetime, `YYYY-MM-DD HH:MM:SS`. */
  updated_at: string;
}

export interface AdminOverview {
  sessions: SessionStats[];
  agents: AgentStatus[];
  global_master: Instructions;
}

export interface Activity {
  id: string;
  session_id: number;
  session_name: string;
  kind: 'entry_created' | 'entry_appended' | string;
  shelf_name: string;
  book_name: string;
  entry_name: string;
  actor: string;
  agent_name: string | null;
  host_name: string | null;
  reason: string | null;
  content: string;
  /** Unix seconds, as a string. */
  created_at: string;
}

export interface LibraryShelf { shelf_name: string; description: string; book_count: number; entry_count: number }
export interface LibraryBook { shelf_name: string; book_name: string; shelf_description: string; description: string; entry_count: number }
export interface LibraryEntry {
  name: string;
  description: string;
  labels: string[];
  shelf_name: string;
  book_name: string;
  shelf_description: string;
  book_description: string;
}
export interface Library { shelves: LibraryShelf[]; books: LibraryBook[]; entries: LibraryEntry[] }
export interface EntryRecord extends LibraryEntry { context: string }

/** Session-scoped views first, then instance-wide ones. */
export type SessionView = 'overview' | 'library' | 'activity' | 'agents' | 'instructions';
export type InstanceView = 'sessions' | 'live' | 'all-agents' | 'global-instructions';
export type View = SessionView | InstanceView;

/** Where the library should open after navigating from another view. */
export interface EntryTarget { shelf: string; book: string; entry: string }
