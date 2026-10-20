import { defineConfig } from "vite";
import solid from "vite-plugin-solid";
import tailwindcss from "@tailwindcss/vite";

// The phone companion is a second page built to its own directory: the Rust gateway embeds
// `dist-companion/` at compile time, so the desktop bundle is not duplicated into it.
export default defineConfig({
  root: "src/companion",
  base: "./",
  publicDir: false,
  plugins: [solid({ dev: false }), tailwindcss()],
  build: {
    outDir: "../../dist-companion",
    emptyOutDir: true,
    target: "es2022",
  },
});
