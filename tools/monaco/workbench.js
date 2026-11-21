import {
  initialize,
  getService,
  IEditorService,
  ITextModelService,
  ICommandService,
  IWorkingCopyService,
  ITextFileService,
  INotificationService,
  IQuickInputService,
  IWorkbenchLayoutService,
  ISearchService,
} from '@codingame/monaco-vscode-api';
import workbench from '@codingame/monaco-vscode-workbench-service-override';
import explorer from '@codingame/monaco-vscode-explorer-service-override';
import configuration from '@codingame/monaco-vscode-configuration-service-override';
import model from '@codingame/monaco-vscode-model-service-override';
import theme from '@codingame/monaco-vscode-theme-service-override';
import languages from '@codingame/monaco-vscode-languages-service-override';
import keys from '@codingame/monaco-vscode-keybindings-service-override';
import { Emitter } from '@codingame/monaco-vscode-api/vscode/vs/base/common/event';
import { IKeybindingService } from '@codingame/monaco-vscode-api/vscode/vs/platform/keybinding/common/keybinding.service';
import { ResolvedKeybindingItem } from '@codingame/monaco-vscode-api/vscode/vs/platform/keybinding/common/resolvedKeybindingItem';
import quickAccess from '@codingame/monaco-vscode-quickaccess-service-override';
import notifications from '@codingame/monaco-vscode-notifications-service-override';
import dialogs from '@codingame/monaco-vscode-dialogs-service-override';
import workingCopy from '@codingame/monaco-vscode-working-copy-service-override';
import '@codingame/monaco-vscode-theme-defaults-default-extension';
import { URI } from '@codingame/monaco-vscode-api/vscode/vs/base/common/uri';
import {
  EmptyFileSystemProvider,
  registerCustomProvider,
} from '@codingame/monaco-vscode-files-service-override';
import {
  createFileSystemProviderError,
  FileSystemProviderErrorCode,
} from '@codingame/monaco-vscode-api/vscode/vs/platform/files/common/files';
import {
  registerAction2,
  Action2,
  MenuId,
} from '@codingame/monaco-vscode-api/vscode/vs/platform/actions/common/actions';
import { ContextKeyExpr } from '@codingame/monaco-vscode-api/vscode/vs/platform/contextkey/common/contextkey';
import { Codicon } from '@codingame/monaco-vscode-api/vscode/vs/base/common/codicons';
import {
  KeyChord,
  KeyMod,
  KeyCode,
} from '@codingame/monaco-vscode-api/vscode/vs/base/common/keyCodes';
import {
  registerEditorPane,
  SimpleEditorInput,
  SimpleEditorPane,
} from '@codingame/monaco-vscode-workbench-service-override/common';
import MarkdownIt from 'markdown-it';
import { SearchProviderType } from '@codingame/monaco-vscode-api/vscode/vs/workbench/services/search/common/search';
import search from '@codingame/monaco-vscode-search-service-override';
import { RemoteSearch } from './search.js';
import { pagedEditorId, registerPagedFiles } from './paged-files.js';
import { codeMenuServices } from './menus.js';
import { registerCodeFormatters } from './formatters.js';
import { sshTerminalServices } from './ssh-terminal.js';
import textmate from '@codingame/monaco-vscode-textmate-service-override';
import '@codingame/monaco-vscode-docker-default-extension';
import '@codingame/monaco-vscode-ini-default-extension';
import '@codingame/monaco-vscode-javascript-default-extension';
import '@codingame/monaco-vscode-json-default-extension';
import '@codingame/monaco-vscode-markdown-basics-default-extension';
import '@codingame/monaco-vscode-python-default-extension';
import '@codingame/monaco-vscode-rust-default-extension';
import '@codingame/monaco-vscode-shellscript-default-extension';
import '@codingame/monaco-vscode-typescript-basics-default-extension';
import '@codingame/monaco-vscode-yaml-default-extension';

import { CancellationError } from '@codingame/monaco-vscode-api/vscode/vs/base/common/errors';

// Superseded searches and tree refreshes can leave a detached workbench
// promise. Its cancellation is normal when the user switches query or view.
window.addEventListener('unhandledrejection', (event) => {
  if (event.reason instanceof CancellationError) event.preventDefault();
});

const { root, zh, dark } = window.arcnixWorkspace;
const resource = (path) => URI.from({ scheme: 'sftp', path });
const send = (message) => window.ipc.postMessage(JSON.stringify(message));
const pending = new Map();
let sequence = 0;
function request(op, path, fields = {}) {
  const id = ++sequence;
  return new Promise((resolve, reject) => {
    pending.set(id, { resolve, reject });
    send({ id, op, path, ...fields });
  });
}
window.arcnixReply = (replies) => {
  for (const { id, value, error } of replies) {
    const promise = pending.get(id);
    pending.delete(id);
    if (error) {
      const failure = /not found|no such file|SFTP\(2\)/i.test(error)
        ? createFileSystemProviderError(
            error,
            FileSystemProviderErrorCode.FileNotFound,
          )
        : new Error(error);
      promise?.reject(failure);
    } else promise?.resolve(value);
  }
};

class RemoteFiles extends EmptyFileSystemProvider {
  versions = new Map();
  large = new Map();
  remember(path, snapshot) {
    if (snapshot) this.large.set(path, snapshot);
    else this.large.delete(path);
  }
  async stat(uri) {
    const stat = await request('stat', uri.path);
    this.remember(uri.path, stat.pageSnapshot);
    return stat;
  }
  async readdir(uri) {
    const entries = await request('list', uri.path);
    return entries.map(([name, type, snapshot]) => {
      this.remember(URI.joinPath(uri, name).path, snapshot);
      return [name, type];
    });
  }
  async readFile(uri) {
    let document;
    try {
      document = await request('read', uri.path);
    } catch (error) {
      if (error.code !== FileSystemProviderErrorCode.FileNotFound)
        (await getService(INotificationService)).error(error);
      throw error;
    }
    const { content, ...baseline } = document;
    if (!(await getService(IWorkingCopyService)).isDirty(uri))
      this.versions.set(uri.path, baseline);
    const text = baseline.crlf ? content.replaceAll('\n', '\r\n') : content;
    return new TextEncoder().encode(baseline.bom ? '\uFEFF' + text : text);
  }
  mkdir(uri) {
    return request('mkdir', uri.path);
  }
  async writeFile(uri, bytes, options) {
    if (
      options.create &&
      !bytes.length &&
      (!options.overwrite || !this.versions.has(uri.path))
    ) {
      await request('create', uri.path);
      return;
    }
    const baseline = this.versions.get(uri.path);
    if (!baseline)
      throw new Error(
        zh ? '请先打开远程文件' : 'Open the remote file before saving',
      );
    const content = new TextDecoder('utf-8', { fatal: true })
      .decode(bytes)
      .replaceAll('\r\n', '\n');
    baseline.version = await request('write', uri.path, {
      ...baseline,
      content,
    });
  }
}
const remoteFiles = new RemoteFiles();
registerCustomProvider('sftp', remoteFiles);
window.MonacoEnvironment = {
  getWorker: (_, label) =>
    new Worker(
      label === 'TextMateWorker' ? '/grammar-worker.js' : '/worker.js',
      { type: 'module' },
    ),
};

const previewId = 'arcnix.markdown.preview';
class PreviewInput extends SimpleEditorInput {
  constructor(uri) {
    super(uri);
    this.setName((zh ? '预览 ' : 'Preview ') + uri.path.split('/').pop());
  }
  get typeId() {
    return previewId;
  }
  matches(other) {
    return (
      other instanceof PreviewInput &&
      other.resource.toString() === this.resource.toString()
    );
  }
}
const markdown = new MarkdownIt({ html: false });
class PreviewPane extends SimpleEditorPane {
  constructor(group) {
    super(previewId, group);
  }
  initialize() {
    const el = document.createElement('article');
    el.className = 'arcnix-preview';
    return el;
  }
  async renderInput(input, options, context, token) {
    const ref = await (
      await getService(ITextModelService)
    ).createModelReference(input.resource);
    if (token.isCancellationRequested) {
      ref.dispose();
      return;
    }
    const model = ref.object.textEditorModel;
    const images = new Map();
    const render = () => {
      const template = document.createElement('template');
      template.innerHTML = markdown.render(model.getValue());
      for (const anchor of template.content.querySelectorAll('a'))
        anchor.removeAttribute('href');
      for (const img of template.content.querySelectorAll('img')) {
        const src = img.getAttribute('src');
        img.removeAttribute('src');
        if (!src || /^[a-z][a-z\d+.-]*:|^[/\\]/i.test(src)) continue;
        try {
          const target = new URL(
            src,
            input.resource
              .with({ scheme: 'https', authority: 'workspace.invalid' })
              .toString(),
          );
          if (target.origin !== 'https://workspace.invalid') continue;
          const path = decodeURIComponent(target.pathname);
          if (!images.has(path))
            images.set(
              path,
              request('image', path).catch(() => null),
            );
          images.get(path).then((url) => {
            if (url && img.isConnected) img.src = url;
          });
        } catch {}
      }
      this.container.replaceChildren(template.content);
    };
    render();
    const listener = model.onDidChangeContent(render);
    return {
      dispose() {
        listener.dispose();
        ref.dispose();
      },
    };
  }
}
registerEditorPane(previewId, 'Markdown preview', PreviewPane, [PreviewInput]);
const isMarkdown = ContextKeyExpr.and(
  ContextKeyExpr.regex('resourceFilename', /\.(md|markdown|mdown|mkdn?)$/i),
  ContextKeyExpr.notEquals('activeEditor', previewId),
  ContextKeyExpr.notEquals('activeEditor', pagedEditorId),
);
registerAction2(
  class extends Action2 {
    constructor() {
      super({
        id: previewId,
        title: zh
          ? 'Markdown: 打开侧边预览'
          : 'Markdown: Open preview to the side',
        icon: Codicon.openPreview,
        f1: true,
        precondition: isMarkdown,
        menu: { id: MenuId.EditorTitle, group: 'navigation', when: isMarkdown },
        keybinding: {
          weight: 200,
          primary: KeyChord(KeyMod.CtrlCmd | KeyCode.KeyK, KeyCode.KeyV),
        },
      });
    }
    async run(accessor, target) {
      const editors = accessor.get(IEditorService);
      const uri =
        target?.scheme === 'sftp' ? target : editors.activeEditor?.resource;
      if (uri?.scheme === 'sftp')
        await editors.openEditor(
          new PreviewInput(uri),
          { pinned: true, revealIfOpened: true },
          -2,
        );
    }
  },
);
await initialize(
  {
    ...workbench(),
    ...explorer(),
    ...search(),
    ...codeMenuServices(),
    ...sshTerminalServices(root, send),
    ...configuration(),
    ...model(),
    ...languages(),
    ...textmate(),
    ...theme(),
    ...keys(),
    ...quickAccess({ shouldUseGlobalPicker: () => true }),
    ...notifications(),
    ...dialogs(),
    ...workingCopy({ storage: 'memory' }),
  },
  document.getElementById('workbench'),
  {
    workspaceProvider: {
      workspace: { folderUri: resource(root) },
      trusted: false,
      open: async () => false,
    },
    productConfiguration: { nameShort: 'Code', nameLong: 'Arcnix Code' },
    configurationDefaults: {
      'editor.editContext': false,
      'editor.minimap.enabled': false,
      'workbench.startupEditor': 'none',
      'workbench.colorTheme': dark ? 'Dark Modern' : 'Light Modern',
      'workbench.iconTheme': 'vs-minimal',
      'workbench.editor.enablePreview': false,
      'search.showLineNumbers': true,
      'explorer.openEditors.visible': 0,
      'window.commandCenter': false,
      'window.menuBarVisibility': 'visible',
      'terminal.integrated.enablePersistentSessions': false,
      'terminal.integrated.tabs.title': 'SSH',
      'terminal.integrated.shellIntegration.enabled': false,
      'terminal.integrated.gpuAcceleration': 'off',
      'workbench.layoutControl.enabled': false,
      'files.autoSave': 'off',
    },
  },
);

const searchService = await getService(ISearchService);
await registerCodeFormatters(zh);
const pagedFiles = await registerPagedFiles(request, zh, remoteFiles.large);
const remoteSearch = new RemoteSearch(root, request, zh, pagedFiles);
for (const type of [SearchProviderType.file, SearchProviderType.text])
  searchService.registerSearchResultProvider('sftp', type, remoteSearch);
const editors = await getService(IEditorService);
const commands = await getService(ICommandService);
window.arcnixCommand = (id) => commands.executeCommand(id);
window.arcnixOpen = async (path) => {
  const uri = resource(path);
  try {
    await remoteFiles.stat(uri);
    return await editors.openEditor({
      resource: uri,
      options: { pinned: true },
    });
  } catch (error) {
    (await getService(INotificationService)).error(error);
  }
};
// The native child uses physical pixels; measure the command slot in its own
// CSS pixels so the original picker keeps keyboard focus, caret and IME.
const workbenchLayout = await getService(IWorkbenchLayoutService);
new ResizeObserver(() => workbenchLayout.layout()).observe(
  document.getElementById('workbench'),
);
window.arcnixCommandSlot = (physical, titleColors) => {
  const style = document.documentElement.style;
  if (titleColors) {
    style.setProperty('--command-surround', titleColors[0]);
    style.setProperty('--command-foreground', titleColors[1]);
  }
  const [left, width, height, inset, menuLeft, menuWidth] = physical ?? [0, 0, 0, 0, 0, 0];
  for (const [name, n] of Object.entries({ left, width, height, inset, 'menu-left': menuLeft, 'menu-width': menuWidth }))
    style.setProperty('--command-' + name, n / window.devicePixelRatio + 'px');
  document.documentElement.classList.toggle('has-command-slot', !!physical);
  document.documentElement.classList.toggle('has-menu-slot', menuWidth > 0);
};
const titlebar = document.querySelector('.part.titlebar');
new ResizeObserver(() => document.documentElement.style.setProperty('--menu-height', titlebar.offsetHeight + 'px')).observe(titlebar);
const quickInput = await getService(IQuickInputService);
function setTitlePicker(visible) {
  document.documentElement.classList.toggle('title-picker', visible);
  send({ op: 'titlePicker', visible });
}
quickInput.onShow(() => {
  const picker = quickInput.currentQuickInput;
  setTitlePicker(!!picker?.contextKey && !picker.hideInput && !picker.anchor);
});
quickInput.onHide(() => setTitlePicker(false));
const copies = await getService(IWorkingCopyService);
copies.onDidChangeDirty(() => send({ op: 'dirty', value: copies.hasDirty }));
await (
  await getService(ICommandService)
).executeCommand('workbench.view.explorer');
const textFiles = await getService(ITextFileService);
const saveCommand = 'workbench.action.files.save';
const keybindings = await getService(IKeybindingService);
const saveShortcutChanged = new Emitter();
const nativeSave = /Mac/.test(navigator.platform) ? 'Meta+S' : 'Ctrl+S';
keybindings.registerKeybindingProvider({
  onDidChangeKeybindings: saveShortcutChanged.event,
  provideKeybindings: () => {
    const bindings = window.arcnixWorkspace.saveShortcut;
    const custom = bindings.filter((binding) => binding !== nativeSave).flatMap((binding) => {
      const layout = binding.toLowerCase().replace('meta+', 'cmd+').replace(/arrow(up|down|left|right)/, '$1');
      return keybindings.resolveUserBinding(layout);
    });
    const items = custom.map((key) => new ResolvedKeybindingItem(key, saveCommand, undefined, undefined, false, null, false));
    if (!bindings.includes(nativeSave))
      items.unshift(new ResolvedKeybindingItem(undefined, '-' + saveCommand, undefined, undefined, false, null, false));
    return items;
  },
});
window.arcnixSetSaveShortcut = (binding) => {
  if (window.arcnixWorkspace.saveShortcut === binding) return;
  window.arcnixWorkspace.saveShortcut = binding;
  saveShortcutChanged.fire();
};
document.addEventListener(
  'keydown',
  (event) => {
    if (
      (event.ctrlKey || event.metaKey) &&
      !event.altKey &&
      !event.shiftKey &&
      !event.isComposing &&
      event.key.toLowerCase() === 'p' &&
      !quickInput.currentQuickInput
    ) {
      event.preventDefault();
      event.stopImmediatePropagation();
      void commands.executeCommand('workbench.action.quickOpen');
      return;
    }
    if (event.target.closest?.('.terminal-wrapper, .xterm')) return;
    const save = window.arcnixChordMatches(event, window.arcnixWorkspace.saveShortcut);
    // Suppress the bundled workbench's original binding after remapping it.
    if (!save && !window.arcnixChordMatches(event, nativeSave)) return;
    event.preventDefault();
    event.stopImmediatePropagation();
    if (!save || event.repeat) return;
    const editorUri = event.target
      .closest?.('.monaco-editor[data-uri]')
      ?.getAttribute('data-uri');
    const uri = editorUri
      ? URI.parse(editorUri)
      : editors.activeEditor?.resource;
    if (
      uri?.scheme === 'sftp' &&
      (editorUri || editors.activeEditor?.typeId !== pagedEditorId)
    )
      void textFiles.save(uri, { reason: 1 });
  },
  true,
);
window.addEventListener('focus', () => send({ op: 'focus' }));
send({ op: 'ready' });
