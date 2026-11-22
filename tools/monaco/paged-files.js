import { PageLineIndex } from './line-index.js';
import { FileViewport } from './file-viewport.js';
import {
  IEditorResolverService,
  IWorkingCopyService,
  getService,
} from '@codingame/monaco-vscode-api';
import {
  SimpleEditorInput,
  SimpleEditorPane,
  registerEditorPane,
} from '@codingame/monaco-vscode-workbench-service-override/common';
import { EditorInputCapabilities } from '@codingame/monaco-vscode-api/vscode/vs/workbench/common/editor';

export const pagedEditorId = 'arcnix.largeFile';
const id = pagedEditorId;

class PageInput extends SimpleEditorInput {
  constructor(uri, entry) {
    super(uri);
    this.entry = entry;
    this.known = new Map([[0, { line: 0, column: 0 }]]);
    this.setName(uri.path.split('/').pop());
    this.addCapability(EditorInputCapabilities.Readonly);
  }
  get typeId() {
    return id;
  }
  matches(other) {
    return (
      other instanceof PageInput &&
      this.resource.toString() === other.resource.toString()
    );
  }
}

export async function registerPagedFiles(request, zh, large) {
  let searchIndex = new Map();
  const resultEntries = new WeakMap();
  const lookup = (uri) => searchIndex.get(uri.path) ?? resultEntries.get(uri);
  class PagePane extends SimpleEditorPane {
    version = 0;
    constructor(group) {
      super(id, group);
    }
    initialize() {
      const container = document.createElement('section');
      container.className = 'arcnix-paged-file';
      container.tabIndex = 0;
      return container;
    }
    focus() {
      (this.viewer?.body ?? this.container).focus();
    }
    releasePage() {
      if (this.handle != null) {
        void request('closePage', '', { handle: this.handle }).catch(() => {});
        this.handle = undefined;
      }
    }
    clearInput() {
      this.releasePage();
      this.version++;
      this.viewer?.dispose();
      super.clearInput();
    }
    renderInput(input, options, _context, token) {
      return this.show(input, options, token);
    }
    setOptions(options) {
      super.setOptions(options);
      if (this.input && options?.selection) void this.show(this.input, options);
    }
    async show(input, options, token) {
      this.releasePage();
      const version = ++this.version;
      let opening = true;
      const stale = () =>
        version !== this.version || (opening && token?.isCancellationRequested);
      this.viewer?.dispose();
      const selection = options?.selection;
      const entry = selection
        ? (lookup(input.resource) ?? input.entry)
        : undefined;
      try {
        const current =
          entry?.snapshot ??
          (await request('stat', input.resource.path)).pageSnapshot;
        if (stale()) return;
        if (!current)
          throw new Error(
            zh
              ? '文件大小已变化，请重新打开。'
              : 'File size changed. Reopen the file.',
          );
        // A previously opened tab can continue from its verified prefix.
        const resume =
          !entry &&
          input.tailStamp &&
          current.size >= input.snapshot.size &&
          (current.size > input.snapshot.size ||
            current.modified === input.snapshot.modified);
        const initial = await request('attachPage', input.resource.path, {
          snapshot: resume ? input.snapshot : current,
        });
        if (stale()) {
          if (initial.handle != null) await request('closePage', '', { handle: initial.handle });
          return;
        }
        this.handle = initial.handle;
        if (resume && String(input.tailStamp) !== String(initial.stamp))
          throw new Error(
            zh ? '文件已变化，请重新打开。' : 'File changed. Reopen the file.',
          );
        const snapshot = initial.snapshot;
        input.tailStamp = initial.stamp;
        if (
          input.snapshot?.size !== snapshot.size ||
          input.snapshot?.modified !== snapshot.modified
        ) {
          input.known = new Map([[0, { line: 0, column: 0 }]]);
          input.saved = undefined;
          input.lineIndex = new PageLineIndex();
          input.liveIndex = false;
        }
        const searched = entry ?? lookup(input.resource);
        if (
          searched?.snapshot.size === snapshot.size &&
          searched.snapshot.modified === snapshot.modified &&
          searched.lineIndex.next >= input.lineIndex.next
        )
          input.lineIndex = searched.lineIndex;
        // A split can attach to an input whose other pane is still following.
        // Share sealed slabs only; the former EOF checkpoint is mutable.
        if (input.liveIndex) {
          const seam =
            Math.floor((snapshot.size - 1) / window.arcnixWorkspace.pageBytes) *
            window.arcnixWorkspace.pageBytes;
          input.lineIndex = input.lineIndex.beforeAppend(seam, true);
        }
        input.snapshot = snapshot;
        const range =
          entry && selection
            ? {
                startLineNumber: selection.startLineNumber - 1,
                startColumn: selection.startColumn - 1,
                endLineNumber:
                  (selection.endLineNumber ?? selection.startLineNumber) - 1,
                endColumn: (selection.endColumn ?? selection.startColumn) - 1,
              }
            : undefined;
        const anchor =
          range &&
          entry.anchors.findLast(
            (a) =>
              a.line < range.startLineNumber ||
              (a.line === range.startLineNumber &&
                a.column <= range.startColumn),
          );
        this.viewer = new FileViewport(
          this.container,
          initial.handle == null ? request : (op, path, args) =>
            op === 'closeRange' ? Promise.resolve() : request(op, path, { ...args, handle: initial.handle }),
          input.resource.path,
          snapshot,
          zh,
          () => !stale(),
          input.known,
          input.lineIndex,
          initial.stamp,
          (next, stamp, index) => {
            input.snapshot = next;
            input.tailStamp = stamp;
            input.lineIndex = index;
            input.liveIndex = true;
          },
          (pos) => {
            input.saved = pos;
          },
        );
        await this.viewer.open(anchor, range, input.saved);
        opening = false;
      } catch (error) {
        if (!stale()) {
          this.viewer?.dispose();
          this.releasePage();
          this.container.textContent = error.message;
        }
      }
    }
  }
  registerEditorPane(id, zh ? '大文件查看器' : 'Large file viewer', PagePane, [
    PageInput,
  ]);
  const copies = await getService(IWorkingCopyService);
  (await getService(IEditorResolverService)).registerEditor(
    '*',
    {
      id,
      label: zh ? '大文件查看器' : 'Large file viewer',
      priority: 'exclusive',
    },
    {
      canSupportResource: (uri) =>
        uri.scheme === 'sftp' &&
        (large.has(uri.path) || !!lookup(uri)) &&
        !copies.isDirty(uri),
    },
    {
      createEditorInput: ({ resource, options }) => ({
        editor: new PageInput(resource, lookup(resource)),
        options,
      }),
    },
  );
  return {
    rememberFile(path, snapshot) {
      if (snapshot) large.set(path, snapshot);
      else large.delete(path);
    },
    beginSearch() {
      const current = (searchIndex = new Map());
      return {
        set(uri, entry) {
          resultEntries.set(uri, entry);
          current.set(uri.path, entry);
        },
        get(uri) {
          return resultEntries.get(uri);
        },
      };
    },
  };
}
