import React, { useState, useEffect, useCallback, useRef } from 'react';
import { Settings } from 'lucide-react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { getCurrentWindow } from '@tauri-apps/api/window';
import SearchInput from './components/SearchInput';
import ResultsList from './components/ResultsList';
import SettingsModal from './components/SettingsModal';
import ToastOverlay from './components/ToastOverlay';
import { IndexStatus } from './types';
import { useStore } from './store';
import { useUIStore } from './stores/uiStore';
import { useSearch } from './hooks/useSearch';
import { useKeyboardShortcuts } from './hooks/useKeyboardShortcuts';
import { useOpenResult } from './hooks/useOpenResult';
import { hexToRgb } from './utils/color';

function App() {
  const [isSettingsOpen, setIsSettingsOpen] = useState(false);
  const [settingsTab, setSettingsTab] = useState<'appearance' | 'shortcuts'>('appearance');
  const [indexStatus, setIndexStatus] = useState<IndexStatus | null>(null);
  const { loadSettings } = useStore();
  const { theme, accentColor, toast, hideToast } = useUIStore();

  const {
    query, setQuery,
    results, setResults,
    selectedIndex, setSelectedIndex,
    isLoading, handleKeyDown,
    resultsRef, queryRef, selectedIndexRef,
  } = useSearch();

  const searchInputRef = useRef<HTMLInputElement>(null);
  const lastNavMethodRef = useRef<'keyboard' | 'mouse'>('mouse');
  const isSettingsOpenRef = useRef(isSettingsOpen);
  isSettingsOpenRef.current = isSettingsOpen;

  useKeyboardShortcuts({
    searchInputRef,
    resultsRef, queryRef, selectedIndexRef,
    lastNavMethodRef,
    isSettingsOpenRef,
    setQuery, setResults, setSelectedIndex,
    setIsSettingsOpen, setSettingsTab,
  });

  useEffect(() => {
    const root = document.documentElement;
    root.classList.remove('dark', 'light');
    root.classList.add(theme);
    root.style.setProperty('--accent', accentColor);
    root.style.setProperty('--accent-rgb', hexToRgb(accentColor));
  }, [theme, accentColor]);

  useEffect(() => {
    loadSettings();
    invoke<IndexStatus>('get_index_status')
      .then((status) => setIndexStatus(status))
      .catch(() => {});
  }, [loadSettings]);

  useEffect(() => {
    const unlisten = listen<IndexStatus>('index-progress', (event) => setIndexStatus(event.payload));
    const unlistenSettings = listen('open-settings', () => {
      setSettingsTab('appearance');
      setIsSettingsOpen(true);
    });
    return () => {
      unlisten.then(f => f());
      unlistenSettings.then(f => f());
    };
  }, []);

  useEffect(() => {
    searchInputRef.current?.focus();
  }, []);

  useEffect(() => {
    const unlisten = listen<string>('desktop-key', async (event) => {
      const char = event.payload;
      const win = getCurrentWindow();
      const isVisible = await win.isVisible();

      if (isVisible) return;

      await win.show();
      await win.setFocus();
      setQuery(char);

      setTimeout(() => {
        searchInputRef.current?.focus();
        searchInputRef.current?.setSelectionRange(1, 1);
      }, 50);
    });

    return () => {
      unlisten.then(f => f());
    };
  }, [setQuery]);

  useEffect(() => {
    if (toast.visible) {
      const timer = setTimeout(() => hideToast(), 2200);
      return () => clearTimeout(timer);
    }
  }, [toast.visible, hideToast]);

  const handleClickResult = useCallback((index: number) => {
    setSelectedIndex(index);
  }, [setSelectedIndex]);

  const handleOpenResult = useOpenResult();
  const handleSearchKeyDown = useCallback((e: React.KeyboardEvent<HTMLInputElement>) => {
    if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
      lastNavMethodRef.current = 'keyboard';
    }
    handleKeyDown(e);
  }, [handleKeyDown]);

  const hasSearch = query.trim().length > 0;
  const isIndexing = indexStatus?.is_indexing;
  const progressPercent = indexStatus?.progress_percent ?? 0;

  return (
    <div className="h-screen flex flex-col">
      <header className="flex items-center justify-center px-4 py-2 border-b border-white/10 bg-black/40 select-none relative" style={{ WebkitAppRegion: 'drag' } as React.CSSProperties}>
        <div data-tauri-drag-region className="absolute inset-0 z-0" style={{ WebkitAppRegion: 'drag' } as React.CSSProperties} />
        <div className="w-full max-w-xl relative z-10" style={{ WebkitAppRegion: 'no-drag' } as React.CSSProperties}>
          <SearchInput
            ref={searchInputRef}
            value={query}
            onChange={setQuery}
            onKeyDown={handleSearchKeyDown}
            isLoading={isLoading}
          />
        </div>
        <div className="absolute right-4 flex items-center gap-2 z-10">
          {indexStatus && (
            <div className="flex items-center gap-1.5 text-[10px] text-[var(--text-tertiary)]">
              <span className={`w-1.5 h-1.5 rounded-full ${isIndexing ? 'bg-amber-400 animate-pulse' : 'bg-emerald-500'}`} />
              {isIndexing ? 'Indexing' : `${(indexStatus.files_indexed / 1000).toFixed(0)}k`}
            </div>
          )}
          <button
            onClick={() => { setSettingsTab('appearance'); setIsSettingsOpen(true); }}
            className="p-1 rounded-lg hover:bg-[var(--border-default)] text-[var(--text-tertiary)] hover:text-[var(--text-secondary)]"
          >
            <Settings className="w-3.5 h-3.5" />
          </button>
        </div>
      </header>

      {isIndexing && (
        <div className="px-4 pb-2 flex-shrink-0">
          <div className="flex items-center gap-2">
            <div className="flex-1 h-1 bg-[var(--border-subtle)] rounded-full overflow-hidden">
              <div className="h-full rounded-full transition-all duration-500" style={{ width: `${Math.min(progressPercent, 100)}%`, backgroundColor: 'var(--accent)' }} />
            </div>
            <span className="text-[10px] text-[var(--text-tertiary)] tabular-nums font-mono">{Math.round(progressPercent)}%</span>
          </div>
        </div>
      )}

      <div className="flex-1 flex px-4 pb-3 min-h-0">
        <div className="flex flex-col flex-1 min-w-0">
          <ResultsList
            results={results}
            selectedIndex={selectedIndex}
            onClick={handleClickResult}
            onOpen={handleOpenResult}
            query={query}
            lastNavMethodRef={lastNavMethodRef}
          />
        </div>
      </div>

      <footer className="flex-shrink-0 px-4 pb-3">
        {!hasSearch && results.length === 0 && (
          <div className="flex items-center justify-center gap-4 text-[10px] text-[var(--text-tertiary)]">
            <span>Up/Down <span className="text-[var(--text-tertiary)]/70">navigate</span></span>
            <span>Enter <span className="text-[var(--text-tertiary)]/70">open</span></span>
            <span>Ctrl+, <span className="text-[var(--text-tertiary)]/70">settings</span></span>
          </div>
        )}
      </footer>

      <SettingsModal
        isOpen={isSettingsOpen}
        onClose={() => setIsSettingsOpen(false)}
        initialTab={settingsTab}
      />
      <ToastOverlay />
    </div>
  );
}

export default App;
