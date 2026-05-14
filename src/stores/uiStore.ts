import { create } from 'zustand';
import { loadFromStorage, saveToStorage } from '../utils/storage';
import { DEFAULT_SHORTCUTS, ShortcutDefinition } from './shortcutDefaults';

export type ThemeMode = 'dark' | 'light';
export type Density = 'spacious' | 'balanced';

export const ACCENT_PRESETS = [
  '#3b82f6', '#6366f1', '#8b5cf6', '#a855f7', '#d946ef',
  '#ec4899', '#f43f5e', '#ef4444', '#f97316', '#f59e0b',
  '#10b981', '#059669', '#14b8a6', '#06b6d4', '#0ea5e9',
];

export type { ShortcutDefinition };

interface ToastState {
  message: string;
  type: 'success' | 'error' | 'info';
  visible: boolean;
}

function loadShortcuts(): ShortcutDefinition[] {
  try {
    const stored = localStorage.getItem('quikfind-shortcuts');
    if (stored) {
      const parsed = JSON.parse(stored) as ShortcutDefinition[];
      if (Array.isArray(parsed) && parsed.length > 0) return parsed;
    }
  } catch {}
  return DEFAULT_SHORTCUTS.map(s => ({ ...s }));
}

function saveShortcuts(shortcuts: ShortcutDefinition[]): void {
  try {
    localStorage.setItem('quikfind-shortcuts', JSON.stringify(shortcuts));
  } catch {}
}

interface UIState {
  theme: ThemeMode;
  accentColor: string;
  density: Density;
  keyboardShortcuts: ShortcutDefinition[];
  toast: ToastState;
  setTheme: (theme: ThemeMode) => void;
  setAccentColor: (color: string) => void;
  setDensity: (density: Density) => void;
  updateShortcut: (id: string, keys: string) => void;
  resetShortcuts: () => void;
  showToast: (message: string, type?: ToastState['type']) => void;
  hideToast: () => void;
}

export const useUIStore = create<UIState>((set, get) => ({
  theme: loadFromStorage<ThemeMode>('quikfind-theme', 'dark'),
  accentColor: loadFromStorage<string>('quikfind-accent', '#3b82f6'),
  density: loadFromStorage<Density>('quikfind-density', 'balanced'),
  keyboardShortcuts: loadShortcuts(),
  toast: { message: '', type: 'info', visible: false },

  setTheme: (theme) => {
    saveToStorage('quikfind-theme', theme);
    set({ theme });
  },

  setAccentColor: (color) => {
    saveToStorage('quikfind-accent', color);
    set({ accentColor: color });
  },

  setDensity: (density) => {
    saveToStorage('quikfind-density', density);
    set({ density });
  },

  updateShortcut: (id, keys) => {
    const shortcuts = get().keyboardShortcuts.map(s =>
      s.id === id ? { ...s, keys } : s
    );
    saveShortcuts(shortcuts);
    set({ keyboardShortcuts: shortcuts });
  },

  resetShortcuts: () => {
    const shortcuts = DEFAULT_SHORTCUTS.map(s => ({ ...s }));
    saveShortcuts(shortcuts);
    set({ keyboardShortcuts: shortcuts });
  },

  showToast: (message, type = 'success') => {
    set({ toast: { message, type, visible: true } });
  },

  hideToast: () => {
    set({ toast: { message: '', type: 'info', visible: false } });
  },
}));
