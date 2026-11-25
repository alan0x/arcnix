import {mountWriter} from './embedded-writer.js';

const status = document.getElementById('message');
const post = value => window.ipc.postMessage(value);
const {remote} = window.officeOptions;

// Supply the local destinations to Syntra's shared menu, toolbar and key dispatcher.
// Both actions flush the same Writer surface before handing the model to the host.
const mountBar = window.SyntraOffice.mount;
let saveAs = false;
let modified = window.officeOptions.dirty;
if (!remote) {
    window.SyntraOffice.mount = options => {
        const saveCopy = options.actions.save_as;
        options.actions.save = {...saveCopy, label: ['Save', '保存'], shortcut: 'Ctrl+S',
            enabled: () => modified && saveCopy.enabled()};
        options.actions.save_as = {...saveCopy, label: ['Save as…', '另存为…'], shortcut: 'Ctrl+Shift+S', run() {
            saveAs = true;
            try { saveCopy.run(); } finally { saveAs = false; }
        }};
        options.groups[0] = ['save', ...options.groups[0]];
        return mountBar(options);
    };
}
const editor = mountWriter({
    root: document.getElementById('commands'), selector:'#body', status, options:window.officeOptions,
    onChange(raw, revision) {
        // The host tab marks unsaved edits. Keep this status line for errors.
        status.replaceChildren();
        modified = true;
        if (raw.startsWith('{"__writer_plain_text":')) {
            post('plain\n' + revision + '\n' + JSON.parse(raw).__writer_plain_text);
        } else post('model\n' + raw);
    },
    onSave: () => post(remote || saveAs ? 'save' : 'confirm_save'),
    onRead: () => post('discard'),
});
if (window.officeOptions.dirty) status.replaceChildren();
window.SyntraOffice.mount = mountBar;
window.officeRestore = editor.restore;
// An edit over the model size limit remains unsaved even when Syntra cannot export it.
document.getElementById('body').addEventListener('input', () => post('dirty'), {once:true});
document.addEventListener('click', e => { if (e.target.closest('a')) e.preventDefault(); });
try {
    const response = await fetch('./model.json', {cache:'no-store'});
    if (!response.ok) throw new Error(String(response.status));
    editor.load(await response.text());
} catch (error) {
    status.textContent = (officeOptions.zh ? '无法打开编辑器。' : 'Could not open the editor. ') + error.message;
}
