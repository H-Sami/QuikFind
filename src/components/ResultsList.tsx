import React, { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { FileText, Folder, AppWindow } from 'lucide-react';
import { SearchResult } from '../types';
import { VIRTUALIZATION_THRESHOLD } from '../hooks/constants';

interface ResultsListProps {
  results: SearchResult[];
  selectedIndex: number;
  onClick: (index: number) => void;
  onOpen: (result: SearchResult) => void;
  query: string;
  lastNavMethodRef: { current: 'keyboard' | 'mouse' };
}

const ITEM_HEIGHT = 60;

const OVERSCAN = 5;

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
    case 'App': return 'bg-purple-500/10 text-purple-500';
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
  const itemPadding = 'px-3.5 py-2.5';
  const itemGap = 'gap-2.5';
  const iconSize = 'w-8 h-8';

  const nameHighlighted = useMemo(
    () => highlightMatch(result.name, query),
    [result.name, query]
  );

  return (
    <div
      onClick={() => { onClick(index); onOpen(result); }}
      ref={ref}
      className={`group flex items-center ${itemGap} ${itemPadding} cursor-pointer rounded-xl transition-[transform,box-shadow] duration-150 ${
        isSelected
          ? 'bg-[var(--accent)]/8 border border-[var(--accent)]/20 ring-1 ring-[var(--accent)]/40'
          : 'border border-transparent hover:bg-[var(--border-subtle)] hover:border-[var(--border-default)]'
      }`}
    >
      <div className={`flex-shrink-0 ${iconSize} rounded-xl flex items-center justify-center ${getIconBg(result.kind)}`}>
        {getIcon(result.kind)}
      </div>

      <div className="flex-1 min-w-0">
        <div className="flex items-center gap-2">
          <span className="font-medium truncate text-[13px] text-[var(--text-primary)]">
            {nameHighlighted}
          </span>
          {result.kind === 'App' && (
            <span className="text-[10px] px-1.5 py-0.5 rounded-md bg-purple-500/15 text-purple-500 font-medium flex-shrink-0">
              APP
            </span>
          )}
        </div>
        <div className="flex items-center gap-1.5 text-xs text-[var(--text-tertiary)] truncate mt-0.5">
          <span className="truncate">{result.path}</span>
          {result.size && result.size > 0 && (
            <span className="flex-shrink-0">· {formatSize(result.size)}</span>
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
  const scrollRef = useRef<HTMLDivElement>(null);
  const [scrollTop, setScrollTop] = useState(0);
  const itemRefs = useRef<(HTMLDivElement | null)[]>([]);
  const [hasUserInteracted, setHasUserInteracted] = useState(false);

  const handleScroll = useCallback(() => {
    if (scrollRef.current) {
      setScrollTop(scrollRef.current.scrollTop);
    }
  }, []);

  const emptyResults = useMemo(() => {
    if (!query || query.trim() === '') {
      return (
        <div className="flex items-center justify-center h-full w-full text-center px-8" style={{ minHeight: '100%' }}>
          <div>
            <div className="text-7xl mb-6 opacity-30">⌘</div>
            <div className="text-2xl font-semibold text-white/90 mb-3">
              Start typing to search
            </div>
            <div className="max-w-xs text-sm text-white/50 leading-relaxed">
              Your results will appear here as you type.
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
    if (!hasUserInteracted) return;
    if (lastNavMethodRef.current !== 'keyboard') return;
    if (selectedIndex < 0 || selectedIndex >= results.length) return;

    if (results.length > VIRTUALIZATION_THRESHOLD) {
      const container = scrollRef.current;
      if (!container) return;

      const containerHeight = container.clientHeight;
      const currentScrollTop = container.scrollTop;

      const itemTop = selectedIndex * ITEM_HEIGHT;
      const itemBottom = itemTop + ITEM_HEIGHT;

      if (itemTop < currentScrollTop) {
        container.scrollTo({ top: itemTop, behavior: 'auto' });
      } else if (itemBottom > currentScrollTop + containerHeight) {
        container.scrollTo({ top: itemBottom - containerHeight, behavior: 'auto' });
      }
    } else {
      const el = itemRefs.current[selectedIndex];
      if (el) {
        el.scrollIntoView({ behavior: 'auto', block: 'nearest' });
      }
    }
  }, [selectedIndex, results.length, lastNavMethodRef, hasUserInteracted]);

  if (emptyResults) return emptyResults;

  const virtualized = results.length > VIRTUALIZATION_THRESHOLD;
  const containerHeight = scrollRef.current?.clientHeight || 600;
  const totalHeight = results.length * ITEM_HEIGHT;
  const startIdx = virtualized
    ? Math.max(0, Math.floor(scrollTop / ITEM_HEIGHT) - OVERSCAN)
    : 0;
  const endIdx = virtualized
    ? Math.min(results.length, Math.ceil((scrollTop + containerHeight) / ITEM_HEIGHT) + OVERSCAN)
    : results.length;
  const visibleItems = results.slice(startIdx, endIdx);
  const offsetY = startIdx * ITEM_HEIGHT;

  return (
    <div className="flex-1 flex flex-col min-h-0">
      <div className="flex-1 overflow-y-auto custom-scrollbar -mx-1 px-1" ref={scrollRef} onScroll={handleScroll} onMouseEnter={() => { lastNavMethodRef.current = 'mouse'; setHasUserInteracted(true); }} onClick={() => setHasUserInteracted(true)} onKeyDown={() => setHasUserInteracted(true)} tabIndex={-1}>
        <div style={virtualized ? { height: totalHeight, position: 'relative' } : undefined}>
          <div style={virtualized ? { transform: `translateY(${offsetY}px)` } : undefined}
               className="space-y-0.5">
            {visibleItems.map((result, i) => {
              const actualIndex = startIdx + i;
              return (
                <ResultItem
                  key={result.id}
                  ref={(el) => (itemRefs.current[actualIndex] = el)}
                  result={result}
                  index={actualIndex}
                  isSelected={selectedIndex === actualIndex}
                  query={query}
                  onClick={onClick}
                  onOpen={onOpen}
                />
              );
            })}
          </div>
        </div>
      </div>
    </div>
  );
};

export default React.memo(ResultsList);
