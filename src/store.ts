import { create } from 'zustand';
import { invoke } from '@tauri-apps/api/core';
import { AppSettings, HistoryItem } from './types';

const HISTORY_LIMIT = 50;

interface AppStore {
  settings: AppSettings;
  history: HistoryItem[];
  loadSettings: () => Promise<void>;
  updateSettings: (newSettings: Partial<AppSettings>) => Promise<void>;
  addToHistory: (item: HistoryItem) => Promise<void>;
  loadHistory: () => Promise<void>;
}

const defaultSettings: AppSettings = {
  indexed_paths: [],
  excluded_patterns: ['**/node_modules/**', '**/.git/**', '**/target/**', '**/dist/**', '**/.DS_Store'],
  max_results: 25,
  hotkey: 'CmdOrCtrl+Space',
  theme: 'dark',
  enable_content_search: true,
  indexing_interval_minutes: 30,
  fuzzy_threshold: 0.6,

  launch_on_startup: false,
};

export const useStore = create<AppStore>((set, get) => ({
  settings: defaultSettings,
  history: [],

  loadSettings: async () => {
    try {
      const settings: AppSettings = await invoke('get_settings');
      set({ settings });
    } catch (error) {
      console.error('Failed to load settings:', error);
      set({ settings: defaultSettings });
    }
  },

  updateSettings: async (newSettings) => {
    const current = get().settings;
    const updated = { ...current, ...newSettings };
    
    try {
      await invoke('update_settings', { settings: updated });
      set({ settings: updated });
    } catch (error) {
      console.error('Failed to update settings:', error);
    }
  },

  addToHistory: async (item) => {
    try {
      await invoke('add_to_history', { item });
      set(state => ({
        history: [item, ...state.history.slice(0, HISTORY_LIMIT - 1)]
      }));
    } catch (error) {
      console.error('Failed to add to history:', error);
    }
  },

  loadHistory: async () => {
    try {
      const history: HistoryItem[] = await invoke('get_history', { limit: HISTORY_LIMIT });
      set({ history });
    } catch (error) {
      console.error('Failed to load history:', error);
    }
  },
}));