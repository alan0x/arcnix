(() => {
    document.addEventListener('keydown', event => {
        if (!(event.ctrlKey || event.metaKey) || event.altKey || event.key.toLowerCase() !== 's') return;
        const editor = event.target.closest?.('.remote-editor-modal, .desktop-editor');
        if (!editor) return;
        event.preventDefault();
        event.stopPropagation();
        const save = editor.querySelector('.remote-editor-save');
        if (save && !save.disabled) save.click();
    }, true);

    document.addEventListener('wheel', event => {
        const strip = event.target.closest?.('.editor-tab-strip');
        if (!strip || event.ctrlKey || event.deltaX || !event.deltaY || strip.scrollWidth <= strip.clientWidth) return;
        const scale = event.deltaMode === 1 ? 20 : event.deltaMode === 2 ? strip.clientWidth : 1;
        strip.scrollLeft += event.deltaY * scale;
        event.preventDefault();
    }, {passive: false});

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
    let lastTab;
    function sync() {
        renderPreviews();
        const active = document.querySelector('.editor-tab-item.active');
        if (!active || active === lastTab) { lastTab = active; return; }
        lastTab = active;
        const strip = active.closest('.editor-tab-strip');
        const viewport = strip.getBoundingClientRect();
        const tab = active.getBoundingClientRect();
        if (tab.left < viewport.left) strip.scrollLeft += tab.left - viewport.left;
        else if (tab.right > viewport.right) strip.scrollLeft += tab.right - viewport.right;
    }
    new MutationObserver(sync).observe(document, {childList: true, characterData: true, attributes: true, attributeFilter: ['class'], subtree: true});
    renderPreviews();
})();
