# QuikFind

**Fast, private, local desktop search & launcher for Windows.**

QuikFind is a blazingly fast, privacy-first desktop search tool built with Rust (Tauri) and React. It indexes your files locally and lets you search and launch applications instantly — all without sending your data anywhere.

## Features

- **Instant search** — powered by Tantivy, a full-text search engine written in Rust
- **App launcher** — quickly launch applications from your start menu
- **Fuzzy matching** — find files even when you don't remember the exact name
- **Keyboard-first** — designed to be used entirely with the keyboard
- **Private by design** — everything runs locally, zero telemetry
- **Customizable** — configure shortcuts, themes, and excluded folders

## Tech Stack

| Layer     | Technology                        |
| --------- | --------------------------------- |
| Frontend  | React + TypeScript + Tailwind CSS |
| Backend   | Rust (Tauri)                      |
| Search    | Tantivy (full-text search engine) |
| State     | Zustand                           |
| Icons     | Lucide React                      |

## Getting Started

### Prerequisites

- [Node.js](https://nodejs.org/) >= 18
- [Rust](https://rustup.rs/) (stable toolchain)
- [Tauri CLI](https://v2.tauri.app/start/cli/)

### Install & Run

```bash
# Install frontend dependencies
npm install

# Run in development mode
npm run tauri dev
```

### Build for Production

```bash
npm run tauri build
```

The installer will be available in `src-tauri/target/release/bundle/`.

## Project Structure

```
QuikFind/
├── src/                  # React frontend
│   ├── components/       # UI components
│   ├── hooks/            # React hooks
│   ├── stores/           # Zustand stores
│   ├── utils/            # Utility functions
│   └── main.tsx          # App entry point
├── src-tauri/            # Rust backend
│   ├── src/              # Rust source code
│   ├── benches/          # Benchmarks
│   ├── Cargo.toml        # Rust dependencies
│   └── tauri.conf.json   # Tauri configuration
├── public/               # Static assets
├── index.html            # HTML entry point
├── package.json          # Node dependencies
└── vite.config.ts        # Vite configuration
```

## License

MIT © 2026-present
