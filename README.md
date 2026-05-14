# QuikFind

Fast, private, local desktop search & launcher for Windows.

Built with **Rust (Tauri 2)** + **React 18** + **TypeScript** + **Tailwind CSS**.

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
