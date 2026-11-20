import { getService, INotificationService } from '@codingame/monaco-vscode-api';
import { ILanguageFeaturesService } from '@codingame/monaco-vscode-api/vscode/vs/editor/common/services/languageFeatures.service';
import '@codingame/monaco-vscode-api/vscode/vs/editor/contrib/format/browser/formatActions';

const parsers = {
  javascript: 'babel', javascriptreact: 'babel',
  typescript: 'typescript', typescriptreact: 'typescript',
  json: 'json-stringify', jsonc: 'jsonc', css: 'css', scss: 'scss', less: 'less', html: 'html',
};
// CSS and HTML may be opened as plain text without their language extensions.
const otherFiles = { css: 'css', scss: 'scss', less: 'less', html: 'html', htm: 'html' };

async function formatDocument(input, token) {
  const worker = new Worker('/format-worker.js', { type: 'module' });
  let cancel;
  try {
    return await new Promise((resolve, reject) => {
      cancel = token.onCancellationRequested(() => resolve(undefined));
      worker.onmessage = ({ data }) => data.error
        ? reject(new Error(data.error))
        : resolve(data.formatted);
      worker.onerror = (event) => reject(new Error(event.message || 'Formatter could not start'));
      worker.postMessage(input);
    });
  } finally {
    cancel?.dispose();
    worker.terminate();
  }
}

export async function registerCodeFormatters(zh) {
  const features = await getService(ILanguageFeaturesService);
  const notifications = await getService(INotificationService);
  features.documentFormattingEditProvider.register([
    ...Object.keys(parsers).map((language) => ({ scheme: 'sftp', language })),
    { scheme: 'sftp', language: 'plaintext', pattern: '**/*.{css,scss,less,html,htm}' },
  ], {
    displayName: 'Prettier',
    async provideDocumentFormattingEdits(model, options, token) {
      const parser = parsers[model.getLanguageId()] ?? otherFiles[model.uri.path.split('.').at(-1)];
      if (!parser || token.isCancellationRequested) return [];
      const version = model.getVersionId();
      const text = model.getValue();
      try {
        const formatted = await formatDocument({
          text, parser, tabWidth: options.tabSize, useTabs: !options.insertSpaces,
          endOfLine: model.getEOL() === '\r\n' ? 'crlf' : 'lf',
        }, token);
        if (token.isCancellationRequested || model.isDisposed() || model.getVersionId() !== version
          || formatted == null || formatted === text) return [];
        return [{ range: model.getFullModelRange(), text: formatted }];
      } catch (error) {
        if (!token.isCancellationRequested && !model.isDisposed() && model.getVersionId() === version) {
          const detail = String(error.message).split('\n', 1)[0];
          notifications.error(zh ? `无法格式化文档：${detail}` : `Unable to format document: ${detail}`);
        }
        return [];
      }
    },
  });
}
