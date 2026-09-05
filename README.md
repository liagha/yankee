# yankee

A small command-line tool that resolves media URLs down to stream assets and downloads them to disk. Single static binary, no external runtime.

## Usage

```
yankee info <url> [--audio] [--proxy <p>]
yankee get  <url> [--audio] [--proxy <p>] [--dir <d>]
```

- `info` prints the resolved title, source, artist, and available stream URLs.
- `get` downloads the best matching stream into the output directory.
- `--audio` selects an audio-only stream; otherwise the best video stream is used.
- `--proxy` routes all traffic through the given proxy (e.g. `http://127.0.0.1:2080`).

Output defaults to `downloads/`; override per-run with `--dir` or globally in `config.toml`:

```toml
dir = "downloads"
```

## Sources

- **YouTube** — resolves video and audio streams for any watch URL.
- **Spotify** — resolves track metadata and finds a matching audio stream.
- **Instagram** — resolves post media.

## Build

```
cargo build --release
```

Needs a system OpenSSL (`native-tls`).