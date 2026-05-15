use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::sync::OnceLock;
use tantivy::schema::{Schema, INDEXED, STORED, STRING, TEXT};

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct SearchResult {
    pub id: String,
    pub path: String,
    pub name: String,
    pub kind: ResultKind,
    pub score: f32,
    pub size: Option<u64>,
    pub modified: Option<i64>,
    pub icon: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum ResultKind {
    File,
    Folder,
    App,
    Bookmark,
    Note,
    Custom(String),
}

impl ResultKind {
    #[must_use]
    pub fn as_str(&self) -> &str {
        match self {
            Self::File => "File",
            Self::Folder => "Folder",
            Self::App => "App",
            Self::Bookmark => "Bookmark",
            Self::Note => "Note",
            Self::Custom(s) => s.as_str(),
        }
    }
}

impl std::str::FromStr for ResultKind {
    type Err = std::convert::Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s {
            "File" => Self::File,
            "Folder" => Self::Folder,
            "App" => Self::App,
            "Bookmark" => Self::Bookmark,
            "Note" => Self::Note,
            other => Self::Custom(other.to_string()),
        })
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct SearchResults {
    pub results: Vec<SearchResult>,
    pub total: u64,
    pub query_time_ms: u64,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct AppResult {
    pub id: String,
    pub name: String,
    pub path: String,
    pub icon: Option<String>,
    pub score: f32,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct IndexStatus {
    pub is_indexing: bool,
    pub phase: String,
    pub files_indexed: u64,
    pub total_files: u64,
    pub progress_percent: f32,
    pub last_updated: i64,
    pub errors: Vec<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(default)]
pub struct AppSettings {
    pub indexed_paths: Vec<String>,
    pub excluded_patterns: Vec<String>,
    pub max_results: u32,
    pub hotkey: String,
    pub theme: String,
    pub enable_content_search: bool,
    pub enable_type_to_search: bool,
    pub indexing_interval_minutes: u32,
    pub launch_on_startup: bool,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            indexed_paths: Vec::new(),
            excluded_patterns: vec![
                "**/node_modules/**".to_string(),
                "**/.git/**".to_string(),
                "**/target/**".to_string(),
                "**/dist/**".to_string(),
                "**/.DS_Store".to_string(),
                "**/__pycache__/**".to_string(),
                "**.pyc".to_string(),
            ],
            max_results: 25,
            hotkey: "CmdOrCtrl+Space".to_string(),
            theme: "dark".to_string(),
            enable_content_search: true,
            enable_type_to_search: false,
            indexing_interval_minutes: 30,
            launch_on_startup: false,
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct HistoryItem {
    pub id: String,
    pub path: String,
    pub name: String,
    pub kind: String,
    pub opened_at: i64,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct IndexingProgress {
    pub files_indexed: u64,
    pub total_files: u64,
    pub current_file: Option<String>,
    pub errors: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct FileEntry {
    pub path: String,
    pub name: String,
    pub size: u64,
    pub modified: i64,
    pub kind: ResultKind,
    pub content: Option<String>,
}

static SCHEMA: OnceLock<Schema> = OnceLock::new();

#[must_use]
pub fn schema() -> &'static Schema {
    SCHEMA.get_or_init(build_tantivy_schema)
}

fn build_tantivy_schema() -> Schema {
    let mut builder = Schema::builder();
    builder.add_text_field("path", TEXT | STORED);
    builder.add_text_field("name", TEXT | STORED);
    // content is TEXT-only (not STORED): files are searchable by content keywords but
    // no snippet is returned. Add STORED here if snippet display is required in future.
    builder.add_text_field("content", TEXT);
    builder.add_u64_field("size", INDEXED | STORED);
    builder.add_i64_field("modified", INDEXED | STORED);
    builder.add_text_field("kind", STRING | STORED);
    builder.add_text_field("id", STRING | STORED);
    builder.build()
}

pub const TEXT_EXTENSIONS: &[&str] = &[
    "txt",
    "md",
    "rs",
    "py",
    "js",
    "ts",
    "jsx",
    "tsx",
    "json",
    "toml",
    "yaml",
    "yml",
    "html",
    "css",
    "scss",
    "less",
    "c",
    "cpp",
    "h",
    "hpp",
    "java",
    "kt",
    "swift",
    "go",
    "rb",
    "php",
    "sh",
    "bash",
    "zsh",
    "fish",
    "ps1",
    "bat",
    "cmd",
    "xml",
    "cfg",
    "conf",
    "ini",
    "env",
    "gitignore",
    "dockerfile",
    "makefile",
    "sql",
    "r",
    "m",
    "mm",
    "pl",
    "pm",
    "lua",
    "zig",
    "nim",
    "ex",
    "exs",
    "clj",
    "cljs",
    "edn",
    "scala",
    "groovy",
    "gradle",
    "tex",
    "bib",
    "vue",
    "svelte",
    "astro",
    "mjs",
    "cjs",
    "mts",
    "cts",
];

static EXT_SET: OnceLock<HashSet<&'static str>> = OnceLock::new();

/// Returns `true` if the file at `path` has a text extension or is a known
/// extension-less text filename (e.g., `Makefile`, `Dockerfile`).
#[must_use]
pub fn is_text_extension(path: &str) -> bool {
    let ext_set = EXT_SET.get_or_init(|| TEXT_EXTENSIONS.iter().copied().collect());
    let p = std::path::Path::new(path);

    if let Some(ext) = p.extension().and_then(|e| e.to_str()) {
        if ext_set.contains(ext.to_lowercase().as_str()) {
            return true;
        }
    }

    if let Some(name) = p.file_name().and_then(|n| n.to_str()) {
        return ext_set.contains(name.to_lowercase().as_str());
    }

    false
}

#[must_use]
pub fn compute_file_id(path: &str) -> String {
    let input = normalized_identity_path(path);
    blake3::hash(input.as_bytes()).to_hex()[..16].to_string()
}

#[must_use]
pub fn normalized_identity_path(path: &str) -> String {
    path.replace('\\', "/").to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_result_kind_conversion() {
        assert_eq!(ResultKind::File.as_str(), "File");
        assert_eq!(ResultKind::Folder.as_str(), "Folder");
        assert_eq!(ResultKind::App.as_str(), "App");
        assert_eq!("File".parse::<ResultKind>().unwrap().as_str(), "File");
        assert_eq!(
            "CustomType".parse::<ResultKind>().unwrap().as_str(),
            "CustomType"
        );
    }

    #[test]
    fn test_is_text_extension() {
        assert!(is_text_extension("file.rs"));
        assert!(is_text_extension("file.py"));
        assert!(is_text_extension("file.txt"));
        assert!(is_text_extension("file.md"));
        assert!(is_text_extension("file.js"));
        assert!(is_text_extension("/path/to/file.tsx"));
        assert!(!is_text_extension("file.exe"));
        assert!(!is_text_extension("file.dll"));
        assert!(!is_text_extension("file.png"));
        assert!(!is_text_extension("file.pdf"));
    }

    #[test]
    fn test_compute_file_id() {
        let id1 = compute_file_id("/path/to/file.txt");
        let id2 = compute_file_id("/path/to/file.txt");
        let id3 = compute_file_id("/path/to/other.txt");

        assert_eq!(id1, id2, "Same file should produce same ID");
        assert_ne!(id1, id3, "Different files should produce different IDs");
        assert_eq!(id1.len(), 16, "ID should be 16 hex chars");
    }

    #[test]
    fn test_default_settings() {
        let settings = AppSettings::default();
        assert_eq!(settings.max_results, 25);
        assert_eq!(settings.hotkey, "CmdOrCtrl+Space");
        assert_eq!(settings.theme, "dark");
        assert!(settings.enable_content_search);
        assert!(!settings.enable_type_to_search);
        assert!(settings.indexed_paths.is_empty());
        assert!(settings.excluded_patterns.len() > 2);
    }

    #[test]
    fn test_compute_file_id_is_stable_for_same_path() {
        let id1 = compute_file_id("C:\\Users\\Ada\\Notes.txt");
        let id2 = compute_file_id("c:/users/ada/notes.txt");
        let id3 = compute_file_id("C:\\Users\\Ada\\Other.txt");

        assert_eq!(id1, id2, "Path identity should normalize slashes and case");
        assert_ne!(id1, id3, "Different paths should produce different IDs");
    }
}
