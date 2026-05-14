# QuikFind

**Fast, private, local desktop search & launcher for Windows.**

QuikFind is a blazingly fast, privacy-first desktop launcher built with Rust (Tauri) and React. It indexes your files locally using Tantivy (full-text search) with Nucleo fuzzy matching, and lets you search files and launch applications instantly — all without sending your data anywhere.

## Features

- **Full-text file search** — powered by Tantivy; searches file names, paths, and optionally file contents
- **Fuzzy matching** — uses Nucleo for typo-tolerant matching with recency boosting
- **App launcher** — scans Start Menu, Program Files, and WindowsApps for installed applications
- **"Type to Search"** — detects desktop focus via global keyboard listener (rdev) and opens instantly when you start typing
- **Global hotkey** — configurable shortcut (default Ctrl+Space) to show/hide from anywhere
- **Real-time file watching** — uses the `notify` crate to keep the index updated as files change
- **System tray integration** — minimizes to tray on close; tray menu with Show, Options, Quit
- **Onboarding flow** — first-run wizard to select folders to index
- **Customizable themes** — dark and light modes with custom accent colors
- **Configurable keyboard shortcuts** — remap all shortcuts via the settings UI
- **Search history** — tracks recently opened files
- **Plugin system** — extensible plugin registry for adding custom data sources
- **Autostart support** — option to launch on Windows startup (minimized to tray)
- **Window state persistence** — remembers position and size across sessions
- **Glob-based exclusion** — skip node_modules, .git, target, dist, etc.
- **LRU query caching** — caches recent searches for instant repeat results
- **Parallel indexing** — uses jwalk + rayon for fast multi-threaded file traversal
- **Index progress tracking** — real-time UI feedback during indexing
- **Keyboard-first navigation** — full keyboard control with arrow keys and shortcuts

## Tech Stack

| Layer        | Technology                              |
| ------------ | --------------------------------------- |
| Frontend     | React 18 + TypeScript + Tailwind CSS 3  |
| Build tool   | Vite 5                                  |
| Backend      | Rust (Tauri 2)                          |
| Search       | Tantivy 0.22 + Nucleo 0.4              |
| State        | Zustand                                 |
| Icons        | Lucide React                            |
| Database     | SQLite (rusqlite)                       |
| File watching| notify                                  |
| File walking | jwalk + rayon                           |
| Hashing      | blake3                                  |
| Caching      | lru                                     |
| Global keys  | tauri-plugin-global-shortcut + rdev     |
| Autostart    | tauri-plugin-autostart                  |

## Getting Started

### Prerequisites

- [Node.js](https://nodejs.org/) >= 18
- [Rust](https://rustup.rs/) (stable toolchain, edition 2021)
- [Tauri CLI](https://v2.tauri.app/start/cli/)

### Install & Run

```bash
npm install
npm run tauri dev
```

### Build for Production

```bash
npm run tauri build
```

Installers (MSI, NSIS) will be in `src-tauri/target/release/bundle/`.

## Project Structure

```
QuikFind/
├── src/                    # React frontend
│   ├── components/         # UI components
│   ├── hooks/              # React hooks
│   ├── stores/             # Zustand stores
│   ├── utils/              # Utility functions
│   ├── types.ts            # TypeScript interfaces
│   ├── store.ts            # Global app store
│   └── main.tsx            # App entry point
├── src-tauri/              # Rust backend
│   ├── src/
│   │   ├── main.rs         # Entry point
│   │   ├── lib.rs          # App setup, hotkeys, tray, window management
│   │   ├── commands.rs     # Tauri command handlers
│   │   ├── search.rs       # Tantivy + Nucleo search engine
│   │   ├── indexer.rs      # File traversal and indexing
│   │   ├── watcher.rs      # Real-time file system watcher
│   │   ├── apps.rs         # App scanner (Windows/macOS/Linux)
│   │   ├── models.rs       # Data models, schema, text extensions
│   │   ├── settings.rs     # SQLite settings/history/cache database
│   │   ├── desktop_listener.rs  # Global keyboard listener (Type to Search)
│   │   ├── plugins.rs      # Plugin system architecture
│   │   └── error.rs        # Error types
│   ├── benches/            # Criterion benchmarks
│   ├── Cargo.toml          # Rust dependencies
│   └── tauri.conf.json     # Tauri configuration
├── public/                 # Static assets
├── index.html              # HTML entry point
├── tailwind.config.js      # Tailwind configuration
├── vite.config.ts          # Vite configuration
└── package.json            # Node dependencies
```

## License

MIT © 2026-present
