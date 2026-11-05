// `tauri build --debug` sets TAURI_ENV_DEBUG. Build the frontend in Vite's
// development mode for it so `import.meta.env.DEV` and React's dev build match
// the Rust debug profile; the env values still come from the same `.env`.
import { build } from 'vite';

const debug = process.env.TAURI_ENV_DEBUG === 'true';
// Vite picks the React dev/prod bundle from NODE_ENV, not from `mode`.
if (debug) process.env.NODE_ENV = 'development';

await build({
  mode: debug ? 'development' : 'production',
  build: { minify: !debug },
});
