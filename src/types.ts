export interface SearchResult {
  id: string;
  path: string;
  name: string;
  kind: 'File' | 'Folder' | 'App' | 'Bookmark' | 'Note' | string;
  score: number;
  size?: number;
  modified?: number;
  content_snippet?: string;
  icon?: string;
}

export interface SearchResults {
  results: SearchResult[];
  total: number;
  query_time_ms: number;
}

export interface AppResult {
  id: string;
  name: string;
  path: string;
  icon?: string;
  score: number;
}

export interface IndexStatus {
  is_indexing: boolean;
  files_indexed: number;
  total_files: number;
  progress_percent: number;
  last_updated: number;
  errors: string[];
}

export interface AppSettings {
  indexed_paths: string[];
  excluded_patterns: string[];
  max_results: number;
  hotkey: string;
  theme: 'dark' | 'light' | 'system';
  enable_content_search: boolean;
  indexing_interval_minutes: number;
  fuzzy_threshold: number;
  launch_on_startup: boolean;
}

export interface HistoryItem {
  id: string;
  path: string;
  name: string;
  kind: string;
  opened_at: number;
}


