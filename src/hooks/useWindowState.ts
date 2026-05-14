import { useEffect } from 'react';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { invoke } from '@tauri-apps/api/core';

interface WindowState {
  x: number;
  y: number;
  width: number;
  height: number;
}

export function useWindowState() {
  useEffect(() => {
    const win = getCurrentWindow();

    const saveState = async () => {
      try {
        const [pos, size] = await Promise.all([
          win.outerPosition(),
          win.outerSize()
        ]);

        const state: WindowState = {
          x: pos.x,
          y: pos.y,
          width: size.width,
          height: size.height,
        };

        await invoke('save_window_state', { json: JSON.stringify(state) });
      } catch (e) {
        console.error('Failed to save window state:', e);
      }
    };

    let timeout: ReturnType<typeof setTimeout>;
    const handleMoveOrResize = () => {
      clearTimeout(timeout);
      timeout = setTimeout(saveState, 300);
    };

    win.listen('tauri://move', handleMoveOrResize);
    win.listen('tauri://resize', handleMoveOrResize);

    return () => {
      clearTimeout(timeout);
    };
  }, []);
}
