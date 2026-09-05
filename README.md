# yankee

Yank shit from stuff.

Give it a link to anything on YouTube, Spotify, or Instagram and it drags the goods home by the scruff. One binary, zero babysitting.

## Usage

```
yankee info <url> [--audio] [--proxy <p>]
yankee get  <url> [--audio] [--proxy <p>] [--dir <d>]
```

- `info` — peek at the loot before you commit: title, source, artist, stream URLs.
- `get` — commit. Best stream it can find, dragged into your folder.
- `--audio` — skip the looking parts, just the sound.
- `--proxy` — route everything through a proxy (e.g. `http://127.0.0.1:2080`) when you'd rather not wave from your own lawn.

Spoils land in `downloads/` by default. Change it per-run with `--dir`, or once for good in `config.toml`:

```toml
dir = "downloads"
```

## Sources

- **YouTube** — any watch URL, video or audio.
- **Spotify** — track in, actual audio out.
- **Instagram** — post media, grabbed courteously enough.

## Build

```
cargo build --release
```

Needs a system OpenSSL (`native-tls`).

## Why "yankee"?

Because it yanks. And it's vaguely American about it.