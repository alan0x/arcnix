// Measure the live reading area so fitting follows window, dock, and Desktop changes.
(() => {
    let panel, stage, sheet, documentId, mode = 'page', scale = 1;
    function layout() {
        if (!sheet || !stage.clientWidth || !stage.clientHeight) return;
        const image = sheet.querySelector('img');
        if (!image?.naturalWidth) return;
        const base = Number(sheet.querySelector('.pdf-page-width').textContent);
        const height = base * image.naturalHeight / image.naturalWidth;
        const style = getComputedStyle(stage);
        const widthSpace = stage.clientWidth - parseFloat(style.paddingLeft) - parseFloat(style.paddingRight);
        const heightSpace = stage.clientHeight - parseFloat(style.paddingTop) - parseFloat(style.paddingBottom);
        if (mode === 'width') scale = widthSpace / base;
        if (mode === 'page') scale = Math.min(widthSpace / base, heightSpace / height);
        const width = Math.max(1, Math.floor(base * scale)) + 'px';
        if (sheet.style.width !== width) sheet.style.width = width;
        const label = Math.round(scale * 100) + '%';
        const status = panel.querySelector('.pdf-scale');
        if (status.textContent !== label) status.textContent = label;
        panel.querySelector('.pdf-fit').setAttribute('aria-pressed', String(mode === 'page'));
        panel.querySelector('.pdf-width').setAttribute('aria-pressed', String(mode === 'width'));
    }
    let frame;
    function scheduleLayout() {
        if (frame) return;
        frame = requestAnimationFrame(() => { frame = 0; layout(); });
    }
    const resize = new ResizeObserver(scheduleLayout);
    function sync() {
        const next = document.querySelector('.pdf-reader');
        const nextId = next?.querySelector('.pdf-document-id')?.textContent;
        const nextSheet = next?.querySelector('.pdf-sheet');
        if (panel === next && documentId === nextId && sheet === nextSheet) return;
        sheet?.querySelector('img')?.removeEventListener('load', layout);
        if (panel !== next || documentId !== nextId) { mode = 'page'; scale = 1; }
        resize.disconnect();
        panel = next;
        documentId = nextId;
        sheet = nextSheet;
        stage = panel?.querySelector('.pdf-stage');
        if (!stage) return;
        stage.scrollTop = stage.scrollLeft = 0;
        if (sheet) {
            sheet.querySelector('img')?.addEventListener('load', layout);
            resize.observe(stage);
            layout();
        } else {
            panel.querySelector('.pdf-scale').textContent = '';
        }
    }
    new MutationObserver(sync).observe(document, {childList: true, subtree: true, characterData: true});
    function zoom(factor) {
        mode = 'scale';
        scale = Math.min(4, Math.max(0.1, scale * factor));
        layout();
    }
    document.addEventListener('click', e => {
        const button = e.target.closest?.('.pdf-reader button');
        if (!button || button.disabled || !sheet) return;
        if (button.matches('.viewer-in')) zoom(1.25);
        if (button.matches('.viewer-out')) zoom(0.8);
        if (button.matches('.pdf-fit, .pdf-width')) {
            mode = button.matches('.pdf-fit') ? 'page' : 'width';
            layout();
        }
    });
    document.addEventListener('wheel', e => {
        if (!e.ctrlKey || !sheet || !stage.contains(e.target) || !e.deltaY) return;
        e.preventDefault();
        zoom(e.deltaY < 0 ? 1.25 : 0.8);
    }, {passive: false});
    document.addEventListener('keydown', e => {
        if (!panel?.closest('.desktop-window.active:not(.hidden)') || !panel.getClientRects().length ||
            e.defaultPrevented || e.isComposing || e.metaKey || e.ctrlKey || e.altKey ||
            e.target.closest?.('input, textarea, select, [contenteditable], [role="dialog"]') ||
            document.querySelector('[role="dialog"]:not(.hidden)')) return;
        const action = {ArrowLeft: 'viewer-previous', PageUp: 'viewer-previous',
            ArrowRight: 'viewer-next', PageDown: 'viewer-next', '+': 'viewer-in', '=': 'viewer-in',
            '-': 'viewer-out', '0': 'pdf-fit', w: 'pdf-width', f: 'viewer-fullscreen'}[e.key];
        if (action) {
            e.preventDefault();
            panel.querySelector('.' + action)?.click();
        }
    });
})();
