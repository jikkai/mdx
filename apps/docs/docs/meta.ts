import { defineDirectory } from '@amamo/doctrine'

export default defineDirectory({
  items: [
    { icon: 'BookOpen', page: 'index', title: '@amamo/mdx' },
    { icon: 'Rocket', page: 'getting-started', title: 'Getting started' },
    { icon: 'Settings', page: 'configuration', title: 'Configuration' },
    { icon: 'Code', page: 'compiler-api', title: 'Compiler API' },
    { icon: 'Zap', page: 'vite', title: 'Vite 8' },
    { icon: 'Layers', page: 'next', title: 'Next 16' },
    { icon: 'Cpu', page: 'native-targets', title: 'Native targets' },
  ],
})
