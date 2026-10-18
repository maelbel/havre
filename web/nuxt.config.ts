// Havre web UI.
//
// SPA mode (`ssr: false`): `nuxt generate` outputs plain static files in
// `.output/public`, which the Rust server serves. No Node.js in production,
// and the same build will work inside Tauri later.
export default defineNuxtConfig({
  compatibilityDate: '2026-10-01',
  ssr: false,
  devtools: { enabled: true },

  css: [
    '@fontsource-variable/geist',
    '@fontsource-variable/geist-mono',
    '@fontsource-variable/newsreader',
    '~/assets/css/main.css',
  ],

  app: {
    head: {
      title: 'Havre',
      htmlAttrs: { lang: 'en' },
      meta: [
        { name: 'viewport', content: 'width=device-width, initial-scale=1' },
        { name: 'color-scheme', content: 'dark light' },
      ],
      link: [{ rel: 'icon', type: 'image/svg+xml', href: '/favicon.svg' }],
    },
  },

  // In development, `pnpm dev` runs on :3000 and forwards /api to the Rust
  // server on :8080 (and /mcp, for AI apps), so both behave like one site.
  nitro: {
    devProxy: {
      '/api': { target: 'http://127.0.0.1:8080/api', changeOrigin: true },
      '/mcp': { target: 'http://127.0.0.1:8080/mcp', changeOrigin: true },
    },
  },
})
