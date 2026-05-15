import { useEffect, useRef } from 'react';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { SearchResult } from '../types';
import { useUIStore } from '../stores/uiStore';
import { matchShortcut } from '../utils/shortcuts';
import { useOpenResult } from './useOpenResult';

interface UseKeyboardShortcutsProps {
  searchInputRef: React.RefObject<HTMLInputElement | null>;
  resultsRef: React.MutableRefObject<SearchResult[]>;
  queryRef: React.MutableRefObject<string>;
  selectedIndexRef: React.MutableRefObject<number>;
  lastNavMethodRef: React.MutableRefObject<'keyboard' | 'mouse'>;
  isSettingsOpenRef: React.MutableRefObject<boolean>;
  setQuery: (q: string) => void;
  setResults: (r: SearchResult[]) => void;
  setSelectedIndex: (updater: number | ((prev: number) => number)) => void;
  setIsSettingsOpen: (open: boolean) => void;
  setSettingsTab: (tab: 'appearance' | 'shortcuts') => void;
}

export function useKeyboardShortcuts({
  searchInputRef,
  resultsRef,
  queryRef,
  selectedIndexRef,
  lastNavMethodRef,
  isSettingsOpenRef,
  setQuery,
  setResults,
  setSelectedIndex,
  setIsSettingsOpen,
  setSettingsTab,
}: UseKeyboardShortcutsProps) {
  const openResult = useOpenResult();
  const openResultRef = useRef(openResult);
  openResultRef.current = openResult;

  const shortcutsRef = useRef(useUIStore.getState().keyboardShortcuts);

  useEffect(() => {
    const unsub = useUIStore.subscribe(
      (state) => { shortcutsRef.current = state.keyboardShortcuts; },
    );
    return unsub;
  }, []);

  useEffect(() => {
    const handleGlobalKeyDown = (e: KeyboardEvent) => {
      const shortcuts = shortcutsRef.current;
      const target = e.target as HTMLElement;
      const isInput = target.tagName === 'INPUT' || target.tagName === 'TEXTAREA';
      const s = { query: queryRef.current, results: resultsRef.current, selectedIndex: selectedIndexRef.current, isSettingsOpen: isSettingsOpenRef.current };

      for (const shortcut of shortcuts) {
        if (matchShortcut(e, shortcut.keys)) {
          switch (shortcut.id) {
            case 'focus-search':
              e.preventDefault();
              searchInputRef.current?.focus();
              return;
            case 'clear-search':
              if (s.isSettingsOpen) {
                setIsSettingsOpen(false);
              } else if (s.query) {
                setQuery('');
                setResults([]);
                searchInputRef.current?.focus();
              } else {
                getCurrentWindow().hide();
              }
              return;
            case 'open-settings':
              e.preventDefault();
              setSettingsTab('appearance');
              setIsSettingsOpen(true);
              return;
            case 'show-shortcuts':
              e.preventDefault();
              setSettingsTab('shortcuts');
              setIsSettingsOpen(true);
              return;
            case 'open-result':
              if (s.results[s.selectedIndex]) {
                openResultRef.current(s.results[s.selectedIndex]);
              }
              return;
            case 'navigate-up':
              if (!isInput) {
                e.preventDefault();
                lastNavMethodRef.current = 'keyboard';
                setSelectedIndex((prev: number) => Math.max(0, prev - 1));
                return;
              }
              break;
            case 'navigate-down':
              if (!isInput) {
                e.preventDefault();
                lastNavMethodRef.current = 'keyboard';
                setSelectedIndex((prev: number) => Math.min(prev + 1, Math.max(0, s.results.length - 1)));
                return;
              }
              break;
          }
          break;
        }
      }
    };

    window.addEventListener('keydown', handleGlobalKeyDown);
    return () => window.removeEventListener('keydown', handleGlobalKeyDown);
  }, [searchInputRef, resultsRef, queryRef, selectedIndexRef, lastNavMethodRef, isSettingsOpenRef, setQuery, setResults, setSelectedIndex, setIsSettingsOpen, setSettingsTab]);
}
