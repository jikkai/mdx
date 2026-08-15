import { defineDirectory } from '@amamo/doctrine'

export default defineDirectory({
  items: [
    { icon: 'BookOpen', page: 'index', title: '@amamo/mdx' },
    { icon: 'Rocket', page: 'getting-started', title: '快速开始' },
    { icon: 'Settings', page: 'configuration', title: '配置' },
    { icon: 'Code', page: 'compiler-api', title: '编译器 API' },
    { icon: 'Zap', page: 'vite', title: 'Vite 8' },
    { icon: 'Layers', page: 'next', title: 'Next 16' },
    { icon: 'Cpu', page: 'native-targets', title: '原生目标' },
  ],
})
