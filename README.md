# QuikFind

Fast, private, local desktop search & launcher for Windows.

Built with **Rust (Tauri 2)** + **React 18** + **TypeScript** + **Tailwind CSS**.

<img width="1026" height="344" alt="image" src="https://github.com/user-attachments/assets/13cc3e7b-f9b6-4cd6-8afa-19923496dbbc" />


## Features

- Full-text file search (Tantivy) + fuzzy matching (Nucleo)
- App launcher (scans Start Menu, Program Files, WindowsApps)
- "Type to Search" — global keyboard listener opens on keystroke
- Configurable global hotkey (default Ctrl+Space)
- Real-time file watching (notify crate)
- System tray integration (minimize on close)
- Light/dark themes with custom accent colors
- Remappable keyboard shortcuts
- Glob-based exclusion patterns
- Plugin system for custom data sources
- Search history
- LRU query caching
- Parallel indexing (jwalk + rayon)
- Window state persistence
- Autostart on boot

## Quick Start

```bash
npm install
npm run tauri dev
```

## First Run

On first launch, QuikFind indexes all files and folders on the selected drives.
Depending on the total number of files on your PC, this initial indexing may take
**several minutes**. During this time search results will be partial — only
already-indexed files will appear. The indexer runs in the background, and results
improve incrementally as indexing progresses.

## Build

```bash
npm run tauri build
```

Installers (MSI, NSIS) → `src-tauri/target/release/bundle/`.

## Project

```
QuikFind/
├── src/                  # React frontend
├── src-tauri/src/        # Rust backend
│   ├── main.rs
│   ├── lib.rs            # Setup, tray, hotkeys
│   ├── commands.rs       # Tauri command handlers
│   ├── search.rs         # Search engine (Tantivy + Nucleo)
│   ├── indexer.rs        # File traversal & indexing
│   ├── watcher.rs        # File system watcher
│   ├── apps.rs           # App scanner
│   ├── models.rs         # Data models, schema
│   ├── settings.rs       # SQLite database
│   ├── desktop_listener.rs  # Global keyboard listener
│   ├── plugins.rs        # Plugin architecture
│   └── error.rs
├── src-tauri/benches/    # Benchmarks
├── src-tauri/Cargo.toml
├── src-tauri/tauri.conf.json
├── tailwind.config.js
├── vite.config.ts
└── package.json
```

## License

MIT
