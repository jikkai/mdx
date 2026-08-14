import { defineConfig } from '@amamo/doctrine'

export default defineConfig({
  copyright: 'Copyright © 2026 白熱.',
  description:
    '@amamo/mdx documentation for the native MDX content compiler and its Vite and Next adapters.',
  githubUrl: 'https://github.com/jikkai/mdx',
  iconLibrary: 'lucide-react',
  locales: {
    default: 'en',
    labels: { en: 'English', 'zh-CN': '简体中文' },
    names: ['en', 'zh-CN'],
  },
  siteUrl: process.env.DOCS_SITE_URL ?? 'http://localhost/',
  title: '@amamo/mdx',
})
