// Each dialog opens centered. Only a titlebar drag gives its backdrop a local position.
(() => {
    const controls = 'button, a, input, textarea, select, label, [contenteditable], [role="button"], [role="tab"]';
    let drag;

    document.addEventListener('pointerdown', event => {
        const header = event.target.closest?.('.modal-header');
        if (!header || drag || !event.isPrimary || event.button !== 0 || event.target.closest(controls)) return;
        const card = header.closest('.modal-card');
        const host = card.parentElement;
        const box = card.getBoundingClientRect();
        const frame = host.getBoundingClientRect();
        const style = getComputedStyle(host);
        drag = {
            header, host, pointer: event.pointerId, x: event.clientX, y: event.clientY,
            left: box.left - frame.left - parseFloat(style.paddingLeft),
            top: box.top - frame.top - parseFloat(style.paddingTop),
            previousX: host.style.getPropertyValue('--modal-x'),
            previousY: host.style.getPropertyValue('--modal-y'),
        };
        event.preventDefault();
        header.setPointerCapture(event.pointerId);
    }, true);

    document.addEventListener('pointermove', event => {
        if (event.pointerId !== drag?.pointer) return;
        if (!event.buttons) { finish(true); return; }
        const dx = event.clientX - drag.x;
        const dy = event.clientY - drag.y;
        if (!drag.moved && Math.max(Math.abs(dx), Math.abs(dy)) < 4) return;
        drag.moved = true;
        drag.header.setAttribute('data-dragging', '');
        drag.host.style.setProperty('--modal-x', (drag.left + dx) + 'px');
        drag.host.style.setProperty('--modal-y', (drag.top + dy) + 'px');
        drag.host.setAttribute('data-modal-moved', '');
    });

    function finish(cancel) {
        if (!drag) return;
        const { header, host, pointer, moved, previousX, previousY } = drag;
        drag = null;
        if (cancel && moved) {
            host.style.setProperty('--modal-x', previousX);
            host.style.setProperty('--modal-y', previousY);
            host.toggleAttribute('data-modal-moved', Boolean(previousX));
        }
        header.removeAttribute('data-dragging');
        if (header.hasPointerCapture(pointer)) header.releasePointerCapture(pointer);
    }

    document.addEventListener('pointerup', event => { if (event.pointerId === drag?.pointer) finish(false); });
    for (const name of ['pointercancel', 'lostpointercapture']) {
        document.addEventListener(name, event => { if (event.pointerId === drag?.pointer) finish(true); });
    }
    window.addEventListener('blur', () => finish(true));
    window.addEventListener('keydown', event => {
        if (event.key !== 'Escape' || !drag?.moved) return;
        finish(true);
        event.preventDefault();
        event.stopImmediatePropagation();
    }, true);
})();
