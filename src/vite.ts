import type { Plugin, ViteDevServer } from 'vite'

import type { IAmamoMdxConfig } from './config.js'
import type { IAdapterCompiler } from './compiler.js'
import { createAdapterCompiler } from './compiler.js'

export function amamoMdx(config: IAmamoMdxConfig): Plugin {
  let compilerPromise: Promise<IAdapterCompiler> | undefined
  let startup: Promise<unknown> | undefined
  let disposed = false

  function compiler(): Promise<IAdapterCompiler> {
    compilerPromise ??= createAdapterCompiler(config)
    return compilerPromise
  }

  function ensureBuild(): Promise<unknown> {
    startup ??= compiler().then((instance) => instance.build())
    return startup
  }

  async function dispose(): Promise<void> {
    if (disposed || !compilerPromise) return
    disposed = true
    const instance = await compilerPromise
    await instance.dispose()
  }

  return {
    name: 'amamo-mdx',
    enforce: 'pre',
    sharedDuringBuild: true,
    async buildStart() {
      await ensureBuild()
    },
    async transform(_source, id) {
      const instance = await compiler()
      if (!instance.isContentFile(id)) return null
      const result = await instance.transform(id)
      return { code: result.code, map: result.map }
    },
    async configureServer(server) {
      await ensureBuild()
      const instance = await compiler()
      const update = async (file: string) => {
        if (!instance.isContentFile(file)) return
        try {
          const result = await instance.transform(file)
          if (result.outputsWritten > 0) invalidateGeneratedModule(server, instance)
        } catch (error) {
          reportWatchError(server, error)
        }
      }
      const remove = async (file: string) => {
        if (!instance.isContentFile(file)) return
        try {
          if (await instance.remove(file)) invalidateGeneratedModule(server, instance)
        } catch (error) {
          reportWatchError(server, error)
        }
      }
      server.watcher.on('add', update)
      server.watcher.on('change', update)
      server.watcher.on('unlink', remove)
      server.httpServer?.once('close', async () => {
        server.watcher.off('add', update)
        server.watcher.off('change', update)
        server.watcher.off('unlink', remove)
        try {
          await dispose()
        } catch (error) {
          server.config.logger.error(asError(error).stack ?? asError(error).message)
        }
      })
    },
  }
}

function invalidateGeneratedModule(server: ViteDevServer, compiler: IAdapterCompiler): void {
  const module = server.moduleGraph.getModuleById(compiler.generatedCollectionModule)
  if (module) server.moduleGraph.invalidateModule(module)
  server.hot.send({ type: 'full-reload' })
}

function reportWatchError(server: ViteDevServer, error: unknown): void {
  const value = asError(error)
  server.config.logger.error(value.stack ?? value.message)
  server.hot.send({
    type: 'error',
    err: {
      message: value.message,
      stack: value.stack ?? value.message,
    },
  })
}

function asError(error: unknown): Error {
  return error instanceof Error ? error : new Error(String(error))
}
