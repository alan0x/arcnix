import { PageLineIndex, nextReadCursor } from './line-index.js';
import { URI } from '@codingame/monaco-vscode-api/vscode/vs/base/common/uri';
import { CancellationError } from '@codingame/monaco-vscode-api/vscode/vs/base/common/errors';

// One worker per query lets Stop cancel even a busy regular expression.
// SFTP operations stay serialized on the existing SSH worker.
export class RemoteSearch {
  constructor(root, request, zh, pagedFiles) {
    this.root = root;
    this.request = request;
    this.zh = zh;
    this.pagedFiles = pagedFiles;
  }
  clearCache() {
    return Promise.resolve();
  }
  getAIName() {
    return Promise.resolve(undefined);
  }
  fileSearch(query, token) {
    return this.run(query, undefined, token);
  }
  textSearch(query, progress, token) {
    return this.run(query, progress, token);
  }

  async run(query, progress, token) {
    const results = [];
    const matchesByPath = new Map();
    if (query.onlyOpenEditors)
      return { results, messages: [], limitHit: false };
    if (token?.isCancellationRequested) throw new CancellationError();
    progress?.({
      message: this.zh ? '正在通过 SFTP 搜索…' : 'Searching over SFTP…',
    });
    const pages = query.contentPattern
      ? this.pagedFiles.beginSearch()
      : undefined;
    const worker = new Worker('/search-worker.js', { type: 'module' });
    const cursor = nextReadCursor();
    let indexedPath, lineIndex;
    let usedReader = false;
    let cancellation;
    try {
      return await new Promise((resolve, reject) => {
        cancellation = token?.onCancellationRequested(() =>
          reject(new CancellationError()),
        );
        worker.onerror = (e) => reject(new Error(e.message));
        worker.onmessage = async ({ data }) => {
          if (token?.isCancellationRequested) return;
          switch (data.kind) {
            case 'range': {
              try {
                usedReader = true;
                const value = await this.request('readRange', data.path, {
                  offset: data.offset,
                  snapshot: data.snapshot,
                  cursor,
                });
                if (indexedPath !== data.path) {
                  indexedPath = data.path;
                  lineIndex = new PageLineIndex();
                }
                lineIndex.append(data.offset, value);
                worker.postMessage({ value });
              } catch (error) {
                worker.postMessage({ error: error.message });
              }
              break;
            }
            case 'read':
            case 'list': {
              try {
                const value =
                  data.kind === 'read'
                    ? (await this.request('read', data.path)).content
                    : await this.request('searchDirectory', data.path, {
                        max_bytes: query.contentPattern
                          ? query.maxFileSize || Number.MAX_SAFE_INTEGER
                          : null,
                      });
                worker.postMessage({ value });
              } catch (error) {
                worker.postMessage({ error: error.message });
              }
              break;
            }
            case 'match': {
              const existing = matchesByPath.get(data.path);
              const resource =
                existing?.resource ??
                URI.from({ scheme: 'sftp', path: data.path });
              if (data.fileSnapshot !== undefined)
                this.pagedFiles.rememberFile(data.path, data.fileSnapshot);
              if (data.anchor) {
                if (!existing)
                  pages.set(resource, {
                    snapshot: data.snapshot,
                    lineIndex,
                    anchors: [],
                    first: data.matches[0].rangeLocations[0].source,
                  });
                pages.get(resource).anchors.push(data.anchor);
              }
              const match = {
                resource,
                results: data.matches,
              };
              if (existing) {
                for (const item of data.matches ?? [])
                  existing.results.push(item);
              } else {
                const complete = { ...match, results: data.matches?.slice() };
                matchesByPath.set(data.path, complete);
                results.push(complete);
              }
              progress?.(match);
              break;
            }
            case 'done': {
              const messages = data.skipped
                ? [
                    {
                      type: 2,
                      text: this.zh
                        ? `有 ${data.skipped} 个文件或目录无法读取。`
                        : `Could not read ${data.skipped} files or folders.`,
                    },
                  ]
                : [];
              resolve({ results, limitHit: data.limitHit, messages });
              break;
            }
            case 'error':
              reject(new Error(data.message));
              break;
          }
        };
        worker.postMessage({
          query: JSON.parse(JSON.stringify(query)),
          root: this.root,
        });
      });
    } finally {
      cancellation?.dispose();
      worker.terminate();
      if (usedReader) await this.request('closeRange', '', { cursor });
    }
  }
}
