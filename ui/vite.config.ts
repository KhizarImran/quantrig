import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// `npm run dev` serves the UI; the API stays on the Rust side.
export default defineConfig({
  plugins: [react()],
  server: { proxy: { "/run": "http://127.0.0.1:9000", "/report": "http://127.0.0.1:9000" } },
});
