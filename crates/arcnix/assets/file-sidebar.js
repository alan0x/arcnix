// Preview the divider in the owning window. Persist one CSS-pixel width on release.
(() => {
    let drag;
    function geometry(handle) {
        const body = handle.parentElement;
        const frame = body.querySelector('.files-sidebar-frame');
        const scale = body.getBoundingClientRect().width / body.offsetWidth || 1;
        return {body, scale, width: frame.getBoundingClientRect().width / scale,
            max: Math.min(420, Math.floor(body.clientWidth * 0.48))};
    }
    function bounded(value, max) {
        return Math.max(104, Math.min(max, Math.round(value)));
    }
    function commit(body, width) {
        const input = body.querySelector('.files-sidebar-width-input');
        input.value = String(width);
        input.dispatchEvent(new Event('input', {bubbles: true}));
    }
    document.addEventListener('pointerdown', event => {
        const handle = event.target.closest?.('.files-sidebar-splitter');
        if (!handle || event.button !== 0 || drag) return;
        event.preventDefault();
        handle.focus({preventScroll: true});
        handle.setPointerCapture(event.pointerId);
        drag = {...geometry(handle), id: event.pointerId, x: event.clientX};
    });
    function update(event, end) {
        if (!drag || event.pointerId !== drag.id) return;
        const width = bounded(drag.width + (event.clientX - drag.x) / drag.scale, drag.max);
        if (end) {
            if (Math.round(drag.width) !== width) commit(drag.body, width);
            clear(event);
        } else {
            drag.body.style.setProperty('--sidebar-preview', width + 'px');
        }
    }
    function clear(event) {
        if (!drag || event.pointerId !== drag.id) return;
        drag.body.style.removeProperty('--sidebar-preview');
        drag = undefined;
    }
    document.addEventListener('pointermove', event => update(event, false));
    document.addEventListener('pointerup', event => update(event, true));
    document.addEventListener('pointercancel', clear);
    document.addEventListener('lostpointercapture', clear);
    document.addEventListener('keydown', event => {
        const handle = event.target.closest?.('.files-sidebar-splitter');
        if (!handle || !['ArrowLeft','ArrowRight'].includes(event.key)) return;
        const {body, width, max} = geometry(handle);
        event.preventDefault();
        event.stopPropagation();
        commit(body, bounded(width + (event.key === 'ArrowLeft' ? -8 : 8), max));
    }, true);
})();
