// Preview the sidebar width during the drag and persist it on release.
(() => {
    const handles = '.files-sidebar-splitter, .server-sidebar-splitter';
    let drag;
    function geometry(handle) {
        const body = handle.parentElement;
        const server = handle.classList.contains('server-sidebar-splitter');
        const frame = body.querySelector(server ? '.server-explorer' : '.files-sidebar-frame');
        const input = body.querySelector(server ? '.server-sidebar-width-input' : '.files-sidebar-width-input');
        const scale = body.getBoundingClientRect().width / body.offsetWidth || 1;
        return {
            frame, input, scale, width: frame.getBoundingClientRect().width / scale,
            min: server ? 176 : 104,
            max: Math.min(server ? 480 : 420, Math.floor(body.clientWidth * (server ? 0.45 : 0.48))),
        };
    }
    function bounded(value, {min, max}) {
        return Math.max(min, Math.min(max, Math.round(value)));
    }
    function commit(input, width) {
        input.value = String(width);
        input.dispatchEvent(new Event('input', {bubbles: true}));
    }
    document.addEventListener('pointerdown', event => {
        const handle = event.target.closest?.(handles);
        if (!handle || event.button !== 0 || drag) return;
        event.preventDefault();
        handle.focus({preventScroll: true});
        handle.setPointerCapture(event.pointerId);
        drag = {...geometry(handle), id: event.pointerId, x: event.clientX};
    });
    function update(event, end) {
        if (!drag || event.pointerId !== drag.id) return;
        const width = bounded(drag.width + (event.clientX - drag.x) / drag.scale, drag);
        if (end) {
            if (Math.round(drag.width) !== width) commit(drag.input, width);
            clear(event);
        } else {
            drag.frame.style.setProperty('--sidebar-preview', width + 'px');
        }
    }
    function clear(event) {
        if (!drag || event.pointerId !== drag.id) return;
        drag.frame.style.removeProperty('--sidebar-preview');
        drag = undefined;
    }
    document.addEventListener('pointermove', event => update(event, false));
    document.addEventListener('pointerup', event => update(event, true));
    document.addEventListener('pointercancel', clear);
    document.addEventListener('lostpointercapture', clear);
    document.addEventListener('keydown', event => {
        const handle = event.target.closest?.(handles);
        if (!handle || !['ArrowLeft','ArrowRight'].includes(event.key)) return;
        const sidebar = geometry(handle);
        event.preventDefault();
        event.stopPropagation();
        commit(sidebar.input, bounded(sidebar.width + (event.key === 'ArrowLeft' ? -8 : 8), sidebar));
    }, true);
})();
