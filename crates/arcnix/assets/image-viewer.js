// Keep pointer gestures and pixel geometry local to the displayed image.
(() => {
    let stage, picture, scale, angle, fit, x, y, drag;
    const clamp = (value, min, max) => Math.max(min, Math.min(max, value));
    const panel = () => stage?.closest('.image-viewer');
    function render() {
        if (!picture?.naturalWidth || !stage.clientWidth || !stage.clientHeight) return;
        const w = picture.naturalWidth, h = picture.naturalHeight;
        const turned = angle % 180 !== 0;
        const width = turned ? h : w, height = turned ? w : h;
        if (fit) scale = Math.min(1, stage.clientWidth / width, stage.clientHeight / height);
        const dx = Math.max(0, (width * scale - stage.clientWidth) / 2);
        const dy = Math.max(0, (height * scale - stage.clientHeight) / 2);
        x = clamp(x, -dx, dx);
        y = clamp(y, -dy, dy);
        picture.style.width = w + 'px';
        picture.style.height = h + 'px';
        picture.style.transform = `translate(-50%, -50%) translate(${x}px, ${y}px) rotate(${angle}deg) scale(${scale})`;
        stage.classList.toggle('can-pan', dx > 0 || dy > 0);
        panel().querySelector('.viewer-scale').textContent = Math.round(scale * 100) + '%';
        panel().querySelector('.viewer-dimensions').textContent = w + ' × ' + h;
        panel().querySelector('.viewer-fit').setAttribute('aria-pressed', String(fit));
    }
    const resize = new ResizeObserver(render);
    function sync() {
        const nextStage = document.querySelector('.image-viewer .viewer-stage');
        const nextPicture = nextStage?.querySelector('img');
        if (stage === nextStage && picture === nextPicture) return;
        resize.disconnect();
        picture?.removeEventListener('load', render);
        stage = nextStage;
        picture = nextPicture;
        drag = null;
        if (!stage) return;
        scale = 1;
        angle = x = y = 0;
        fit = true;
        stage.classList.remove('can-pan', 'panning');
        for (const label of panel().querySelectorAll('.viewer-scale, .viewer-dimensions')) label.textContent = '';
        panel().querySelector('.viewer-fit')?.setAttribute('aria-pressed', 'false');
        if (picture) {
            picture.draggable = false;
            picture.addEventListener('load', render);
            resize.observe(stage);
            render();
        }
    }
    new MutationObserver(sync).observe(document, {childList: true, subtree: true});
    function zoom(value, px = 0, py = 0) {
        const next = clamp(value, Math.min(0.05, scale), 16);
        x = px - (px - x) * next / scale;
        y = py - (py - y) * next / scale;
        scale = next;
        fit = false;
        render();
    }
    document.addEventListener('click', event => {
        const button = event.target.closest?.('.image-viewer button');
        if (!button || button.disabled || !panel()?.contains(button)) return;
        if (!picture?.naturalWidth) return;
        if (button.matches('.viewer-in')) zoom(scale * 1.25);
        if (button.matches('.viewer-out')) zoom(scale / 1.25);
        if (button.matches('.viewer-fit')) { fit = true; render(); }
        if (button.matches('.viewer-actual')) zoom(1);
        if (button.matches('.viewer-left, .viewer-right')) {
            angle = (angle + (button.matches('.viewer-right') ? 90 : 270)) % 360;
            x = y = 0;
            render();
        }
    }, true);
    document.addEventListener('wheel', event => {
        if (!picture || !stage.contains(event.target) || !event.deltaY) return;
        event.preventDefault();
        const box = stage.getBoundingClientRect();
        zoom(scale * (event.deltaY < 0 ? 1.25 : 0.8),
            (event.clientX - box.left - box.width / 2) * stage.clientWidth / box.width,
            (event.clientY - box.top - box.height / 2) * stage.clientHeight / box.height);
    }, {passive: false});
    document.addEventListener('dblclick', event => {
        if (!picture || !stage.contains(event.target)) return;
        if (fit) zoom(1);
        else { fit = true; render(); }
    });
    document.addEventListener('pointerdown', event => {
        if (event.button !== 0 || !stage?.classList.contains('can-pan') || !stage.contains(event.target)) return;
        const box = stage.getBoundingClientRect();
        drag = {id: event.pointerId, startX: event.clientX, startY: event.clientY, x, y,
            ratioX: stage.clientWidth / box.width, ratioY: stage.clientHeight / box.height};
        stage.setPointerCapture(event.pointerId);
        stage.classList.add('panning');
        event.preventDefault();
    });
    document.addEventListener('pointermove', event => {
        if (drag?.id !== event.pointerId) return;
        x = drag.x + (event.clientX - drag.startX) * drag.ratioX;
        y = drag.y + (event.clientY - drag.startY) * drag.ratioY;
        render();
    });
    document.addEventListener('lostpointercapture', event => {
        if (drag?.id === event.pointerId) {
            drag = null;
            stage.classList.remove('panning');
        }
    });
    document.addEventListener('keydown', event => {
        const viewer = panel();
        if (!viewer?.closest('.desktop-window.active:not(.hidden)') ||
            !viewer.getClientRects().length || event.defaultPrevented || event.isComposing ||
            event.target.closest?.('input, textarea, select, [contenteditable], [role="dialog"]') ||
            document.querySelector('[role="dialog"]:not(.hidden)') || event.metaKey || event.ctrlKey || event.altKey) return;
        const key = {ArrowLeft: 'previous', ArrowRight: 'next', '+': 'in', '=': 'in', '-': 'out',
            '0': 'fit', '1': 'actual', r: 'right', R: 'left', f: 'fullscreen'}[event.key];
        if (key) {
            event.preventDefault();
            viewer.querySelector('.viewer-' + key)?.click();
        }
    });
})();
