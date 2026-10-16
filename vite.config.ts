import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';
import tailwindcss from '@tailwindcss/vite';

/**
 * Vite configuration for the Subsystem Integration Manager desktop client.
 *
 * The frontend is bundled by Vite and embedded into the Tauri binary
 * (`frontendDist: "../dist"`). The dev server below is used only for
 * `npm run tauri dev`; there is no always-running Node/Next server at runtime.
 */
export default defineConfig({
  plugins: [react(), tailwindcss()],
  // Tauri owns the terminal output; do not wipe it.
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    // src-tauri is compiled by cargo, watching it from vite only causes churn.
    watch: { ignored: ['**/src-tauri/**'] },
  },
  build: {
    // Match the WebView2 / WKWebView baseline shipped by Tauri v2 targets.
    target: 'chrome110',
    outDir: 'dist',
    emptyOutDir: true,
    sourcemap: false,
    chunkSizeWarningLimit: 1200,
  },
});
