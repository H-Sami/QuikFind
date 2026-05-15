use crate::error::{QuikFindError, Result};
use crate::models::{
    compute_file_id, schema as tantivy_schema, FileEntry, ResultKind, SearchResult, SearchResults,
};
use lru::LruCache;
use nucleo::{Config, Matcher};
use std::num::NonZeroUsize;
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;
use tantivy::collector::TopDocs;
use tantivy::directory::MmapDirectory;
use tantivy::query::{BooleanQuery, FuzzyTermQuery, QueryParser};
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
    matcher: Arc<parking_lot::Mutex<Matcher>>,
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
                #[allow(clippy::unwrap_used)]
                NonZeroUsize::new(200).unwrap(),
            ))),
            matcher: Arc::new(parking_lot::Mutex::new(Matcher::new(Config::DEFAULT))),
        }
    }

    /// Initializes the Tantivy index at the given directory.
    ///
    /// # Errors
    /// Returns an error if the index cannot be created or opened.
    pub fn initialize_index(&self, index_dir: &Path) -> Result<()> {
        std::fs::create_dir_all(index_dir).map_err(QuikFindError::Io)?;

        let mmap_dir = MmapDirectory::open(index_dir)
            .map_err(|e| QuikFindError::Generic(format!("MmapDir: {e}")))?;

        let index = if Index::exists(&mmap_dir)
            .map_err(|e| QuikFindError::Generic(format!("Index exists: {e}")))?
        {
            info!("Opening existing Tantivy index at {:?}", index_dir);
            Index::open(mmap_dir).map_err(QuikFindError::Tantivy)?
        } else {
            info!("Creating new Tantivy index at {:?}", index_dir);
            Index::create(mmap_dir, self.schema.clone(), IndexSettings::default())
                .map_err(QuikFindError::Tantivy)?
        };

        let writer = index.writer(256_000_000).map_err(QuikFindError::Tantivy)?;
        let reader = index
            .reader_builder()
            .reload_policy(ReloadPolicy::Manual)
            .try_into()
            .map_err(QuikFindError::Tantivy)?;

        *self.index.blocking_write() = Some(index);
        *self.reader.blocking_write() = Some(reader);
        *self
            .writer
            .lock()
            .map_err(|e| QuikFindError::Generic(format!("Writer lock: {e}")))? = Some(writer);

        info!("Tantivy index initialized successfully");
        Ok(())
    }

    /// Indexes or replaces a file entry document.
    ///
    /// # Errors
    /// Returns an error if the index writer is not available or indexing fails.
    pub fn index_document(&self, entry: &FileEntry) -> Result<()> {
        let mut writer_lock = self
            .writer
            .lock()
            .map_err(|e| QuikFindError::Generic(format!("Writer lock: {e}")))?;
        let writer = writer_lock.as_mut().ok_or(QuikFindError::WriterNotReady)?;

        let file_id = compute_file_id(&entry.path);
        writer.delete_term(tantivy::Term::from_field_text(self.fields.id, &file_id));
        writer
            .add_document(self.document_for_entry(entry, &file_id))
            .map_err(QuikFindError::Tantivy)?;

        Ok(())
    }

    /// Deletes a document from the index by its stable ID.
    ///
    /// # Errors
    /// Returns an error if the index writer is not available.
    pub fn delete_document(&self, doc_id: &str) -> Result<()> {
        let mut writer_lock = self
            .writer
            .lock()
            .map_err(|e| QuikFindError::Generic(format!("Writer lock: {e}")))?;
        let writer = writer_lock.as_mut().ok_or(QuikFindError::WriterNotReady)?;

        writer.delete_term(tantivy::Term::from_field_text(self.fields.id, doc_id));
        Ok(())
    }

    /// Replaces an existing document by stable path identity.
    ///
    /// # Errors
    /// Returns an error if the index writer is not available or Tantivy operations fail.
    pub fn update_document(&self, entry: &FileEntry) -> Result<()> {
        self.index_document(entry)
    }

    fn document_for_entry(&self, entry: &FileEntry, file_id: &str) -> TantivyDocument {
        doc!(
            self.fields.path => entry.path.as_str(),
            self.fields.name => entry.name.as_str(),
            self.fields.content => entry.content.as_deref().unwrap_or(""),
            self.fields.size => entry.size,
            self.fields.modified => entry.modified,
            self.fields.kind => entry.kind.as_str(),
            self.fields.id => file_id,
        )
    }

    /// Commits pending changes to the index.
    ///
    /// # Errors
    /// Returns an error if the index writer is not available or commit fails.
    pub fn commit(&self) -> Result<()> {
        let mut writer_lock = self
            .writer
            .lock()
            .map_err(|e| QuikFindError::Generic(format!("Writer lock: {e}")))?;
        let writer = writer_lock.as_mut().ok_or(QuikFindError::WriterNotReady)?;

        writer.commit().map_err(QuikFindError::Tantivy)?;
        Ok(())
    }

    /// Forces an immediate reader reload so committed data is visible to searches.
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

    /// Clears all query caches.
    ///
    /// # Errors
    /// Returns an error if a cache mutex is poisoned.
    pub fn invalidate_caches(&self) -> Result<()> {
        self.query_cache
            .lock()
            .map_err(|e| QuikFindError::Generic(format!("Cache lock: {e}")))?
            .clear();
        Ok(())
    }

    /// Commits mutations, reloads the reader, and invalidates cached queries.
    ///
    /// # Errors
    /// Returns an error if commit or reload fails.
    pub async fn commit_reload_invalidate(&self) -> Result<()> {
        self.commit()?;
        self.reload_searcher().await?;
        self.invalidate_caches()?;
        Ok(())
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
        let size = doc.get_first(self.fields.size).and_then(|v| v.as_u64());
        let modified = doc.get_first(self.fields.modified).and_then(|v| v.as_i64());

        let name_score = Self::fuzzy_score_name_parallel(&self.matcher, orig_query, name);
        let recency_boost = modified.map_or(0.0, |m| {
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

    /// Performs a search across the file index.
    ///
    /// # Errors
    /// Returns an error if the reader is not initialized.
    pub async fn perform_search(
        &self,
        query: &str,
        limit: u32,
        offset: u32,
        max_results: u32,
        enable_content_search: bool,
        use_cache: bool,
    ) -> Result<SearchResults> {
        let start = Instant::now();

        if query.trim().is_empty() {
            return Ok(SearchResults {
                results: Vec::new(),
                total: 0,
                query_time_ms: 0,
            });
        }

        let cache_key = search_cache_key(query, limit, offset, max_results, enable_content_search);
        if use_cache {
            #[allow(clippy::unwrap_used)]
            if let Some(cached) = self.query_cache.lock().unwrap().get(&cache_key) {
                return Ok(cached.clone());
            }
        }

        let query_lower = query.to_lowercase();
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

        scored_results.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        deduplicate_by_path(&mut scored_results);

        let total = scored_results.len() as u64;
        let effective_limit = limit.min(max_results);
        let results = scored_results
            .into_iter()
            .skip(offset as usize)
            .take(effective_limit as usize)
            .collect();

        #[allow(clippy::cast_possible_truncation)]
        let query_time_ms = start.elapsed().as_millis() as u64;
        let search_results = SearchResults {
            results,
            total,
            query_time_ms,
        };

        if use_cache {
            #[allow(clippy::unwrap_used)]
            self.query_cache
                .lock()
                .unwrap()
                .put(cache_key, search_results.clone());
        }

        Ok(search_results)
    }

    /// Clears the entire search index.
    ///
    /// # Errors
    /// Returns an error if the index writer is not available or clearing fails.
    pub async fn clear_index(&self) -> Result<()> {
        {
            let mut writer_lock = self
                .writer
                .lock()
                .map_err(|e| QuikFindError::Generic(format!("Writer lock: {e}")))?;
            let writer = writer_lock.as_mut().ok_or(QuikFindError::WriterNotReady)?;

            writer
                .delete_all_documents()
                .map_err(QuikFindError::Tantivy)?;
            writer.commit().map_err(QuikFindError::Tantivy)?;
        }
        self.reload_searcher().await?;
        self.invalidate_caches()?;
        info!("Index cleared");
        Ok(())
    }
}

fn search_cache_key(
    query: &str,
    limit: u32,
    offset: u32,
    max_results: u32,
    enable_content_search: bool,
) -> String {
    format!(
        "{}:{}:{}:{}:{}",
        query.trim().to_lowercase(),
        limit,
        offset,
        max_results,
        enable_content_search
    )
}

fn deduplicate_by_path(results: &mut Vec<SearchResult>) {
    let mut seen = std::collections::HashSet::new();
    results.retain(|result| seen.insert(result.path.replace('\\', "/").to_lowercase()));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_key_includes_result_affecting_inputs() {
        let base = search_cache_key("Readme", 10, 0, 25, false);

        assert_ne!(base, search_cache_key("Readme", 10, 0, 25, true));
        assert_ne!(base, search_cache_key("Readme", 20, 0, 25, false));
        assert_ne!(base, search_cache_key("Readme", 10, 1, 25, false));
        assert_ne!(base, search_cache_key("Readme", 10, 0, 50, false));
        assert_eq!(base, search_cache_key(" readme ", 10, 0, 25, false));
    }

    #[test]
    fn deduplication_keeps_highest_scored_path() {
        let mut results = vec![
            SearchResult {
                id: "1".to_string(),
                path: "C:\\Temp\\Note.txt".to_string(),
                name: "Note.txt".to_string(),
                kind: ResultKind::File,
                score: 10.0,
                size: None,
                modified: None,
                icon: None,
            },
            SearchResult {
                id: "2".to_string(),
                path: "c:/temp/note.txt".to_string(),
                name: "Note.txt".to_string(),
                kind: ResultKind::File,
                score: 3.0,
                size: None,
                modified: None,
                icon: None,
            },
        ];

        deduplicate_by_path(&mut results);

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, "1");
    }

    #[test]
    fn invalidate_caches_clears_query_cache() {
        let engine = SearchEngine::new();
        engine.query_cache.lock().unwrap().put(
            search_cache_key("a", 1, 0, 1, false),
            SearchResults {
                results: Vec::new(),
                total: 0,
                query_time_ms: 0,
            },
        );

        engine.invalidate_caches().unwrap();

        assert_eq!(engine.query_cache.lock().unwrap().len(), 0);
    }
}
