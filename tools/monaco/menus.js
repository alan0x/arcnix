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
import { IWorkbenchThemeService } from '@codingame/monaco-vscode-api/vscode/vs/workbench/services/themes/common/workbenchThemeService.service';

const localPreferences = new MenuId('arcnix.code.preferences');
const localMenu = (id) =>
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
    zh ? '颜色主题' : 'Color Theme',
    (service) => service.getColorThemes(),
    (service) => service.getColorTheme(),
    (service, id) => service.setColorTheme(id, 'auto'),
  );
  localThemePicker(
    'arcnix.code.fileIconTheme',
    zh ? '文件图标主题' : 'File Icon Theme',
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
      title: zh ? '命令面板…' : 'Command Palette…',
    },
    group: 'commands',
  });
  return {
    [IMenuService.toString()]: new SyncDescriptor(CodeMenuService),
    [IContextMenuService.toString()]: new SyncDescriptor(CodeContextMenus),
  };
}
