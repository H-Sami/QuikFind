import { useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { SearchResult } from '../types';
import { useUIStore } from '../stores/uiStore';

export function useOpenResult() {
  const showToast = useUIStore((state) => state.showToast);

  return useCallback(async (result: SearchResult) => {
    try {
      if (result.kind === 'App') {
        await invoke('launch_app', { appId: result.id });
      } else {
        await invoke('open_path', { path: result.path });
      }

      await getCurrentWindow().hide();
    } catch (error) {
      console.error('Failed to open result:', { result, error });
      showToast(`Failed to open ${result.name}`, 'error');
    }
  }, [showToast]);
}
