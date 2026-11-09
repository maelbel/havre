// Havre web UI.
//
// Components come from Nuxt UI (built on Reka UI), themed with Havre's tokens
// in `assets/css/main.css`. Nothing is fetched from other servers: icons are
// Havre's own (`icons.ts`) bundled into the build, fonts are local files.
import { havreIconCollection } from './icons'

// SPA mode (`ssr: false`): `nuxt generate` outputs plain static files in
// `.output/public`, which the Rust server serves. No Node.js in production,
// and the same build will work inside Tauri later.
export default defineNuxtConfig({
  compatibilityDate: '2026-10-01',
  ssr: false,
  devtools: { enabled: true },

  modules: ['@nuxt/ui'],

  ui: {
    // Fonts are the @fontsource files below; @nuxt/fonts would download
    // from Google instead.
    fonts: false,
    theme: { colors: ['primary', 'neutral', 'success', 'error'] },
  },

  // Dark first; the choice is remembered under the key Havre always used.
  // `dataValue` keeps `<html data-theme="…">` for Havre's own CSS, the class
  // (`dark` / `light`) is what Nuxt UI reads.
  colorMode: {
    preference: 'dark',
    fallback: 'dark',
    classSuffix: '',
    dataValue: 'theme',
    storageKey: 'havre-theme',
  },

  icon: {
    // Never ask the Iconify API: every icon used is in the build.
    provider: 'none',
    fallbackToApi: false,
    serverBundle: false,
    customCollections: [havreIconCollection()],
    clientBundle: { scan: true, includeCustomCollections: true },
  },

  css: [
    // The design system's fonts, bundled (nothing is fetched from Google):
    // Jost (text, titles, with its italic), Josefin Sans (small capital
    // labels), JetBrains Mono (dates, figures, code).
    '@fontsource-variable/jost',
    '@fontsource-variable/jost/wght-italic.css',
    '@fontsource-variable/josefin-sans',
    '@fontsource-variable/jetbrains-mono',
    '~/assets/css/main.css',
  ],

  app: {
    head: {
      title: 'Havre',
      htmlAttrs: { lang: 'en' },
      meta: [
        // `viewport-fit=cover`: on phones with a notch or a home bar the page
        // fills the screen, and CSS keeps clear of them with `env(safe-area-*)`.
        { name: 'viewport', content: 'width=device-width, initial-scale=1, viewport-fit=cover' },
        { name: 'color-scheme', content: 'dark light' },
        // Installed on a phone (web/public/manifest.webmanifest): the colour of the bars around it.
        { name: 'theme-color', content: '#0e0f11', media: '(prefers-color-scheme: dark)' },
        { name: 'theme-color', content: '#e9e4da', media: '(prefers-color-scheme: light)' },
        { name: 'apple-mobile-web-app-title', content: 'Havre' },
        { name: 'mobile-web-app-capable', content: 'yes' },
      ],
      link: [
        { rel: 'icon', type: 'image/svg+xml', href: '/favicon.svg' },
        // `use-credentials`: behind a sign-in proxy (Authentik…), the manifest needs your session cookie too.
        { rel: 'manifest', href: '/manifest.webmanifest', crossorigin: 'use-credentials' },
        { rel: 'apple-touch-icon', href: '/apple-touch-icon.png' },
      ],
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
