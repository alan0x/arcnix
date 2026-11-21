import {
  IQuickInputService,
  IStorageService,
} from '@codingame/monaco-vscode-api';
import { registerServiceInitializePreParticipant } from '@codingame/monaco-vscode-api/lifecycle';
import { AccountsActivityActionViewItem } from '@codingame/monaco-vscode-api/vscode/vs/workbench/browser/parts/globalCompositeBar';
import {
  StorageScope,
  StorageTarget,
} from '@codingame/monaco-vscode-api/vscode/vs/platform/storage/common/storage';
import { IMenuService } from '@codingame/monaco-vscode-api/vscode/vs/platform/actions/common/actions.service';
import { MenuService } from '@codingame/monaco-vscode-api/vscode/vs/platform/actions/common/menuService';
import { IContextMenuService } from '@codingame/monaco-vscode-api/vscode/vs/platform/contextview/browser/contextView.service';
import { ContextMenuService } from '@codingame/monaco-vscode-api/vscode/vs/platform/contextview/browser/contextMenuService';
import {
  Action2,
  MenuId,
  MenuRegistry,
  registerAction2,
} from '@codingame/monaco-vscode-api/vscode/vs/platform/actions/common/actions';
import { SyncDescriptor } from '@codingame/monaco-vscode-api/vscode/vs/platform/instantiation/common/descriptors';
import { ContextKeyExpr } from '@codingame/monaco-vscode-api/vscode/vs/platform/contextkey/common/contextkey';
import { TEXT_FILE_EDITOR_ID } from '@codingame/monaco-vscode-api/vscode/vs/workbench/contrib/files/common/files';
import { IWorkbenchThemeService } from '@codingame/monaco-vscode-api/vscode/vs/workbench/services/themes/common/workbenchThemeService.service';

const localPreferences = new MenuId('arcnix.code.preferences');
const mainMenu = new MenuId('arcnix.code.menubar');
const emptyMenu = new MenuId('arcnix.code.empty');
const localMenu = (id) =>
  id === MenuId.MenubarMainMenu ? mainMenu :
  id === MenuId.MenubarHomeMenu ? emptyMenu :
  id === MenuId.GlobalActivity || id === MenuId.MenubarPreferencesMenu
    ? localPreferences
    : id;

class CodeMenuService extends MenuService {
  createMenu(id, ...args) {
    return super.createMenu(localMenu(id), ...args);
  }
  getMenuActions(id, ...args) {
    return super.getMenuActions(localMenu(id), ...args);
  }
}

class CodeContextMenus extends ContextMenuService {
  showContextMenu(delegate) {
    if (!delegate.getActions) return super.showContextMenu(delegate);
    super.showContextMenu({
      ...delegate,
      getActions: () =>
        delegate
          .getActions()
          .filter(
            (action) =>
              !['toggleAccountsVisibility', 'toggle.hideAccounts'].includes(
                action.id,
              ),
          ),
    });
  }
}

export function codeMenuServices() {
  registerServiceInitializePreParticipant((accessor) => {
    accessor
      .get(IStorageService)
      .store(
        AccountsActivityActionViewItem.ACCOUNTS_VISIBILITY_PREFERENCE_KEY,
        false,
        StorageScope.PROFILE,
        StorageTarget.MACHINE,
      );
  });
  const zh = window.arcnixWorkspace.zh;
  // The upstream menubar supplies keyboard navigation, shortcuts and overflow.
  // Only contribute actions backed by this workbench or the SSH provider.
  function menu(en, cn, entries) {
    const submenu = new MenuId('arcnix.code.menu.' + en);
    MenuRegistry.appendMenuItem(mainMenu, {
      submenu, title: { original: en, value: zh ? cn : en }, order: menuOrder++,
    });
    for (const [order, [id, label, translated, precondition]] of entries.entries()) {
      MenuRegistry.appendMenuItem(submenu, {
        command: { id, title: zh ? translated : label, precondition }, order,
      });
    }
  }
  let menuOrder = 0;
  const active = ContextKeyExpr.has('activeEditor');
  const text = ContextKeyExpr.equals('activeEditor', TEXT_FILE_EDITOR_ID);
  const writable = ContextKeyExpr.and(text, ContextKeyExpr.not('activeEditorReadonly'));
  registerAction2(class extends Action2 {
    constructor() {
      super({
        id: 'arcnix.files.browse',
        title: zh ? '打开文件…' : 'Open file…',
      });
    }
    run() {
      window.ipc.postMessage(JSON.stringify({ op: 'browseFiles' }));
    }
  });
  menu('File', '文件', [
    ['explorer.newFile', 'New file…', '新建文件…'],
    ['arcnix.files.browse', 'Open file…', '打开文件…'],
    ['workbench.action.files.save', 'Save', '保存', writable],
    ['workbench.action.closeActiveEditor', 'Close editor', '关闭编辑器', active],
  ]);
  menu('Edit', '编辑', [
    ['undo', 'Undo', '撤销', writable], ['redo', 'Redo', '重做', writable],
    ['actions.find', 'Find', '查找', text], ['editor.action.startFindReplaceAction', 'Replace', '替换', writable],
  ]);
  menu('Selection', '选择', [
    ['editor.action.selectAll', 'Select all', '全选', text],
    ['editor.action.copyLinesDownAction', 'Copy line down', '向下复制行', writable],
    ['editor.action.moveLinesUpAction', 'Move line up', '向上移动行', writable],
    ['editor.action.moveLinesDownAction', 'Move line down', '向下移动行', writable],
  ]);
  menu('View', '视图', [
    ['workbench.view.explorer', 'Explorer', '资源管理器'],
    ['workbench.view.search', 'Search', '搜索'],
    ['arcnix.markdown.preview', 'Open Markdown preview to the side', '打开 Markdown 侧边预览',
      ContextKeyExpr.and(text, ContextKeyExpr.regex('resourceFilename', /\.(md|markdown|mdown|mkdn?)$/i))],
    ['workbench.action.showCommands', 'Command palette…', '命令面板…'],
  ]);
  menu('Go', '转到', [
    ['workbench.action.navigateBack', 'Back', '后退', ContextKeyExpr.has('canNavigateBack')],
    ['workbench.action.navigateForward', 'Forward', '前进', ContextKeyExpr.has('canNavigateForward')],
    ['workbench.action.gotoLine', 'Go to line…', '转到行…', text],
  ]);
  menu('Terminal', '终端', [
    ['workbench.action.terminal.new', 'New terminal', '新建终端'],
    ['workbench.action.terminal.toggleTerminal', 'Toggle terminal', '显示或隐藏终端'],
    ['workbench.action.terminal.kill', 'Kill active terminal', '关闭活动终端', ContextKeyExpr.has('terminalIsOpen')],
  ]);
  function localThemePicker(id, title, list, current, apply) {
    registerAction2(
      class extends Action2 {
        constructor() {
          super({ id, title, menu: { id: localPreferences, group: 'themes' } });
        }
        async run(accessor) {
          const themes = accessor.get(IWorkbenchThemeService);
          const picker = accessor.get(IQuickInputService);
          const options = (await list(themes)).map((theme) => ({
            label: theme.label,
            id: theme.id,
          }));
          const choice = await picker.pick(options, {
            placeHolder: title,
            activeItem: options.find((item) => item.id === current(themes).id),
          });
          if (choice) await apply(themes, choice.id);
        }
      },
    );
  }
  localThemePicker(
    'arcnix.code.colorTheme',
    zh ? '颜色主题' : 'Color theme',
    (service) => service.getColorThemes(),
    (service) => service.getColorTheme(),
    (service, id) => service.setColorTheme(id, 'auto'),
  );
  localThemePicker(
    'arcnix.code.fileIconTheme',
    zh ? '文件图标主题' : 'File icon theme',
    async (service) => [
      { id: '', label: zh ? '无' : 'None' },
      ...(await service.getFileIconThemes()),
    ],
    (service) => service.getFileIconTheme(),
    (service, id) => service.setFileIconTheme(id, 'auto'),
  );
  MenuRegistry.appendMenuItem(localPreferences, {
    command: {
      id: 'workbench.action.showCommands',
      title: zh ? '命令面板…' : 'Command palette…',
    },
    group: 'commands',
  });
  return {
    [IMenuService.toString()]: new SyncDescriptor(CodeMenuService),
    [IContextMenuService.toString()]: new SyncDescriptor(CodeContextMenus),
  };
}
