import {createWriterSurface, updateWriterSurface, formatWriterSurface} from './writer-surface.js';

const {zh} = window.officeEditor;
const label = (en, cn) => zh ? cn : en;
const post = value => window.ipc.postMessage(value);
const save = document.getElementById('save');
const message = document.getElementById('message');
const discard = document.getElementById('discard');
save.textContent = label('Save as DOCX…', '另存 DOCX…');
discard.textContent = label('Return to reading', '返回阅读');
discard.onclick = () => post('discard');
let revision = 0;
let dirty = window.officeEditor.dirty;
function changed() {
    dirty = true;
    discard.textContent = label('Discard edits', '放弃修改');
    message.textContent = label('Unsaved changes', '有未保存的修改');
}
if (dirty) changed();
let tooLarge = false;
const surface = createWriterSurface('#body', raw => {
    revision++;
    changed();
    // IPC carries only the current model or an entire-body plain-text edit.
    tooLarge = new Blob([raw]).size > window.officeEditor.limit;
    save.disabled = tooLarge;
    if (tooLarge) {
        message.textContent = label('Document exceeds the editing limit.', '文档超过编辑大小限制。');
        return;
    }
    if (raw.startsWith('{"__writer_plain_text":')) {
        post('plain\n' + revision + '\n' + JSON.parse(raw).__writer_plain_text);
    } else {
        post('model\n' + raw);
    }
});
window.officeRestore = (version, model) => {
    if (version === revision) updateWriterSurface(surface, model, true);
};
for (const kind of ['bold', 'italic']) {
    const button = document.getElementById(kind);
    const name = kind === 'bold' ? label('Bold', '粗体') : label('Italic', '斜体');
    button.setAttribute('aria-label', name);
    button.title = name;
    button.onmousedown = e => e.preventDefault();
    button.onclick = () => formatWriterSurface(surface, JSON.stringify({kind, value: !document.queryCommandState(kind)}));
}
save.onclick = () => {
    if (!surface.composing && !tooLarge) {
        surface.emit();
        if (!tooLarge) post('save');
    }
};
document.addEventListener('click', e => { if (e.target.closest('a')) e.preventDefault(); });
addEventListener('pointerdown', () => post('focus'), true);
try {
    const response = await fetch('./model.json', {cache:'no-store'});
    if (!response.ok) throw new Error(String(response.status));
    updateWriterSurface(surface, await response.text(), true);
    save.disabled = false;
} catch (error) {
    message.textContent = label('Could not open the editor. ', '无法打开编辑器。') + error.message;
}
