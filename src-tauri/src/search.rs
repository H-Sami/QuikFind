use crate::error::{QuikFindError, Result};
use std::num::NonZeroUsize;
use crate::models::{
    compute_file_id, schema as tantivy_schema, FileEntry, ResultKind, SearchResult,
    SearchResults,
};
use lru::LruCache;
use nucleo::{Matcher, Config};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;
use tantivy::collector::TopDocs;
use tantivy::directory::MmapDirectory;
use tantivy::query::{
    BooleanQuery, FuzzyTermQuery, QueryParser,
};
use tantivy::schema::{Field, Schema, Value};
use tantivy::{doc, Index, IndexReader, IndexSettings, IndexWriter, ReloadPolicy, TantivyDocument};
use tokio::sync::RwLock;
use tracing::info;

pub struct SearchEngine {
    pub index: Arc<RwLock<Option<Index>>>,
    pub reader: Arc<RwLock<Option<IndexReader>>>,
    pub writer: Arc<std::sync::Mutex<Option<IndexWriter>>>,
    schema: Schema,
    fields: SearchFields,
    query_cache: Arc<std::sync::Mutex<LruCache<String, SearchResults>>>,
    popular_cache: Arc<std::sync::Mutex<LruCache<String, SearchResults>>>,
    matcher: Arc<parking_lot::Mutex<Matcher>>,
    is_indexing: Arc<AtomicBool>,
}

struct SearchFields {
    path: Field,
    name: Field,
    content: Field,
    size: Field,
    modified: Field,
    kind: Field,
    id: Field,
}

impl Default for SearchEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl SearchEngine {
    /// Creates a new search engine.
    ///
    /// # Panics
    /// Panics if the Tantivy schema fields are not properly configured.
    #[must_use]
    pub fn new() -> Self {
        let schema = tantivy_schema().clone();
        // SAFETY: all field names are defined in build_tantivy_schema and are guaranteed to exist
        #[allow(clippy::unwrap_used)]
        let fields = SearchFields {
            path: schema.get_field("path").unwrap(),
            name: schema.get_field("name").unwrap(),
            content: schema.get_field("content").unwrap(),
            size: schema.get_field("size").unwrap(),
            modified: schema.get_field("modified").unwrap(),
            kind: schema.get_field("kind").unwrap(),
            id: schema.get_field("id").unwrap(),
        };

        Self {
            index: Arc::new(RwLock::new(None)),
            reader: Arc::new(RwLock::new(None)),
            writer: Arc::new(std::sync::Mutex::new(None)),
            schema,
            fields,
            query_cache: Arc::new(std::sync::Mutex::new(LruCache::new(
                // SAFETY: 200 is a non-zero compile-time constant
                #[allow(clippy::unwrap_used)]
                NonZeroUsize::new(200).unwrap(),
            ))),
            popular_cache: Arc::new(std::sync::Mutex::new(LruCache::new(
                // SAFETY: 50 is a non-zero compile-time constant
                #[allow(clippy::unwrap_used)]
                NonZeroUsize::new(50).unwrap(),
            ))),
            matcher: Arc::new(parking_lot::Mutex::new(Matcher::new(Config::DEFAULT))),
            is_indexing: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Initializes the Tantivy index at the given directory.
    ///
    /// # Errors
    /// Returns an error if the index cannot be created or opened.
    pub fn initialize_index(&self, index_dir: &Path) -> Result<()> {
        std::fs::create_dir_all(index_dir).map_err(QuikFindError::Io)?;

        let mmap_dir = MmapDirectory::open(index_dir).map_err(|e| QuikFindError::Generic(format!("MmapDir: {e}")))?;

        let index = if Index::exists(&mmap_dir).map_err(|e| QuikFindError::Generic(format!("Index exists: {e}")))? {
            info!("Opening existing Tantivy index at {:?}", index_dir);
            Index::open(mmap_dir).map_err(QuikFindError::Tantivy)?
        } else {
            info!("Creating new Tantivy index at {:?}", index_dir);
            Index::create(mmap_dir, self.schema.clone(), IndexSettings::default()).map_err(QuikFindError::Tantivy)?
        };

        let writer = index
            .writer(256_000_000)
            .map_err(QuikFindError::Tantivy)?;

        let reader = index
            .reader_builder()
            .reload_policy(ReloadPolicy::Manual)
            .try_into()
            .map_err(QuikFindError::Tantivy)?;

        // Use blocking_write since Tauri setup doesn't have a tokio runtime context
        *self.index.blocking_write() = Some(index);
        *self.reader.blocking_write() = Some(reader);
        *self.writer.lock().map_err(|e| QuikFindError::Generic(format!("Writer lock: {e}")))? = Some(writer);

        info!("Tantivy index initialized successfully");
        Ok(())
    }

    /// Indexes a file entry document.
    ///
    /// # Errors
    /// Returns an error if the index writer is not available or indexing fails.
    pub fn index_document(&self, entry: &FileEntry) -> Result<()> {
        let mut writer_lock = self.writer.lock().map_err(|e| {
            QuikFindError::Generic(format!("Writer lock: {e}"))
        })?;
        let writer = writer_lock
            .as_mut()
            .ok_or(QuikFindError::WriterNotReady)?;

        let file_id = compute_file_id(&entry.path, entry.size, entry.modified);

        let document = doc!(
            self.fields.path => entry.path.as_str(),
            self.fields.name => entry.name.as_str(),
            self.fields.content => entry.content.as_deref().unwrap_or(""),
            self.fields.size => entry.size,
            self.fields.modified => entry.modified,
            self.fields.kind => entry.kind.as_str(),
            self.fields.id => file_id.as_str(),
        );

        writer
            .add_document(document)
            .map_err(QuikFindError::Tantivy)?;

        Ok(())
    }

    /// Deletes a document from the index by its ID.
    ///
    /// # Errors
    /// Returns an error if the index writer is not available.
    pub fn delete_document(&self, doc_id: &str) -> Result<()> {
        let mut writer_lock = self.writer.lock().map_err(|e| {
            QuikFindError::Generic(format!("Writer lock: {e}"))
        })?;
        let writer = writer_lock
            .as_mut()
            .ok_or(QuikFindError::WriterNotReady)?;

        let term = tantivy::Term::from_field_text(self.fields.id, doc_id);
        writer.delete_term(term);
        Ok(())
    }

    /// Replaces an existing document by deleting the old one (matched by its `id` field)
    /// and adding a new one. Used during content enrichment (Phase 2) where only the
    /// `content` field changes.
    ///
    /// # Errors
    /// Returns an error if the index writer is not available or Tantivy operations fail.
    pub fn update_document(&self, entry: &FileEntry) -> Result<()> {
        let file_id = compute_file_id(&entry.path, entry.size, entry.modified);
        let mut writer_lock = self
            .writer
            .lock()
            .map_err(|e| QuikFindError::Generic(format!("Writer lock: {e}")))?;
        let writer = writer_lock
            .as_mut()
            .ok_or(QuikFindError::WriterNotReady)?;

        let term = tantivy::Term::from_field_text(self.fields.id, &file_id);
        writer.delete_term(term);

        let document = doc!(
            self.fields.path => entry.path.as_str(),
            self.fields.name => entry.name.as_str(),
            self.fields.content => entry.content.as_deref().unwrap_or(""),
            self.fields.size => entry.size,
            self.fields.modified => entry.modified,
            self.fields.kind => entry.kind.as_str(),
            self.fields.id => file_id.as_str(),
        );
        writer.add_document(document).map_err(QuikFindError::Tantivy)?;
        Ok(())
    }

    /// Commits pending changes to the index.
    ///
    /// # Errors
    /// Returns an error if the index writer is not available or commit fails.
    pub fn commit(&self) -> Result<()> {
        let mut writer_lock = self.writer.lock().map_err(|e| {
            QuikFindError::Generic(format!("Writer lock: {e}"))
        })?;
        let writer = writer_lock
            .as_mut()
            .ok_or(QuikFindError::WriterNotReady)?;

        writer.commit().map_err(QuikFindError::Tantivy)?;
        Ok(())
    }

    /// Forces an immediate reader reload so committed data is visible to searches.
    /// Used by finish_batch_index and the file watcher.
    ///
    /// # Errors
    /// Returns an error if the index reader cannot be reloaded.
    pub async fn reload_searcher(&self) -> Result<()> {
        let reader_lock = self.reader.read().await;
        if let Some(reader) = reader_lock.as_ref() {
            reader.reload().map_err(QuikFindError::Tantivy)?;
        }
        Ok(())
    }

    #[must_use]
    pub fn is_indexing(&self) -> bool {
        self.is_indexing.load(Ordering::Relaxed)
    }

    fn fuzzy_score_name(&self, query: &str, name: &str) -> f32 {
        let mut matcher = self.matcher.lock();
        let result = matcher.fuzzy_match(
            nucleo::Utf32Str::Ascii(query.as_bytes()),
            nucleo::Utf32Str::Ascii(name.as_bytes()),
        );
        match result {
            Some(score) if score > 0 => f32::from(score) / 100.0,
            _ => 0.0,
        }
    }

    fn fuzzy_score_name_parallel(
        matcher: &parking_lot::Mutex<Matcher>,
        query: &str,
        name: &str,
    ) -> f32 {
        let mut m = matcher.lock();
        let result = m.fuzzy_match(
            nucleo::Utf32Str::Ascii(query.as_bytes()),
            nucleo::Utf32Str::Ascii(name.as_bytes()),
        );
        match result {
            Some(score) if score > 0 => f32::from(score) / 100.0,
            _ => 0.0,
        }
    }

    fn doc_to_search_result(
        &self,
        tantivy_score: f32,
        doc: &TantivyDocument,
        now_ts: i64,
        orig_query: &str,
    ) -> SearchResult {
        let path = doc
            .get_first(self.fields.path)
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let name = doc
            .get_first(self.fields.name)
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let kind_str = doc
            .get_first(self.fields.kind)
            .and_then(|v| v.as_str())
            .unwrap_or("File");
        let id = doc
            .get_first(self.fields.id)
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let size: Option<u64> = doc
            .get_first(self.fields.size)
            .and_then(|v| v.as_u64());
        let modified: Option<i64> = doc
            .get_first(self.fields.modified)
            .and_then(|v| v.as_i64());

        let name_score = Self::fuzzy_score_name_parallel(&self.matcher, orig_query, name);

        let recency_boost = modified
            .map_or(0.0, |m| {
                // REASON: recency boost for scoring; float cast precision is acceptable
                #[allow(clippy::cast_precision_loss)]
                let age_hours = (now_ts - m) as f64 / 3600.0;
                if age_hours < 24.0 {
                    1.0
                } else if age_hours < 168.0 {
                    0.5
                } else {
                    0.0
                }
            });

        let final_score = name_score.mul_add(3.0, tantivy_score.max(0.0)) + (recency_boost * 0.5);

        SearchResult {
            id: id.to_string(),
            path: path.to_string(),
            name: name.to_string(),
            kind: kind_str.parse::<ResultKind>().unwrap_or(ResultKind::File),
            score: final_score,
            size,
            modified,
            icon: None,
        }
    }

    /// Performs a search across the index.
    ///
    /// # Errors
    /// Returns an error if the reader is not initialized.
    ///
    /// # Panics
    /// Panics if the query cache mutex is poisoned.
    pub async fn perform_search(
        &self,
        query: &str,
        limit: u32,
        offset: u32,
        _fuzzy_threshold: f32,
        max_results: u32,
        enable_content_search: bool,
    ) -> Result<SearchResults> {
        let start = Instant::now();

        if query.trim().is_empty() {
            return Ok(SearchResults { results: Vec::new(), total: 0, query_time_ms: 0 });
        }

        let cache_key = format!("{query}:{limit}:{offset}");

        // Do not serve cached results during indexing. Mid-index searches return partial or
        // empty data; caching them would poison the cache and hide real results after indexing.
        let is_currently_indexing = self.is_indexing.load(Ordering::Relaxed);
        if !is_currently_indexing {
            // SAFETY: cache mutex is never held across a panic boundary
            #[allow(clippy::unwrap_used)]
            if let Some(cached) = self.query_cache.lock().unwrap().get(&cache_key) {
                return Ok(cached.clone());
            }
        }

        let query_lower = query.to_lowercase();

        // Get a fresh Searcher snapshot from the reader on every request.
        // reload_searcher() is called explicitly after every commit, so the reader
        // always reflects the latest committed state.
        let searcher = {
            let reader_lock = self.reader.read().await;
            reader_lock
                .as_ref()
                .ok_or(QuikFindError::SearcherNotReady)?
                .searcher()
        };

        let query_fields = if enable_content_search {
            vec![self.fields.name, self.fields.content, self.fields.path]
        } else {
            vec![self.fields.name, self.fields.path]
        };
        let query_parser = QueryParser::for_index(searcher.index(), query_fields);

        let fuzzy_query = FuzzyTermQuery::new(
            tantivy::Term::from_field_text(self.fields.name, query_lower.as_str()),
            1,
            true,
        );

        let tantivy_cap = (max_results as usize * 2).max(100);
        let fuzzy_cap = (max_results as usize).max(50);
        let mut all_docs: Vec<(f32, TantivyDocument)> = Vec::with_capacity(tantivy_cap + fuzzy_cap);

        let parsed_query = query_parser
            .parse_query(query)
            .unwrap_or_else(|_| Box::new(BooleanQuery::new(vec![])));

        if let Ok(top_docs) = searcher.search(&parsed_query, &TopDocs::with_limit(tantivy_cap)) {
            for (score, doc_addr) in top_docs {
                if let Ok(doc) = searcher.doc::<TantivyDocument>(doc_addr) {
                    all_docs.push((score, doc));
                }
            }
        }

        if let Ok(fuzzy_docs) = searcher.search(&fuzzy_query, &TopDocs::with_limit(fuzzy_cap)) {
            for (score, doc_addr) in fuzzy_docs {
                if let Ok(doc) = searcher.doc::<TantivyDocument>(doc_addr) {
                    all_docs.push((score * 0.5, doc));
                }
            }
        }

        let now_ts = chrono::Utc::now().timestamp();

        let mut scored_results: Vec<SearchResult> = all_docs
            .into_iter()
            .map(|(tantivy_score, doc)| {
                self.doc_to_search_result(tantivy_score, &doc, now_ts, query)
            })
            .collect();

        scored_results.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
        scored_results.dedup_by(|a, b| a.path == b.path);

        let total = scored_results.len() as u64;
        let effective_limit = limit.min(max_results);
        let results: Vec<SearchResult> = scored_results
            .into_iter()
            .skip(offset as usize)
            .take(effective_limit as usize)
            .collect();

        // REASON: query times < 1ms are still valid as 0; u128->u64 truncation is safe for this range
        #[allow(clippy::cast_possible_truncation)]
        let query_time_ms = start.elapsed().as_millis() as u64;

        let search_results = SearchResults { results, total, query_time_ms };

        // Only cache when the index is stable — never during indexing
        if !is_currently_indexing {
            // SAFETY: cache mutexes are never held across a panic boundary
            #[allow(clippy::unwrap_used)]
            {
                self.query_cache.lock().unwrap().put(cache_key, search_results.clone());
            }
            if query.len() <= 3 {
                #[allow(clippy::unwrap_used)]
                self.popular_cache.lock().unwrap().put(
                    format!("pop:{query}:{limit}:{offset}"),
                    search_results.clone(),
                );
            }
        }

        Ok(search_results)
    }

    #[must_use]
    pub fn search_apps(
        &self,
        query: &str,
        apps: &[(String, String, String)],
    ) -> Vec<super::models::AppResult> {
        if query.trim().is_empty() {
            return apps
                .iter()
                .take(20)
                .map(|(id, name, path)| super::models::AppResult {
                    id: id.clone(),
                    name: name.clone(),
                    path: path.clone(),
                    icon: None,
                    score: 0.0,
                })
                .collect();
        }

        let query_lower = query.to_lowercase();
        let mut scored: Vec<(f32, &(String, String, String))> = apps
            .iter()
            .map(|app| {
                let name_score = self.fuzzy_score_name(&query_lower, &app.1);
                let path_score = if app.2.to_lowercase().contains(&query_lower) {
                    0.5
                } else {
                    0.0
                };
                let exact_prefix = if app.1.to_lowercase().starts_with(&query_lower) {
                    1.0
                } else {
                    0.0
                };
                let score = name_score.mul_add(3.0, exact_prefix * 2.0) + path_score;
                (score, app)
            })
            .collect();

        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        scored.retain(|(s, _)| *s > 0.0);

        scored
            .into_iter()
            .take(20)
            .map(|(score, (id, name, path))| super::models::AppResult {
                id: id.clone(),
                name: name.clone(),
                path: path.clone(),
                icon: None,
                score,
            })
            .collect()
    }

    /// Starts a batch indexing session.
    pub fn start_batch_index(&self) {
        self.is_indexing.store(true, Ordering::Relaxed);
    }

    /// Finishes a batch indexing session and reloads the searcher.
    ///
    /// # Errors
    /// Returns an error if committing or reloading fails.
    pub async fn finish_batch_index(&self) -> Result<()> {
        self.commit()?;
        self.is_indexing.store(false, Ordering::Relaxed);
        self.reload_searcher().await?;
        // Invalidate query caches so post-index searches see real results
        // SAFETY: cache mutexes are never held across a panic boundary
        #[allow(clippy::unwrap_used)]
        {
            self.query_cache.lock().unwrap().clear();
            self.popular_cache.lock().unwrap().clear();
        }
        info!("Batch indexing complete, searcher reloaded, caches cleared");
        Ok(())
    }

    /// Clears the entire search index.
    ///
    /// # Errors
    /// Returns an error if the index writer is not available or clearing fails.
    pub async fn clear_index(&self) -> Result<()> {
        {
            let mut writer_lock = self.writer.lock().map_err(|e| {
                QuikFindError::Generic(format!("Writer lock: {e}"))
            })?;
            let writer = writer_lock
                .as_mut()
                .ok_or(QuikFindError::WriterNotReady)?;

            writer.delete_all_documents().map_err(QuikFindError::Tantivy)?;
            writer.commit().map_err(QuikFindError::Tantivy)?;
        }
        self.reload_searcher().await?;
        info!("Index cleared");
        Ok(())
    }

    /// Completely wipes the index by deleting the index directory and recreating it.
    /// Use this over clear_index when you need to guarantee no old segments remain on disk.
    ///
    /// # Errors
    /// Returns an error if the directory cannot be deleted/recreated.
    pub async fn clear_index_completely(&self, index_dir: &Path) -> Result<()> {
        {
            let mut writer_lock = self
                .writer
                .lock()
                .map_err(|e| QuikFindError::Generic(format!("Writer lock: {e}")))?;
            *writer_lock = None;
        }

        *self.index.write().await = None;
        // No self.searcher to reset — reader.searcher() is always called fresh

        if index_dir.exists() {
            std::fs::remove_dir_all(index_dir).map_err(QuikFindError::Io)?;
        }
        std::fs::create_dir_all(index_dir).map_err(QuikFindError::Io)?;

        let mmap_dir = MmapDirectory::open(index_dir)
            .map_err(|e| QuikFindError::Generic(format!("MmapDir: {e}")))?;

        let index = Index::create(mmap_dir, self.schema.clone(), IndexSettings::default())
            .map_err(QuikFindError::Tantivy)?;

        let writer = index.writer(256_000_000).map_err(QuikFindError::Tantivy)?;

        let reader = index
            .reader_builder()
            .reload_policy(ReloadPolicy::Manual)
            .try_into()
            .map_err(QuikFindError::Tantivy)?;

        *self.index.write().await = Some(index);
        *self.reader.write().await = Some(reader);
        *self.writer
            .lock()
            .map_err(|e| QuikFindError::Generic(format!("Writer lock: {e}")))? = Some(writer);

        self.query_cache
            .lock()
            .map_err(|e| QuikFindError::Generic(format!("Cache lock: {e}")))?
            .clear();
        self.popular_cache
            .lock()
            .map_err(|e| QuikFindError::Generic(format!("Cache lock: {e}")))?
            .clear();

        info!("Index completely wiped and recreated at {:?}", index_dir);
        Ok(())
    }
}
