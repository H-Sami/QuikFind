import React, { useState, useEffect, useCallback } from 'react';
import { Folder, Monitor, Download, Image, Video, Music, Plus, Trash2, Check } from 'lucide-react';
import { open } from '@tauri-apps/plugin-dialog';
import { homeDir, sep } from '@tauri-apps/api/path';
import { useStore } from '../store';
import { useUIStore } from '../stores/uiStore';

interface OnboardingScreenProps {
  onComplete: () => void;
}

interface CommonFolder {
  name: string;
  path: string;
  icon: React.ReactNode;
  checked: boolean;
}

const FOLDER_CONFIGS = [
  { name: 'Documents', icon: <Folder className="w-4 h-4" /> },
  { name: 'Desktop', icon: <Monitor className="w-4 h-4" /> },
  { name: 'Downloads', icon: <Download className="w-4 h-4" /> },
  { name: 'Pictures', icon: <Image className="w-4 h-4" /> },
  { name: 'Videos', icon: <Video className="w-4 h-4" /> },
  { name: 'Music', icon: <Music className="w-4 h-4" /> },
];

const OnboardingScreen: React.FC<OnboardingScreenProps> = ({ onComplete }) => {
  const { settings, updateSettings } = useStore();
  const { showToast } = useUIStore();
  const [commonFolders, setCommonFolders] = useState<CommonFolder[]>([]);
  const [customPaths, setCustomPaths] = useState<string[]>([]);
  const [isLoading, setIsLoading] = useState(true);
  const [isSaving, setIsSaving] = useState(false);

  useEffect(() => {
    const init = async () => {
      try {
        const home = await homeDir();
        const existingPaths = settings.indexed_paths.map(p => p.toLowerCase());

        const folders = FOLDER_CONFIGS.map(config => {
          const folderPath = home + sep + config.name;
          return {
            name: config.name,
            path: folderPath,
            icon: config.icon,
            checked: false,
          };
        });

        const knownCommon = folders.map(f => f.path.toLowerCase());
        const existingCustom = settings.indexed_paths.filter(
          p => !knownCommon.includes(p.toLowerCase())
        );

        setCommonFolders(folders);
        setCustomPaths(existingCustom);
      } catch (e) {
        console.error('Failed to init onboarding:', e);
      } finally {
        setIsLoading(false);
      }
    };
    init();
  }, [settings.indexed_paths]);

  const toggleFolder = (index: number) => {
    setCommonFolders(prev => prev.map((f, i) =>
      i === index ? { ...f, checked: !f.checked } : f
    ));
  };

  const addCustomFolder = async () => {
    try {
      const selected = await open({ directory: true, multiple: false });
      if (selected && !customPaths.includes(selected)) {
        setCustomPaths(prev => [...prev, selected]);
      }
    } catch {}
  };

  const removeCustomFolder = (path: string) => {
    setCustomPaths(prev => prev.filter(p => p !== path));
  };

  const getSelectedPaths = useCallback(() => {
    const checked = commonFolders.filter(f => f.checked).map(f => f.path);
    return [...checked, ...customPaths];
  }, [commonFolders, customPaths]);

  const handleStartIndexing = async () => {
    setIsSaving(true);
    // Clear any previously saved default paths
    const paths = getSelectedPaths().filter(p => !p.includes('/Documents') &&
                                                  !p.includes('/Desktop') &&
                                                  !p.includes('/Downloads'));
    try {
      await updateSettings({
        indexed_paths: paths,
        has_completed_onboarding: true,
      });
      showToast('Indexing started in the background', 'success');
      onComplete();
    } catch (e) {
      console.error('Failed to save onboarding settings:', e);
    } finally {
      setIsSaving(false);
    }
  };

  const handleSkip = async () => {
    try {
      await updateSettings({ has_completed_onboarding: true });
      onComplete();
    } catch (e) {
      console.error('Failed to skip onboarding:', e);
    }
  };

  const selectedCount = getSelectedPaths().length;
  const canContinue = selectedCount > 0;

  if (isLoading) {
    return (
      <div className="h-screen flex items-center justify-center" style={{ background: 'var(--surface)' }}>
        <div className="w-5 h-5 border-2 border-[var(--border-default)] border-t-[var(--accent)] rounded-full animate-spin" />
      </div>
    );
  }

  return (
    <div
      className="h-screen flex flex-col items-center justify-center p-5 overflow-hidden animate-fade-in"
      style={{ background: 'var(--surface)' }}
    >
      <div
        className="w-full max-w-md animate-scale-in"
        style={{
          background: 'var(--surface-elevated)',
          borderColor: 'var(--border-default)',
          borderRadius: '1rem',
          borderWidth: '1px',
          borderStyle: 'solid',
          boxShadow: '0 16px 48px rgba(0,0,0,0.3)',
        }}
      >
        <div className="p-6">
          <div className="text-center mb-6">
            <img
              src="/logo-horizontal.png"
              alt="QuikFind"
              className="h-5 mx-auto mb-4 opacity-70 dark:invert"
            />
            <h1 className="text-2xl font-semibold text-[var(--text-primary)] tracking-tight">
              Welcome to QuikFind
            </h1>
            <p className="text-sm text-[var(--text-secondary)] mt-1.5 leading-relaxed">
              Fast, private, local search — right on your computer.
            </p>
          </div>

          <div className="border-t border-[var(--border-subtle)] mb-5" />

          <div className="mb-4">
            <h2 className="text-sm font-medium text-[var(--text-primary)]">Choose folders to index</h2>
            <p className="text-xs text-[var(--text-tertiary)] mt-0.5">
              Select the locations you want QuikFind to search. You can change this anytime in Settings.
            </p>
          </div>

          <div className="space-y-1 mb-4">
            {commonFolders.map((folder, index) => (
              <button
                key={folder.name}
                onClick={() => toggleFolder(index)}
                className="w-full flex items-center gap-3 px-3 py-2.5 rounded-xl text-xs transition-all duration-150 hover:bg-[var(--border-subtle)] group text-left"
              >
                <div
                  className={`w-5 h-5 rounded-md border-2 flex items-center justify-center transition-all duration-150 flex-shrink-0 ${
                    folder.checked
                      ? 'border-[var(--accent)] bg-[var(--accent)]'
                      : 'border-[var(--border-default)] group-hover:border-[var(--border-hover)]'
                  }`}
                >
                  {folder.checked && (
                    <Check className="w-3 h-3 text-white animate-check-pop" />
                  )}
                </div>
                <span className="text-[var(--text-tertiary)] flex-shrink-0">
                  {folder.icon}
                </span>
                <span className="text-[var(--text-primary)] font-medium">{folder.name}</span>
                <span className="text-[10px] text-[var(--text-tertiary)] truncate ml-auto font-mono hidden sm:block">
                  {folder.path}
                </span>
              </button>
            ))}
          </div>

          <div className="space-y-1 mb-4">
            <button
              onClick={addCustomFolder}
              className="flex items-center gap-2 px-3 py-2 rounded-xl text-xs font-medium text-[var(--accent)] hover:bg-[var(--accent)]/10 transition-all duration-150 w-full"
            >
              <Plus className="w-3.5 h-3.5" />
              Add Custom Folder
            </button>
            {customPaths.length === 0 && settings.indexed_paths.length > 0 && commonFolders.every(f => !f.checked) && (
              <p className="text-[11px] text-[var(--text-tertiary)] px-3 py-1">
                No folders selected. Add a custom folder or check one above.
              </p>
            )}
            {customPaths.map((path) => (
              <div
                key={path}
                className="flex items-center justify-between px-3 py-2 rounded-xl text-xs group hover:bg-[var(--border-subtle)] transition-colors duration-150"
              >
                <span className="truncate text-[var(--text-secondary)]">{path}</span>
                <button
                  onClick={() => removeCustomFolder(path)}
                  className="opacity-0 group-hover:opacity-100 p-1 rounded text-red-400 hover:text-red-500 hover:bg-red-500/10 transition-all duration-150 flex-shrink-0 ml-2"
                >
                  <Trash2 className="w-3 h-3" />
                </button>
              </div>
            ))}
          </div>

          <div className="mb-6 p-4 rounded-xl bg-[var(--surface)] border border-[var(--border-subtle)]">
            <div className="flex items-center justify-between mb-3">
              <div>
                <h3 className="text-sm font-medium text-[var(--text-primary)]">Global Hotkey</h3>
                <p className="text-xs text-[var(--text-tertiary)]">Press this combination to open QuikFind</p>
              </div>
              <div className="text-xs font-mono px-2 py-1 bg-[var(--border-subtle)] rounded">
                {settings.hotkey}
              </div>
            </div>
            <button
              onClick={() => {
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
                  updateSettings({ hotkey: newHotkey });
                  showToast(`Hotkey set to ${newHotkey}`, 'success');
                  window.removeEventListener('keydown', handleKeyDown, true);
                };
                window.addEventListener('keydown', handleKeyDown, true);
                showToast('Press your desired hotkey combination now...', 'success');
              }}
              className="w-full py-2.5 text-sm font-medium rounded-xl bg-[var(--accent)] text-white hover:opacity-90 active:opacity-80 transition-all"
            >
              Click to Record New Hotkey
            </button>
          </div>

          <div className="border-t border-[var(--border-subtle)] pt-4">
            <div className="flex items-center gap-3">
              <button
                onClick={handleSkip}
                className="text-xs text-[var(--text-tertiary)] hover:text-[var(--text-secondary)] transition-colors duration-150 px-1"
              >
                Skip for now
              </button>
              <div className="flex-1" />
              <button
                onClick={handleStartIndexing}
                disabled={!canContinue || isSaving}
                className="px-5 py-2.5 rounded-xl text-xs font-medium transition-all duration-200 disabled:opacity-40 disabled:cursor-not-allowed flex items-center gap-2 min-w-[140px] justify-center hover:scale-[1.02] active:scale-[0.98]"
                style={{
                  backgroundColor: 'var(--accent)',
                  color: '#fff',
                }}
              >
                {isSaving ? (
                  <span className="flex items-center gap-2">
                    <svg className="w-3.5 h-3.5 animate-spin" viewBox="0 0 24 24" fill="none">
                      <circle className="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" strokeWidth="4" />
                      <path className="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4z" />
                    </svg>
                    Saving...
                  </span>
                ) : (
                  <>
                    Start Indexing
                    {selectedCount > 0 && (
                      <span className="opacity-80">({selectedCount})</span>
                    )}
                  </>
                )}
              </button>
            </div>
          </div>
        </div>
      </div>
    </div>
  );
};

export default OnboardingScreen;