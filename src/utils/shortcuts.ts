export function matchShortcut(e: KeyboardEvent | React.KeyboardEvent, shortcutKeys: string): boolean {
  const parts = shortcutKeys.replace(/CmdOrCtrl/g, 'Ctrl').split('+');
  const key = parts[parts.length - 1];
  const mods = parts.slice(0, -1);

  const ctrlRequired = mods.includes('Ctrl');
  const shiftRequired = mods.includes('Shift');
  const altRequired = mods.includes('Alt');

  const ctrlPressed = e.ctrlKey || e.metaKey;
  const shiftPressed = e.shiftKey;
  const altPressed = e.altKey;

  if (ctrlRequired !== ctrlPressed) return false;
  if (shiftRequired !== shiftPressed) return false;
  if (altRequired !== altPressed) return false;

  const eventKey = e.key;
  switch (key) {
    case '↑': return eventKey === 'ArrowUp';
    case '↓': return eventKey === 'ArrowDown';
    case '?': return eventKey === '/' || eventKey === '?';
    case ',': return eventKey === ',';
    case 'Esc': return eventKey === 'Escape';
    default: return eventKey.toLowerCase() === key.toLowerCase();
  }
}
