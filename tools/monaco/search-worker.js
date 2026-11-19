import {
  createRegExp,
  fuzzyContains,
} from '@codingame/monaco-vscode-api/vscode/vs/base/common/strings';
import {
  evaluateExcludeTreeEntry,
  match as globMatch,
} from '@codingame/monaco-vscode-api/vscode/vs/base/common/glob';
import { IgnoreFile } from '@codingame/monaco-vscode-api/vscode/vs/workbench/services/search/common/ignoreFile';
import { QueryGlobTester } from '@codingame/monaco-vscode-api/vscode/vs/workbench/services/search/common/search';
import {
  editorMatchesToTextSearchResults,
  getTextSearchMatchWithModelContext,
} from '@codingame/monaco-vscode-api/vscode/vs/workbench/services/search/common/searchHelpers';

let reply;
import { scanLargeFile } from './large-search.js';

function request(kind, path, fields = {}) {
  return new Promise((resolve, reject) => {
    reply = (data) =>
      data.error ? reject(new Error(data.error)) : resolve(data.value);
    self.postMessage({ kind, path, ...fields });
  });
}

// Editor match ranges use one-based UTF-16 positions in Code's normalized text.
function matchText(text, pattern, query, quota) {
  const lines = text.split('\n');
  const matches = [];
  let line = 0;
  let offset = 0;
  function position(index) {
    while (line + 1 < lines.length && index > offset + lines[line].length) {
      offset += lines[line++].length + 1;
    }
    return [line + 1, index - offset + 1];
  }
  let limitHit = false;
  for (const found of text.matchAll(pattern)) {
    if (matches.length >= quota) {
      limitHit = true;
      break;
    }
    const [startLineNumber, startColumn] = position(found.index);
    const [endLineNumber, endColumn] = position(found.index + found[0].length);
    const range = { startLineNumber, startColumn, endLineNumber, endColumn };
    matches.push({ range });
  }
  const model = {
    getLineCount: () => lines.length,
    getLineContent: (n) => lines[n - 1],
  };
  return {
    count: matches.length,
    matches: getTextSearchMatchWithModelContext(
      editorMatchesToTextSearchResults(matches, model, query.previewOptions),
      model,
      query,
    ),
    limitHit,
  };
}

async function search(query, root) {
  const content = query.contentPattern;
  const pattern =
    content &&
    createRegExp(content.pattern, content.isRegExp, {
      wholeWord: content.isWordMatch,
      matchCase: content.isCaseSensitive,
      global: true,
      multiline: true,
      unicode: true,
    });
  let remaining = query.maxResults ?? 10000;
  let limitHit = false;
  let skipped = 0;
  const seen = new Set();
  for (const folder of query.folderQueries) {
    const base = folder.folder.path;
    const prefix = root.endsWith('/') ? root : root + '/';
    if (
      folder.folder.scheme !== 'sftp' ||
      (base !== root && !base.startsWith(prefix)) ||
      base.split('/').includes('..')
    )
      continue;
    const tester = new QueryGlobTester(query, folder);
    const stack = [
      { path: base, relative: '', ignored: undefined, excluded: false },
    ];
    while (stack.length && !limitHit) {
      const dir = stack.pop();
      let entries;
      try {
        entries = await request('list', dir.path);
      } catch {
        skipped++;
        continue;
      }
      const names = new Set(entries.map(([name]) => name));
      const hasSibling = (name) => names.has(name);
      const childPath = (name) => dir.path.replace(/\/$/, '') + '/' + name;
      let ignored = dir.ignored;
      if (!folder.disregardIgnoreFiles) {
        for (const name of ['.gitignore', '.ignore']) {
          if (!names.has(name)) continue;
          try {
            const rules = await request('read', childPath(name));
            ignored = new IgnoreFile(
              rules,
              dir.relative ? '/' + dir.relative + '/' : '/',
              ignored,
              folder.ignoreGlobCase,
            );
          } catch {
            skipped++;
          }
        }
      }
      for (const [name, directory, snapshot, paged] of entries) {
        const relative = dir.relative ? dir.relative + '/' + name : name;
        const path = childPath(name);
        const { isExcluded, isVisited } = evaluateExcludeTreeEntry(tester, {
          path: relative,
          basename: name,
          isDirectory: directory,
          hasSibling,
          isParentExcluded: dir.excluded,
          isHidden: () =>
            ignored &&
            !ignored.isPathIncludedInTraversal('/' + relative, directory),
        });
        if (!isVisited) continue;
        if (directory) {
          stack.push({ path, relative, ignored, excluded: isExcluded });
          continue;
        }
        if (
          !tester.includedInQuerySync(relative, name, hasSibling) ||
          seen.has(path)
        )
          continue;
        seen.add(path);
        if (pattern) {
          let found;
          try {
            if (paged) {
              found = await scanLargeFile(
                path,
                snapshot,
                pattern,
                remaining,
                request,
                (matches, anchor) => {
                  remaining -= matches.length;
                  self.postMessage({
                    kind: 'match',
                    path,
                    snapshot,
                    matches,
                    anchor,
                  });
                },
              );
            } else {
              found = matchText(
                await request('read', path),
                pattern,
                query,
                remaining,
              );
              if (found.count)
                self.postMessage({
                  kind: 'match',
                  path,
                  matches: found.matches,
                });
              remaining -= found.count;
            }
          } catch (error) {
            if (!/utf-?8|line endings/i.test(error.message)) skipped++;
            continue;
          }
          limitHit = found.limitHit;
        } else {
          const filePattern = query.filePattern;
          if (
            filePattern &&
            !(query.shouldGlobMatchFilePattern
              ? globMatch(filePattern, relative)
              : fuzzyContains(
                  relative.toLowerCase(),
                  filePattern.toLowerCase(),
                ))
          )
            continue;
          if (query.exists || remaining === 0) limitHit = true;
          else {
            self.postMessage({
              kind: 'match',
              path,
              fileSnapshot: paged ? snapshot : null,
            });
            remaining--;
          }
        }
        if (limitHit) break;
      }
    }
  }
  self.postMessage({ kind: 'done', limitHit, skipped });
}

self.onmessage = ({ data }) => {
  if (data.query)
    search(data.query, data.root).catch((error) =>
      self.postMessage({ kind: 'error', message: error.message }),
    );
  else reply(data);
};
