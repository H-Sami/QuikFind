import { useState, useCallback, useEffect, useRef } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { SearchResult, SearchResults } from '../types';
import { useStore } from '../store';
import { SEARCH_DEBOUNCE_SHORT, SEARCH_DEBOUNCE_LONG } from './constants';

export function useSearch() {
  const [query, setQuery] = useState('');
  const [results, setResults] = useState<SearchResult[]>([]);
  const [selectedIndex, setSelectedIndex] = useState(0);
  const [isLoading, setIsLoading] = useState(false);
  const searchIdRef = useRef(0);
  const resultsRef = useRef(results);
  const queryRef = useRef(query);
  const selectedIndexRef = useRef(selectedIndex);
  const { settings } = useStore();

  resultsRef.current = results;
  queryRef.current = query;
  selectedIndexRef.current = selectedIndex;

  const performSearch = useCallback(async (searchQuery: string) => {
    if (!searchQuery.trim()) {
      searchIdRef.current++;
      setResults([]);
      setSelectedIndex(0);
      return;
    }
    const currentId = ++searchIdRef.current;
    setIsLoading(true);
    try {
      const searchResults: SearchResults = await invoke('search', {
        query: searchQuery,
        limit: settings.max_results || 20,
        offset: 0,
      });
      if (currentId !== searchIdRef.current) return;
      setResults(searchResults.results);
      setSelectedIndex(0);
    } catch {
      if (currentId === searchIdRef.current) setResults([]);
    } finally {
      if (currentId === searchIdRef.current) setIsLoading(false);
    }
  }, [settings.max_results]);

  useEffect(() => {
    const delay = query.trim().length <= 3 ? SEARCH_DEBOUNCE_SHORT : SEARCH_DEBOUNCE_LONG;
    const timer = setTimeout(() => performSearch(query), delay);
    return () => clearTimeout(timer);
  }, [query, performSearch]);

  useEffect(() => {
    if (results.length === 0) {
      setSelectedIndex(0);
    } else if (selectedIndex >= results.length) {
      setSelectedIndex(results.length - 1);
    }
  }, [results.length, selectedIndex]);

  const handleKeyDown = useCallback((e: React.KeyboardEvent) => {
    switch (e.key) {
      case 'ArrowDown':
        e.preventDefault();
        setSelectedIndex(prev => Math.min(prev + 1, Math.max(0, results.length - 1)));
        break;
      case 'ArrowUp':
        e.preventDefault();
        setSelectedIndex(prev => Math.max(0, prev - 1));
        break;
      case 'Escape':
        setQuery('');
        break;
    }
  }, [results.length]);

  return {
    query, setQuery,
    results, setResults,
    selectedIndex, setSelectedIndex,
    isLoading,
    handleKeyDown,
    resultsRef,
    queryRef,
    selectedIndexRef,
  };
}
