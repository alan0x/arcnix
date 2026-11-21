(() => {
    document.addEventListener('keydown', event => {
        if (!(event.ctrlKey || event.metaKey) || event.altKey || event.key.toLowerCase() !== 's') return;
        const editor = event.target.closest?.('.remote-editor-modal, .desktop-window.app-editor');
        if (!editor) return;
        event.preventDefault();
        event.stopPropagation();
        const save = editor.querySelector('.remote-editor-save');
        if (save && !save.disabled) save.click();
    }, true);

    const rendered = new WeakMap();
    function renderPreviews() {
        for (const host of document.querySelectorAll('.editor-preview-host')) {
            const preview = host.querySelector('.markdown-preview');
            const html = host.querySelector('.markdown-preview-source')?.textContent;
            if (!preview || html === undefined || rendered.get(preview) === html) continue;
            rendered.set(preview, html);
            // Rust emits Markdown markup with raw HTML, links and images escaped or stripped.
            const template = document.createElement('template');
            template.innerHTML = html;
            preview.replaceChildren(template.content);
        }
    }
    new MutationObserver(renderPreviews).observe(document, {childList: true, characterData: true, subtree: true});
    renderPreviews();
})();
