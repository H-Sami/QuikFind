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
use tantivy::{
    doc, Index, IndexSettings, IndexWriter, ReloadPolicy, Searcher, TantivyDocument,
};
use tokio::sync::RwLock;
use tracing::info;

pub struct SearchEngine {
    pub index: Arc<RwLock<Option<Index>>>,
    pub searcher: Arc<RwLock<Option<Searcher>>>,
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
            searcher: Arc::new(RwLock::new(None)),
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
            .reload_policy(ReloadPolicy::OnCommitWithDelay)
            .try_into()
            .map_err(QuikFindError::Tantivy)?;
        let searcher = reader.searcher();

        // Use blocking_write since Tauri setup doesn't have a tokio runtime context
        *self.index.blocking_write() = Some(index);
        *self.searcher.blocking_write() = Some(searcher);
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

    /// Reloads the searcher to reflect recent index changes.
    ///
    /// # Errors
    /// Returns an error if the index reader cannot be created.
    pub async fn reload_searcher(&self) -> Result<()> {
        let index = self.index.read().await.clone();
        if let Some(index) = index {
            let reader = index
                .reader_builder()
                .reload_policy(ReloadPolicy::OnCommitWithDelay)
                .try_into()
                .map_err(QuikFindError::Tantivy)?;
            let searcher = reader.searcher();
            *self.searcher.write().await = Some(searcher);
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

    // REASON: retained for potential direct Tantivy query API in future
    #[allow(dead_code)]
    fn tantivy_search(
        &self,
        query: &str,
        limit: u32,
        offset: u32,
    ) -> Result<Vec<(f32, TantivyDocument)>> {
        let searcher_lock = self.searcher.blocking_read();
        let searcher = searcher_lock
            .as_ref()
            .ok_or(QuikFindError::SearcherNotReady)?;

        let query_parser = QueryParser::for_index(
            searcher.index(),
            vec![self.fields.name, self.fields.content, self.fields.path],
        );

        let tantivy_query = query_parser
            .parse_query(query)
            .map_err(|e| QuikFindError::Generic(format!("Query parse error: {e}")))?;

        let top_docs = searcher
            .search(
                &tantivy_query,
                &TopDocs::with_limit((limit + offset) as usize),
            )
            .map_err(QuikFindError::Tantivy)?;

        let results: Vec<(f32, TantivyDocument)> = top_docs
            .into_iter()
            .filter_map(|(score, doc_addr)| {
                searcher.doc::<TantivyDocument>(doc_addr).ok().map(|doc| (score, doc))
            })
            .collect();

        Ok(results)
    }

    fn doc_to_search_result(
        &self,
        tantivy_score: f32,
        doc: &TantivyDocument,
        now_ts: i64,
        orig_query: &str,
        query_for_snippet: &str,
        query_len: usize,
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

        let snippet = doc
            .get_first(self.fields.content)
            .and_then(|v| v.as_str())
            .and_then(|c| {
                if c.is_empty() {
                    return None;
                }
                Some(
                    c.to_lowercase().find(query_for_snippet).map_or_else(
                        || c[..c.len().min(100)].to_string(),
                        |pos| {
                            let start = pos.saturating_sub(40);
                            let end = (pos + query_len + 40).min(c.len());
                            let mut snippet = String::with_capacity(90);
                            snippet.push_str("...");
                            snippet.push_str(&c[start..end]);
                            snippet.push_str("...");
                            snippet
                        },
                    ),
                )
            });

        SearchResult {
            id: id.to_string(),
            path: path.to_string(),
            name: name.to_string(),
            kind: kind_str.parse::<ResultKind>().unwrap_or(ResultKind::File),
            score: final_score,
            size,
            modified,
            content_snippet: snippet,
            icon: None,
        }
    }

    /// Performs a search across the index.
    ///
    /// # Errors
    /// Returns an error if the searcher is not available.
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

        let cache_key = format!("{query}:{limit}:{offset}");

        // SAFETY: cache mutex is never held across a panic boundary
        #[allow(clippy::unwrap_used)]
        {
            let mut cache = self.query_cache.lock().unwrap();
            if let Some(cached) = cache.get(&cache_key) {
                return Ok(cached.clone());
            }
        }

        if query.trim().is_empty() {
            return Ok(SearchResults {
                results: Vec::new(),
                total: 0,
                query_time_ms: 0,
            });
        }

        let query_lower = query.to_lowercase();

        let searcher_lock = self.searcher.read().await;
        let searcher = searcher_lock
            .as_ref()
            .ok_or(QuikFindError::SearcherNotReady)?;

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
        let query_for_snippet = &query_lower;
        let query_len = query.len();

        let mut scored_results: Vec<SearchResult> = all_docs
            .into_iter()
            .map(|(tantivy_score, doc)| {
                self.doc_to_search_result(tantivy_score, &doc, now_ts, query, query_for_snippet, query_len)
            })
            .collect();

        scored_results.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

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

        let search_results = SearchResults {
            results,
            total,
            query_time_ms,
        };

        // SAFETY: cache mutex is never held across a panic boundary
        #[allow(clippy::unwrap_used)]
        {
            let mut cache = self.query_cache.lock().unwrap();
            cache.put(cache_key, search_results.clone());
        }

        if query.len() <= 3 {
            // SAFETY: cache mutex is never held across a panic boundary
            #[allow(clippy::unwrap_used)]
            self.popular_cache.lock().unwrap().put(
                format!("pop:{query}:{limit}:{offset}"),
                search_results.clone(),
            );
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
    ///
    /// # Errors
    /// Returns an error if the indexing state cannot be updated.
    pub fn start_batch_index(&self) -> Result<()> {
        self.is_indexing.store(true, Ordering::Relaxed);
        Ok(())
    }

    /// Finishes a batch indexing session and reloads the searcher.
    ///
    /// # Errors
    /// Returns an error if committing or reloading fails.
    pub async fn finish_batch_index(&self) -> Result<()> {
        self.commit()?;
        self.is_indexing.store(false, Ordering::Relaxed);
        self.reload_searcher().await?;
        info!("Batch indexing complete, searcher reloaded");
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

    /// Completely wipes the index by deleting the entire index directory
    /// and recreating a fresh empty index. This is the only reliable way
    /// to guarantee zero old results remain.
    ///
    /// # Errors
    /// Returns an error if the index directory cannot be deleted or recreated.
    pub async fn clear_index_completely(&self, index_dir: &Path) -> Result<()> {
        // Close current writer and index
        {
            let mut writer_lock = self.writer.lock().map_err(|e| {
                QuikFindError::Generic(format!("Writer lock: {e}"))
            })?;
            *writer_lock = None;
        }

        // Drop current index and searcher
        *self.index.write().await = None;
        *self.searcher.write().await = None;

        // Delete the entire index directory
        if index_dir.exists() {
            std::fs::remove_dir_all(index_dir).map_err(QuikFindError::Io)?;
        }

        // Recreate directory and fresh index
        std::fs::create_dir_all(index_dir).map_err(QuikFindError::Io)?;

        let mmap_dir = MmapDirectory::open(index_dir)
            .map_err(|e| QuikFindError::Generic(format!("MmapDir: {e}")))?;

        let index = Index::create(mmap_dir, self.schema.clone(), IndexSettings::default())
            .map_err(QuikFindError::Tantivy)?;

        let writer = index
            .writer(256_000_000)
            .map_err(QuikFindError::Tantivy)?;

        let reader = index
            .reader_builder()
            .reload_policy(ReloadPolicy::OnCommitWithDelay)
            .try_into()
            .map_err(QuikFindError::Tantivy)?;

        let searcher = reader.searcher();

        *self.index.write().await = Some(index);
        *self.searcher.write().await = Some(searcher);
        *self.writer.lock().map_err(|e| QuikFindError::Generic(format!("Writer lock: {e}")))? = Some(writer);

        // Clear in-memory caches
        self.query_cache.lock().map_err(|e| QuikFindError::Generic(format!("Cache lock: {e}")))?.clear();
        self.popular_cache.lock().map_err(|e| QuikFindError::Generic(format!("Cache lock: {e}")))?.clear();

        info!("Index completely wiped and recreated at {:?}", index_dir);
        Ok(())
    }
}
