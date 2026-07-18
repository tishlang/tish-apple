# AI messenger (tish-macos example)

Buddy-list window plus per-chat windows (`macos.openWindow`), native **Markdown** bubbles (`markdown_text`), a **Settings** window (`UserDefaults` via `macos.preferencesGet` / `preferencesSet`), **snap** (`window.snapToRegion` / `nsWindow.snapToRegion`), and mock attach / image / notification hooks.

## Run (macOS)

```bash
cd examples/ai-messenger-macos
npm install
npm run dev
```

Requires the `tish` repo sibling at `../../../tish` (see `package.json` build script).

## Streaming (later)

- **OpenAI-compatible** SSE: merge `choices[0].delta.content`, optional `reasoning_content`, `tool_calls`.
- **LM Studio** native stream: `reasoning.delta`, `message.delta`, `tool_call.*` events ([streaming events](https://lmstudio.ai/docs/developer/rest/streaming-events)).

## Image path

The “Open mock image” control toggles an SF Symbol preview. To try a file path, set a message `imagePath` to an absolute path supported by `NSImage` (see tish-macos `image` vnode).
