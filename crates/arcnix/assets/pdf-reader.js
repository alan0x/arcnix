// Measure the live reading area so fitting follows window, dock, and Desktop changes.
(() => {
    function reader(panel) {
        let stage, sheet, documentId, savedZoom, mode = 'page', scale = 1;
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
            const label = Number((scale * 100).toFixed(1)) + '%';
            const field = panel.querySelector('.pdf-zoom-input');
            if (field !== document.activeElement) field.value = label;
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
            const nextId = panel.querySelector('.pdf-document-id')?.textContent;
            const nextSheet = panel.querySelector('.pdf-sheet');
            const nextZoom = panel.querySelector('.pdf-zoom-mode')?.textContent || 'page';
            const newDocument = documentId !== nextId;
            const newSheet = sheet !== nextSheet;
            if (!newDocument && !newSheet && savedZoom === nextZoom) return;
            savedZoom = nextZoom;
            mode = nextZoom === 'page' || nextZoom === 'width' ? nextZoom : 'scale';
            if (mode === 'scale') scale = Number(nextZoom) / 100;
            if (!newDocument && !newSheet) { layout(); return; }
            sheet?.querySelector('img')?.removeEventListener('load', layout);
            resize.disconnect();
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
                panel.querySelector('.pdf-zoom-input').value = '';
            }
        }
        function remember() {
            const bridge = panel.querySelector('.pdf-zoom-command');
            bridge.value = documentId + ':' + (mode === 'scale' ? Number((scale * 100).toFixed(3)) : mode);
            bridge.dispatchEvent(new Event('input', {bubbles: true}));
        }
        // Apply a typed percentage on Enter or when leaving the field.
        function applyPercentage(field) {
            const match = /^(\d+(?:\.\d+)?|\.\d+)\s*%?$/.exec(field.value.trim());
            const percent = match ? Number(match[1]) : NaN;
            if (Number.isFinite(percent) && percent >= 10 && percent <= 400) {
                mode = 'scale';
                scale = percent / 100;
                remember();
            }
            field.value = Number((scale * 100).toFixed(1)) + '%';
            layout();
        }
        panel.addEventListener('change', e => {
            if (sheet && e.target.matches('.pdf-zoom-input')) applyPercentage(e.target);
        });
        panel.addEventListener('keydown', e => {
            if (sheet && e.key === 'Enter' && e.target.matches('.pdf-zoom-input')) {
                e.preventDefault();
                applyPercentage(e.target);
            }
        });
        function zoom(factor) {
            mode = 'scale';
            scale = Math.min(4, Math.max(0.1, scale * factor));
            remember();
            layout();
        }
        panel.addEventListener('click', e => {
            const button = e.target.closest?.('.pdf-reader button');
            if (!button || button.disabled || !sheet) return;
            if (button.matches('.viewer-in')) zoom(1.25);
            if (button.matches('.viewer-out')) zoom(0.8);
            if (button.matches('.pdf-fit, .pdf-width')) {
                mode = button.matches('.pdf-fit') ? 'page' : 'width';
                remember();
                layout();
            }
        });
        panel.addEventListener('wheel', e => {
            if (!e.ctrlKey || !sheet || !stage.contains(e.target) || !e.deltaY) return;
            e.preventDefault();
            zoom(e.deltaY < 0 ? 1.25 : 0.8);
        }, {passive: false});
        return {sync, dispose() {
            resize.disconnect();
            sheet?.querySelector('img')?.removeEventListener('load', layout);
            cancelAnimationFrame(frame);
        }};
    }
    const readers = new Map();
    function syncReaders() {
        for (const [panel, controller] of readers) {
            if (!panel.isConnected) { controller.dispose(); readers.delete(panel); }
        }
        for (const panel of document.querySelectorAll('.pdf-reader')) {
            if (!panel.querySelector('.pdf-zoom-input')) continue;
            if (!readers.has(panel)) readers.set(panel, reader(panel));
            readers.get(panel).sync();
        }
    }
    new MutationObserver(syncReaders).observe(document, {childList: true, subtree: true, characterData: true});
    syncReaders();
    document.addEventListener('keydown', e => {
        const panel = document.querySelector('.desktop-window.active:not(.hidden) .pdf-reader');
        if (!panel?.getClientRects().length ||
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
