export interface ShortcutDefinition {
  id: string;
  label: string;
  description: string;
  keys: string;
  defaultKeys: string;
  category: string;
}

export const DEFAULT_SHORTCUTS: ShortcutDefinition[] = [
  { id: 'focus-search', label: 'Focus Search', description: 'Focus the search input', keys: 'Ctrl+K', defaultKeys: 'Ctrl+K', category: 'Navigation' },
  { id: 'clear-search', label: 'Clear / Close Preview', description: 'Clear search or close preview', keys: 'Esc', defaultKeys: 'Esc', category: 'Navigation' },
  { id: 'navigate-up', label: 'Navigate Up', description: 'Move selection upward', keys: '↑', defaultKeys: '↑', category: 'Navigation' },
  { id: 'navigate-down', label: 'Navigate Down', description: 'Move selection downward', keys: '↓', defaultKeys: '↓', category: 'Navigation' },
  { id: 'open-result', label: 'Open Result', description: 'Open the selected result', keys: 'Enter', defaultKeys: 'Enter', category: 'Navigation' },
  { id: 'open-settings', label: 'Open Settings', description: 'Open the settings panel', keys: 'Ctrl+,', defaultKeys: 'Ctrl+,', category: 'App' },
  { id: 'show-shortcuts', label: 'Shortcut Help', description: 'Show keyboard shortcuts list', keys: 'Ctrl+Shift+?', defaultKeys: 'Ctrl+Shift+?', category: 'App' },
];
