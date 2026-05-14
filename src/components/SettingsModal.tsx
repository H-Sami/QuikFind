import React, { useState, useEffect, useCallback, useRef } from 'react';
import { X, Monitor, Moon, Sun, LayoutGrid, Rows, Keyboard, Check, RotateCcw } from 'lucide-react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { useStore } from '../store';
import { AppSettings, IndexStatus } from '../types';
import { useUIStore, ACCENT_PRESETS, ThemeMode, Density } from '../stores/uiStore';

interface SettingsModalProps {
  isOpen: boolean;
  onClose: () => void;
  initialTab?: 'appearance' | 'shortcuts';
}

const SectionTitle: React.FC<{ title: string; subtitle?: string }> = ({ title, subtitle }) => (
  <div className="mb-3">
    <h3 className="text-sm font-medium text-[var(--text-primary)]">{title}</h3>
    {subtitle && <p className="text-xs text-[var(--text-tertiary)] mt-0.5">{subtitle}</p>}
  </div>
);

function formatShortcutForDisplay(keys: string): React.ReactNode {
  if (!keys) return null;
  const parts = keys.split('+');
  return (
    <span className="flex items-center gap-1">
      {parts.map((part, i) => (
        <React.Fragment key={i}>
          {i > 0 && <span className="text-[var(--text-tertiary)]/50">+</span>}
          <kbd className="px-1.5 py-0.5 rounded-md text-[10px] font-mono font-medium bg-[var(--border-subtle)] border border-[var(--border-default)] text-[var(--text-secondary)] min-w-[20px] text-center leading-none">
            {part === '\u2191' ? '\u2191' : part === '\u2193' ? '\u2193' : part}
          </kbd>
        </React.Fragment>
      ))}
    </span>
  );
}

const SettingsModal: React.FC<SettingsModalProps> = ({ isOpen, onClose, initialTab = 'appearance' }) => {
  const { settings, updateSettings, loadSettings } = useStore();
  const {
    theme, setTheme,
    accentColor, setAccentColor,
    density, setDensity,
    keyboardShortcuts, updateShortcut, resetShortcuts, showToast,
  } = useUIStore();

  const [localSettings, setLocalSettings] = useState<AppSettings>(settings);
  const [isSaving, setIsSaving] = useState(false);
  const [saveSuccess, setSaveSuccess] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [activeTab, setActiveTab] = useState<'appearance' | 'shortcuts'>(initialTab);
  const [recordingId, setRecordingId] = useState<string | null>(null);
  const [indexStatus, setIndexStatus] = useState<IndexStatus | null>(null);
  const [isReindexing, setIsReindexing] = useState(false);

  const modalRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (isOpen) {
      loadSettings().then(() => {
        const fresh = useStore.getState().settings;
        setLocalSettings(fresh);
        setError(null);
        setSaveSuccess(false);
        setIsSaving(false);
        setActiveTab(initialTab);
        setRecordingId(null);
      });
    }
  }, [isOpen, initialTab]);

  useEffect(() => {
    if (!isOpen) return;

    invoke<IndexStatus>('get_index_status')
      .then(setIndexStatus)
      .catch(() => {});

    const unlisten = listen<IndexStatus>('index-progress', (event) => {
      setIndexStatus(event.payload);
      if (!event.payload.is_indexing) {
        setIsReindexing(false);
      }
    });

    return () => { unlisten.then(f => f()); };
  }, [isOpen]);

  useEffect(() => {
    if (!recordingId) return;
    const handleRecordKey = (e: KeyboardEvent) => {
      e.preventDefault();
      e.stopPropagation();
      const key = e.key;
      if (key === 'Escape') {
        setRecordingId(null);
        return;
      }
      const mods: string[] = [];
      if (e.ctrlKey || e.metaKey) mods.push('Ctrl');
      if (e.shiftKey) mods.push('Shift');
      if (e.altKey) mods.push('Alt');
      const mainKey = (() => {
        switch (key) {
          case 'ArrowUp': return '\u2191';
          case 'ArrowDown': return '\u2193';
          case 'Escape': return null;
          case 'Control': case 'Shift': case 'Alt': case 'Meta': return null;
          default: return key === '?' ? '?' : key;
        }
      })();
      if (!mainKey) return;
      if (mods.length === 0 && mainKey.length > 1 && !['\u2191', '\u2193'].includes(mainKey)) return;
      const shortcutStr = [...mods, mainKey].join('+');
      const existing = keyboardShortcuts.find(s => s.keys === shortcutStr && s.id !== recordingId);
      if (existing) {
        setRecordingId(null);
        showToast(`"${shortcutStr}" is already used by "${existing.label}"`, 'error');
        return;
      }
      updateShortcut(recordingId, shortcutStr);
      setRecordingId(null);
      showToast(`Shortcut updated to ${shortcutStr}`, 'success');
    };
    window.addEventListener('keydown', handleRecordKey, true);
    return () => window.removeEventListener('keydown', handleRecordKey, true);
  }, [recordingId, keyboardShortcuts, updateShortcut, showToast]);

  const saveImmediately = useCallback(async (partial: Partial<AppSettings>) => {
    const updated = { ...localSettings, ...partial };
    setLocalSettings(updated);

    try {
      await updateSettings(updated);
      showToast('Settings updated', 'success');
    } catch (err) {
      showToast('Failed to save settings', 'error');
    }
  }, [localSettings, updateSettings, showToast]);

  const handleReindex = useCallback(async () => {
    setIsReindexing(true);
    try {
      await invoke('start_indexing', { paths: [] });
    } catch (err) {
      showToast('Failed to start indexing', 'error');
      setIsReindexing(false);
    }
  }, [showToast]);

  const handleStopIndex = useCallback(async () => {
    try {
      await invoke('stop_indexing');
      setIsReindexing(false);
    } catch {
      // stop_indexing returns error if nothing is running — ignore it
    }
  }, []);

  const handleSave = useCallback(async () => {
    setIsSaving(true);
    setError(null);
    setSaveSuccess(false);
    try {
      await updateSettings(localSettings);
      await loadSettings();
      setSaveSuccess(true);
      setTimeout(() => { onClose(); setIsSaving(false); }, 600);
    } catch (err) {
      setError(String(err));
      setIsSaving(false);
    }
  }, [localSettings, updateSettings, loadSettings, onClose]);

  if (!isOpen) return null;

  const themeOptions: { value: ThemeMode; label: string; icon: React.ReactNode }[] = [
    { value: 'dark', label: 'Dark', icon: <Moon className="w-4 h-4" /> },
    { value: 'light', label: 'Light', icon: <Sun className="w-4 h-4" /> },
  ];

  const densityOptions: { value: Density; label: string; desc: string; icon: React.ReactNode }[] = [
    { value: 'spacious', label: 'Spacious', desc: 'More breathing room', icon: <Rows className="w-4 h-4" /> },
    { value: 'balanced', label: 'Balanced', desc: 'Show more results', icon: <LayoutGrid className="w-4 h-4" /> },
  ];

  const categories = [...new Set(keyboardShortcuts.map(s => s.category))];

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center" onClick={onClose}>
      <div className="absolute inset-0 bg-black/40 backdrop-blur-sm" />
      <div
        ref={modalRef}
        className="relative w-full max-w-lg max-h-[85vh] overflow-hidden animate-scale-in"
        onClick={e => e.stopPropagation()}
        style={{
          background: 'var(--surface-elevated)',
          borderColor: 'var(--border-default)',
          borderRadius: '1rem',
          borderWidth: '1px',
          borderStyle: 'solid',
          boxShadow: '0 16px 48px rgba(0,0,0,0.3)',
        }}
      >
        <div className="flex items-center justify-between px-5 py-4 border-b border-[var(--border-subtle)]">
          <h2 className="text-base font-semibold text-[var(--text-primary)]">Settings</h2>
          <button
            onClick={onClose}
            className="p-1.5 rounded-lg hover:bg-[var(--border-default)] text-[var(--text-tertiary)] hover:text-[var(--text-primary)] transition-colors duration-150"
          >
            <X className="w-4 h-4" />
          </button>
        </div>

        <div className="flex border-b border-[var(--border-subtle)] px-5">
          <button
            onClick={() => setActiveTab('appearance')}
            className={`py-3 px-1 text-xs font-medium border-b-2 transition-all duration-150 -mb-px ${
              activeTab === 'appearance'
                ? 'border-[var(--accent)] text-[var(--text-primary)]'
                : 'border-transparent text-[var(--text-tertiary)] hover:text-[var(--text-secondary)]'
            }`}
          >
            <Monitor className="w-3.5 h-3.5 inline mr-1.5 -mt-0.5" />
            General
          </button>
          <button
            onClick={() => setActiveTab('shortcuts')}
            className={`py-3 px-1 text-xs font-medium border-b-2 transition-all duration-150 -mb-px ml-5 ${
              activeTab === 'shortcuts'
                ? 'border-[var(--accent)] text-[var(--text-primary)]'
                : 'border-transparent text-[var(--text-tertiary)] hover:text-[var(--text-secondary)]'
            }`}
          >
            <Keyboard className="w-3.5 h-3.5 inline mr-1.5 -mt-0.5" />
            Keyboard Shortcuts
          </button>
        </div>

        <div className="overflow-y-auto custom-scrollbar" style={{ maxHeight: 'calc(85vh - 120px)' }}>
          <div className="p-5">
            {activeTab === 'appearance' && (
              <div className="space-y-6">
                <div>
                  <SectionTitle title="Appearance" subtitle="Customize the look and feel" />
                  <div className="mb-4">
                    <label className="text-xs font-medium text-[var(--text-secondary)] mb-2 block">Theme</label>
                    <div className="flex gap-2">
                      {themeOptions.map(opt => (
                        <button
                          key={opt.value}
                          onClick={() => setTheme(opt.value)}
                          className={`flex-1 flex items-center justify-center gap-2 py-2.5 px-3 rounded-xl text-xs font-medium transition-all duration-200 ${
                            theme === opt.value
                              ? 'bg-[var(--accent)]/15 text-[var(--accent)] border border-[var(--accent)]/25'
                              : 'bg-[var(--border-subtle)] text-[var(--text-secondary)] border border-transparent hover:bg-[var(--border-default)] hover:text-[var(--text-primary)]'
                          }`}
                        >
                          {opt.icon}
                          {opt.label}
                        </button>
                      ))}
                    </div>
                  </div>
                  <div className="mb-4">
                    <label className="text-xs font-medium text-[var(--text-secondary)] mb-2 block">Accent Color</label>
                    <div className="flex flex-wrap gap-2 mb-2.5">
                      {ACCENT_PRESETS.map(color => (
                        <button
                          key={color}
                          onClick={() => setAccentColor(color)}
                          className={`w-7 h-7 rounded-xl transition-all duration-200 ${
                            accentColor === color
                              ? 'ring-2 ring-offset-2 ring-offset-[var(--surface-elevated)] scale-110'
                              : 'hover:scale-110'
                          }`}
                          style={{ backgroundColor: color }}
                          title={color}
                        />
                      ))}
                    </div>
                    <div className="flex items-center gap-3">
                      <div className="relative flex-1">
                        <input
                          type="color"
                          value={accentColor}
                          onChange={(e) => setAccentColor(e.target.value)}
                          className="w-full h-8 cursor-pointer rounded-lg"
                        />
                      </div>
                      <span className="text-xs font-mono text-[var(--text-tertiary)] flex-shrink-0">
                        {accentColor.toUpperCase()}
                      </span>
                    </div>
                  </div>
                </div>

                <div>
                  <SectionTitle title="Density" subtitle="How much content to show" />
                  <div className="flex gap-2">
                    {densityOptions.map(opt => (
                      <button
                        key={opt.value}
                        onClick={() => setDensity(opt.value)}
                        className={`flex-1 flex flex-col items-center gap-1.5 py-3 px-3 rounded-xl text-xs transition-all duration-200 ${
                          density === opt.value
                            ? 'bg-[var(--accent)]/15 text-[var(--accent)] border border-[var(--accent)]/25'
                            : 'bg-[var(--border-subtle)] text-[var(--text-secondary)] border border-transparent hover:bg-[var(--border-default)] hover:text-[var(--text-primary)]'
                        }`}
                      >
                        {opt.icon}
                        <span className="font-medium">{opt.label}</span>
                        <span className="text-[10px] opacity-70">{opt.desc}</span>
                      </button>
                    ))}
                  </div>
                </div>

                <div>
                  <SectionTitle title="Search Behavior" />
                  <div className="space-y-4">
                    <div className="flex items-center justify-between">
                      <div>
                        <div className="text-xs text-[var(--text-primary)]">Content search</div>
                        <div className="text-[11px] text-[var(--text-tertiary)]">Search inside PDFs, code, docs</div>
                      </div>
                      <label className="relative inline-flex items-center cursor-pointer">
                        <input
                          type="checkbox"
                          checked={localSettings.enable_content_search}
                          onChange={(e) => setLocalSettings(prev => ({...prev, enable_content_search: e.target.checked}))}
                          className="sr-only peer"
                        />
                        <div className="w-9 h-5 bg-[var(--border-default)] rounded-full peer peer-checked:after:translate-x-full after:content-[''] after:absolute after:top-[2px] after:left-[2px] after:bg-white after:rounded-full after:h-4 after:w-4 after:transition-all duration-200 peer-checked:bg-[var(--accent)]" />
                      </label>
                    </div>
                    <div>
                      <div className="flex justify-between text-xs mb-1.5">
                        <span className="text-[var(--text-primary)]">Max results</span>
                        <span className="font-mono text-[var(--accent)]">{localSettings.max_results}</span>
                      </div>
                      <input
                        type="range"
                        min="10"
                        max="50"
                        step="5"
                        value={localSettings.max_results}
                        onChange={(e) => {
                          const newValue = parseInt(e.target.value);
                          saveImmediately({ max_results: newValue });
                        }}
                        className="w-full"
                        style={{ accentColor: 'var(--accent)' }}
                      />
                    </div>
                  </div>
                </div>

                <div className="border-t border-[var(--border-subtle)]" />

                <div>
                  <SectionTitle
                    title="Startup"
                    subtitle="Launch QuikFind automatically when Windows starts"
                  />

                  <div className="flex items-center justify-between py-2">
                    <div>
                      <div className="text-xs text-[var(--text-primary)]">Launch on Windows startup</div>
                      <div className="text-[11px] text-[var(--text-tertiary)]">
                        QuikFind will start minimized in the system tray
                      </div>
                    </div>

                    <label className="relative inline-flex items-center cursor-pointer">
                      <input
                        type="checkbox"
                        checked={localSettings.launch_on_startup ?? false}
                        onChange={async (e) => {
                          const enabled = e.target.checked;

                          try {
                            await invoke('set_autostart', { enabled });

                            // Update local + global settings
                            const updated = { ...localSettings, launch_on_startup: enabled };
                            setLocalSettings(updated);
                            await updateSettings(updated);

                            showToast(
                              enabled ? 'QuikFind will launch on startup' : 'Startup launch disabled',
                              'success'
                            );
                          } catch (err) {
                            showToast('Failed to change startup setting', 'error');
                            console.error(err);
                          }
                        }}
                        className="sr-only peer"
                      />
                      <div className="w-9 h-5 bg-[var(--border-default)] rounded-full peer peer-checked:after:translate-x-full after:content-[''] after:absolute after:top-[2px] after:left-[2px] after:bg-white after:rounded-full after:h-4 after:w-4 after:transition-all duration-200 peer-checked:bg-[var(--accent)]" />
                    </label>
                  </div>
                </div>

                <div className="border-t border-[var(--border-subtle)]" />

                <div>
                  <SectionTitle
                    title="Search Index"
                    subtitle="QuikFind indexes your entire PC for instant search"
                  />

                  <div className="flex items-center justify-between py-2 mb-3">
                    <div className="flex items-center gap-2">
                      <span
                        className={`w-2 h-2 rounded-full flex-shrink-0 ${
                          indexStatus?.is_indexing
                            ? 'bg-amber-400 animate-pulse'
                            : 'bg-emerald-500'
                        }`}
                      />
                      <div>
                        <div className="text-xs text-[var(--text-primary)]">
                          {indexStatus?.is_indexing
                            ? 'Indexing in progress...'
                            : 'Index ready'}
                        </div>
                        <div className="text-[11px] text-[var(--text-tertiary)]">
                          {indexStatus
                            ? `${indexStatus.files_indexed.toLocaleString()} files indexed`
                            : 'Loading...'}
                        </div>
                      </div>
                    </div>

                    {indexStatus?.is_indexing ? (
                      <button
                        onClick={handleStopIndex}
                        className="px-3 py-1.5 text-xs font-medium rounded-lg bg-red-500/10 text-red-400 hover:bg-red-500/20 transition-colors"
                      >
                        Stop
                      </button>
                    ) : (
                      <button
                        onClick={handleReindex}
                        disabled={isReindexing}
                        className="px-3 py-1.5 text-xs font-medium rounded-lg bg-[var(--accent)]/10 text-[var(--accent)] hover:bg-[var(--accent)]/20 transition-colors disabled:opacity-50"
                      >
                        Re-Index All Drives
                      </button>
                    )}
                  </div>

                  {indexStatus?.is_indexing && (
                    <div className="space-y-1">
                      <div className="flex-1 h-1.5 bg-[var(--border-subtle)] rounded-full overflow-hidden">
                        <div
                          className="h-full rounded-full transition-all duration-500"
                          style={{
                            width: `${Math.min(indexStatus.progress_percent, 100)}%`,
                            backgroundColor: 'var(--accent)',
                          }}
                        />
                      </div>
                      <div className="flex justify-between text-[10px] text-[var(--text-tertiary)] tabular-nums">
                        <span>{indexStatus.files_indexed.toLocaleString()} files</span>
                        <span>{Math.round(indexStatus.progress_percent)}%</span>
                      </div>
                    </div>
                  )}

                  {indexStatus && indexStatus.errors.length > 0 && (
                    <div className="mt-2 text-[10px] text-amber-400/70">
                      {indexStatus.errors.length} path(s) had errors during last index
                    </div>
                  )}
                </div>

                <div className="border-t border-[var(--border-subtle)]" />

                <div>
                    <button
                        onClick={() => saveImmediately({})}
                    className="w-full py-2 text-xs font-medium bg-[var(--accent)]/10 text-[var(--accent)] rounded-xl hover:bg-[var(--accent)]/20 transition-colors mb-3"
                  >
                    Apply Changes Now
                  </button>

                </div>
              </div>
            )}

            {activeTab === 'shortcuts' && (
              <div className="space-y-5 animate-fade-in">
                <SectionTitle title="Keyboard Shortcuts" subtitle="Customize your keyboard shortcuts. Click a shortcut to remap it." />
                {categories.map(category => (
                  <div key={category}>
                    <h4 className="text-[10px] font-semibold uppercase tracking-wider text-[var(--text-tertiary)] mb-2">
                      {category}
                    </h4>
                    <div className="space-y-1">
                      {keyboardShortcuts
                        .filter(s => s.category === category)
                        .map(shortcut => {
                          const isRecording = recordingId === shortcut.id;
                          return (
                            <div
                              key={shortcut.id}
                              className={`flex items-center justify-between px-3 py-2.5 rounded-xl text-xs transition-all duration-150 ${
                                isRecording
                                  ? 'bg-[var(--accent)]/10 border border-[var(--accent)]/25'
                                  : 'hover:bg-[var(--border-subtle)] border border-transparent'
                              }`}
                            >
                              <div className="min-w-0">
                                <div className="text-[var(--text-primary)] font-medium">{shortcut.label}</div>
                                <div className="text-[10px] text-[var(--text-tertiary)] mt-0.5">{shortcut.description}</div>
                              </div>
                              <button
                                onClick={() => setRecordingId(isRecording ? null : shortcut.id)}
                                className={`flex-shrink-0 ml-3 transition-all duration-150 ${
                                  isRecording ? 'cursor-default' : 'hover:scale-105 active:scale-95'
                                }`}
                              >
                                {isRecording ? (
                                  <div className="flex items-center gap-2 px-3 py-1.5 rounded-lg bg-[var(--accent)]/15 border border-[var(--accent)]/30">
                                    <span className="w-1.5 h-1.5 rounded-full bg-[var(--accent)] recording-pulse" />
                                    <span className="text-[10px] font-medium text-[var(--accent)]">Press shortcut...</span>
                                  </div>
                                ) : (
                                  <div className="flex items-center gap-1.5 px-2.5 py-1.5 rounded-lg bg-[var(--border-subtle)] border border-[var(--border-default)] group-hover:bg-[var(--border-default)] transition-colors duration-150">
                                    {formatShortcutForDisplay(shortcut.keys)}
                                  </div>
                                )}
                              </button>
                            </div>
                          );
                        })}
                    </div>
                  </div>
                ))}
                <div className="border-t border-[var(--border-subtle)] pt-4 flex justify-center">
                  <button
                    onClick={resetShortcuts}
                    className="flex items-center gap-2 px-4 py-2 rounded-xl text-xs font-medium text-[var(--text-secondary)] hover:bg-[var(--border-subtle)] hover:text-[var(--text-primary)] transition-all duration-150"
                  >
                    <RotateCcw className="w-3.5 h-3.5" />
                    Reset to Defaults
                  </button>
                </div>

                <div className="border-t border-[var(--border-subtle)] pt-5 mt-5">
                  <SectionTitle 
                    title="Global Hotkey" 
                    subtitle="This combination opens QuikFind from anywhere, even when minimized" 
                  />
                  
                  <div className="p-4 rounded-2xl bg-[var(--surface)] border border-[var(--border-subtle)] flex items-center justify-between">
                    <div>
                      <div className="text-sm font-semibold text-[var(--text-primary)]">Current Hotkey</div>
                      <div className="font-mono text-xs text-[var(--accent)] mt-1">{localSettings.hotkey}</div>
                    </div>
                    
                    <button
                      onClick={() => {
                        showToast('Press your desired hotkey combination now...', 'success');
                        
                        const handleKeyDown = (e: KeyboardEvent) => {
                          e.preventDefault();
                          e.stopPropagation();
                          
                          const mods: string[] = [];
                          if (e.ctrlKey || e.metaKey) mods.push('CmdOrCtrl');
                          if (e.shiftKey) mods.push('Shift');
                          if (e.altKey) mods.push('Alt');
                          
                          let key = e.key;
                          if (key === ' ') key = 'Space';
                          if (key.length === 1) key = key.toUpperCase();
                          
                          const newHotkey = [...mods, key].join('+');
                          
                          const updated = { ...localSettings, hotkey: newHotkey };
                          setLocalSettings(updated);
                          saveImmediately({ hotkey: newHotkey });
                          
                          window.removeEventListener('keydown', handleKeyDown, true);
                          showToast(`Hotkey successfully changed to ${newHotkey}`, 'success');
                        };
                        
                        window.addEventListener('keydown', handleKeyDown, true);
                      }}
                      className="px-5 py-2.5 text-xs font-semibold rounded-xl bg-[var(--accent)] text-white hover:opacity-90 active:opacity-80 transition-all flex items-center gap-2"
                    >
                      Change Hotkey
                    </button>
                  </div>
                </div>
              </div>
            )}
          </div>
        </div>

        <div className="flex items-center justify-end gap-2.5 px-5 py-4 border-t border-[var(--border-subtle)]">
          <button
            onClick={onClose}
            disabled={isSaving}
            className="px-4 py-2 rounded-xl text-xs font-medium text-[var(--text-secondary)] hover:bg-[var(--border-default)] hover:text-[var(--text-primary)] transition-colors duration-150 disabled:opacity-50"
          >
            Cancel
          </button>
          <button
            onClick={handleSave}
            disabled={isSaving}
            className="px-5 py-2 rounded-xl text-xs font-medium transition-all duration-200 disabled:opacity-50 flex items-center gap-2 min-w-[120px] justify-center"
            style={{ backgroundColor: 'var(--accent)', color: '#fff' }}
          >
            {saveSuccess ? (
              <span className="flex items-center gap-1.5">
                <Check className="w-3.5 h-3.5" />
                Saved
              </span>
            ) : isSaving ? (
              <span className="flex items-center gap-2">
                <svg className="w-3.5 h-3.5 animate-spin" viewBox="0 0 24 24" fill="none">
                  <circle className="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" strokeWidth="4" />
                  <path className="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4z" />
                </svg>
                Saving...
              </span>
            ) : (
              'Save Changes'
            )}
          </button>
        </div>
      </div>
    </div>
  );
};

export default SettingsModal;
