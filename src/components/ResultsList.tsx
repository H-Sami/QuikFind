import React, { useEffect, useMemo, useRef } from 'react';
import { AppWindow, FileText, Folder } from 'lucide-react';
import { SearchResult } from '../types';

interface ResultsListProps {
  results: SearchResult[];
  selectedIndex: number;
  onClick: (index: number) => void;
  onOpen: (result: SearchResult) => void;
  query: string;
  lastNavMethodRef: { current: 'keyboard' | 'mouse' };
}

const getIcon = (kind: string) => {
  switch (kind) {
    case 'Folder': return <Folder className="w-4 h-4" />;
    case 'App': return <AppWindow className="w-4 h-4" />;
    default: return <FileText className="w-4 h-4" />;
  }
};

const getIconBg = (kind: string) => {
  switch (kind) {
    case 'Folder': return 'bg-blue-500/10 text-blue-500';
    case 'App': return 'bg-emerald-500/10 text-emerald-500';
    default: return 'bg-[var(--border-default)] text-[var(--text-secondary)]';
  }
};

const formatSize = (size?: number) => {
  if (!size) return '';
  const mb = size / (1024 * 1024);
  if (mb >= 1) return `${mb.toFixed(1)} MB`;
  const kb = size / 1024;
  return `${kb.toFixed(0)} KB`;
};

function highlightMatch(text: string, query: string) {
  if (!query.trim()) return text;
  const escaped = query.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
  const parts = text.split(new RegExp(`(${escaped})`, 'gi'));
  return parts.map((part, i) =>
    part.toLowerCase() === query.toLowerCase()
      ? <span key={i} className="text-[var(--accent)] font-medium">{part}</span>
      : part
  );
}

interface ResultItemProps {
  result: SearchResult;
  index: number;
  isSelected: boolean;
  query: string;
  onClick: (index: number) => void;
  onOpen: (result: SearchResult) => void;
}

const ResultItem = React.memo(React.forwardRef<HTMLDivElement, ResultItemProps>(function ResultItem({
  result,
  index,
  isSelected,
  query,
  onClick,
  onOpen,
}, ref) {
  const nameHighlighted = useMemo(
    () => highlightMatch(result.name, query),
    [result.name, query]
  );

  return (
    <div
      ref={ref}
      onClick={() => onClick(index)}
      onDoubleClick={() => onOpen(result)}
      className={`group flex items-center gap-2.5 px-3.5 py-2.5 cursor-pointer rounded-xl transition-[background,border-color,box-shadow] duration-150 ${
        isSelected
          ? 'bg-[var(--accent)]/8 border border-[var(--accent)]/20 ring-1 ring-[var(--accent)]/40'
          : 'border border-transparent hover:bg-[var(--border-subtle)] hover:border-[var(--border-default)]'
      }`}
    >
      <div className={`flex-shrink-0 w-8 h-8 rounded-xl flex items-center justify-center ${getIconBg(result.kind)}`}>
        {getIcon(result.kind)}
      </div>

      <div className="flex-1 min-w-0">
        <div className="flex items-center gap-2">
          <span className="font-medium truncate text-[13px] text-[var(--text-primary)]">
            {nameHighlighted}
          </span>
          {result.kind === 'App' && (
            <span className="text-[10px] px-1.5 py-0.5 rounded-md bg-emerald-500/15 text-emerald-500 font-medium flex-shrink-0">
              APP
            </span>
          )}
        </div>
        <div className="flex items-center gap-1.5 text-xs text-[var(--text-tertiary)] truncate mt-0.5">
          <span className="truncate">{result.path}</span>
          {result.size && result.size > 0 && (
            <span className="flex-shrink-0">- {formatSize(result.size)}</span>
          )}
        </div>
      </div>

      <div className="flex-shrink-0 text-right">
        <div className="text-[10px] text-[var(--text-tertiary)]/60 tabular-nums font-mono">
          {(result.score * 100).toFixed(0)}
        </div>
      </div>
    </div>
  );
}));

const ResultsList: React.FC<ResultsListProps> = ({
  results,
  selectedIndex,
  onClick,
  onOpen,
  query,
  lastNavMethodRef,
}) => {
  const itemRefs = useRef<(HTMLDivElement | null)[]>([]);

  const emptyResults = useMemo(() => {
    if (results.length === 0 && !query.trim()) {
      return (
        <div className="flex items-center justify-center h-full w-full text-center px-8">
          <div>
            <div className="text-2xl font-semibold text-white/90 mb-3">
              Start typing to search
            </div>
            <div className="max-w-xs text-sm text-white/50 leading-relaxed">
              Apps and files will appear here.
            </div>
          </div>
        </div>
      );
    }
    if (results.length === 0) {
      return (
        <div className="flex flex-col items-center justify-center h-full w-full text-center p-8">
          <div className="text-lg font-medium text-white/70">No results found</div>
          <div className="text-sm text-white/40 mt-1">Try a different search term</div>
        </div>
      );
    }
    return null;
  }, [query, results.length]);

  useEffect(() => {
    if (lastNavMethodRef.current !== 'keyboard') return;
    const el = itemRefs.current[selectedIndex];
    el?.scrollIntoView({ behavior: 'auto', block: 'nearest' });
  }, [selectedIndex, lastNavMethodRef]);

  if (emptyResults) return emptyResults;

  return (
    <div className="flex-1 flex flex-col min-h-0">
      <div
        className="flex-1 overflow-y-auto custom-scrollbar -mx-1 px-1"
        onMouseEnter={() => { lastNavMethodRef.current = 'mouse'; }}
      >
        <div className="space-y-0.5">
          {results.map((result, index) => (
            <ResultItem
              key={`${result.kind}:${result.id}`}
              ref={(el) => (itemRefs.current[index] = el)}
              result={result}
              index={index}
              isSelected={selectedIndex === index}
              query={query}
              onClick={(i) => {
                lastNavMethodRef.current = 'mouse';
                onClick(i);
              }}
              onOpen={onOpen}
            />
          ))}
        </div>
      </div>
    </div>
  );
};

export default React.memo(ResultsList);
