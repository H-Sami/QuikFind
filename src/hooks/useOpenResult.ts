import { useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { SearchResult } from '../types';
import { useStore } from '../store';

export function useOpenResult() {
  const { addToHistory } = useStore();
  return useCallback(async (result: SearchResult) => {
    try {
      await invoke('open_path', { path: result.path });
      await addToHistory({
        id: result.id,
        path: result.path,
        name: result.name,
        kind: result.kind,
        opened_at: Date.now(),
      });
      const win = getCurrentWindow();
      await win.hide();
    } catch {}
  }, [addToHistory]);
}
