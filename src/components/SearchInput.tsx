import React, { forwardRef } from 'react';
import { Search, Loader2, X } from 'lucide-react';

interface SearchInputProps {
  value: string;
  onChange: (value: string) => void;
  onKeyDown: (e: React.KeyboardEvent<HTMLInputElement>) => void;
  isLoading: boolean;
}

const SearchInput = forwardRef<HTMLInputElement, SearchInputProps>(
  ({ value, onChange, onKeyDown, isLoading }, ref) => {
    return (
      <div className="relative group">
        <div className="absolute left-3 top-1/2 -translate-y-1/2 text-[var(--text-tertiary)] pointer-events-none group-focus-within:text-[var(--accent)]">
          <Search className="w-3.5 h-3.5" />
        </div>

        <input
          ref={ref}
          type="text"
          value={value}
          onChange={(e) => onChange(e.target.value)}
          onKeyDown={onKeyDown}
          onMouseDown={(e) => e.stopPropagation()}
          placeholder="Search..."
          className="w-full h-9 bg-[var(--surface-elevated)] border border-[var(--border-default)] focus:border-[var(--accent)]/50 text-sm pl-8 pr-8 rounded-xl outline-none text-[var(--text-primary)] placeholder:text-[var(--text-tertiary)] focus:ring-2 focus:ring-[var(--accent)]/10"
          autoFocus
          spellCheck={false}
          autoComplete="off"
        />

        {isLoading && (
          <div className="absolute right-3 top-1/2 -translate-y-1/2">
            <Loader2 className="w-3 h-3 animate-spin text-[var(--accent)]/60" />
          </div>
        )}

        {!isLoading && value && (
          <button
            onClick={() => onChange('')}
            className="absolute right-2 top-1/2 -translate-y-1/2 p-0.5 rounded text-[var(--text-tertiary)] hover:text-[var(--text-secondary)]"
          >
            <X className="w-3 h-3" />
          </button>
        )}
      </div>
    );
  }
);

SearchInput.displayName = 'SearchInput';

export default SearchInput;
