import {mountWriter} from './embedded-writer.js';

const status = document.getElementById('message');
const post = value => window.ipc.postMessage(value);
const editor = mountWriter({
    root: document.getElementById('commands'), selector:'#body', status, options:window.officeOptions,
    onChange(raw, revision) {
        if (raw.startsWith('{"__writer_plain_text":')) {
            post('plain\n' + revision + '\n' + JSON.parse(raw).__writer_plain_text);
        } else post('model\n' + raw);
    },
    onSave: () => post('save'), onRead: () => post('discard'),
});
window.officeRestore = editor.restore;
document.addEventListener('click', e => { if (e.target.closest('a')) e.preventDefault(); });
try {
    const response = await fetch('./model.json', {cache:'no-store'});
    if (!response.ok) throw new Error(String(response.status));
    editor.load(await response.text());
} catch (error) {
    status.textContent = (officeOptions.zh ? '无法打开编辑器。' : 'Could not open the editor. ') + error.message;
}
