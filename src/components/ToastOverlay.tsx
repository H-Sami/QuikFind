import { useUIStore } from '../stores/uiStore';

export default function ToastOverlay() {
  const { toast } = useUIStore();
  return (
    <div className={`fixed bottom-6 left-1/2 -translate-x-1/2 z-[60] transition-all duration-300 ease-out ${toast.visible ? 'opacity-100 translate-y-0' : 'opacity-0 translate-y-4 pointer-events-none'}`}>
      <div className={`px-4 py-2.5 rounded-2xl text-xs font-medium shadow-soft-lg backdrop-blur-md animate-bounce-in ${toast.type === 'success' ? 'bg-emerald-500/15 text-emerald-500 border border-emerald-500/20' : toast.type === 'error' ? 'bg-red-500/15 text-red-500 border border-red-500/20' : 'bg-[var(--accent)]/15 text-[var(--accent)] border border-[var(--accent)]/20'}`}>
        <div className="flex items-center gap-2">
          {toast.type === 'success' && (
            <svg className="w-3.5 h-3.5 animate-check-pop" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.5" strokeLinecap="round" strokeLinejoin="round">
              <polyline points="20 6 9 17 4 12" />
            </svg>
          )}
          {toast.message}
        </div>
      </div>
    </div>
  );
}
