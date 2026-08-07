import { defineConfig } from '@amamo/doctrine'

export default defineConfig({
  description: {
    en: '@amamo/mdx documentation for the native MDX content compiler and its Vite and Next adapters.',
    'zh-CN': '@amamo/mdx 文档,介绍原生 MDX 内容编译器及其 Vite 与 Next 适配器。',
  },
  locales: {
    default: 'en',
    labels: { en: 'English', 'zh-CN': '简体中文' },
    names: ['en', 'zh-CN'],
  },
  siteUrl: process.env.DOCS_SITE_URL ?? 'http://localhost/',
  title: { en: '@amamo/mdx', 'zh-CN': '@amamo/mdx' },
})
