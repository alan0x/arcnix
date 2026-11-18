(() => {
    const dragType = 'application/x-arcnix-desktop-icon';
    let drag, marker, focusedMenu;

    function canvasFor(target) {
        if (!(target instanceof Element) || target.closest('[inert],[hidden],.hidden')) return null;
        return target.closest('.desktop-canvas');
    }

    function epoch(canvas) {
        return JSON.parse(canvas.closest('.server-desktop').querySelector('.desktop-motion-state').textContent).epoch;
    }

    function send(canvas, payload) {
        const bridge = canvas.closest('.server-desktop')?.querySelector('.desktop-customization-input');
        if (!bridge) return;
        bridge.value = JSON.stringify({epoch: epoch(canvas), ...payload});
        bridge.dispatchEvent(new Event('input', {bubbles: true}));
    }

    function zone(target) {
        return target.closest('.desktop-dock') ? 'dock' : 'desktop';
    }

    function shortcut(slot) {
        const key = slot?.querySelector('.desktop-item-key');
        return key ? JSON.parse(key.textContent) : null;
    }

    function clearMarker() {
        marker?.removeAttribute('data-desktop-drop');
        marker = null;
    }

    function clearDrag() {
        clearMarker();
        drag?.slot.removeAttribute('data-desktop-dragging');
        drag = null;
    }

    function dropTarget(event) {
        const canvas = canvasFor(event.target);
        const slot = event.target.closest?.('.desktop-icon-slot');
        if (canvas !== drag.canvas || epoch(canvas) !== drag.epoch || !slot || slot === drag.slot || zone(slot) !== drag.zone) return null;
        const rect = slot.getBoundingClientRect();
        const after = drag.zone === 'desktop'
            ? event.clientY >= rect.top + rect.height / 2
            : event.clientX >= rect.left + rect.width / 2;
        return {slot, after};
    }

    document.addEventListener('dragstart', event => {
        const slot = event.target.closest?.('.desktop-icon-slot');
        const canvas = canvasFor(slot);
        if (!canvas || !event.dataTransfer) return;
        clearDrag();
        drag = {slot, canvas, epoch: epoch(canvas), zone: zone(slot), item: shortcut(slot)};
        event.dataTransfer.effectAllowed = 'move';
        event.dataTransfer.setData(dragType, String(drag.epoch));
        slot.setAttribute('data-desktop-dragging', '');
    }, true);

    document.addEventListener('dragover', event => {
        if (!drag) return;
        clearMarker();
        event.preventDefault();
        const target = dropTarget(event);
        event.dataTransfer.dropEffect = target ? 'move' : 'none';
        if (!target) return;
        marker = target.slot;
        marker.setAttribute('data-desktop-drop', target.after ? 'after' : 'before');
    }, true);

    document.addEventListener('drop', event => {
        if (!drag) return;
        event.preventDefault();
        const target = dropTarget(event);
        if (target && event.dataTransfer.getData(dragType) === String(drag.epoch)) {
            send(drag.canvas, {kind: 'reorder', zone: drag.zone, item: drag.item, anchor: shortcut(target.slot), after: target.after});
        }
        clearDrag();
    }, true);
    document.addEventListener('dragend', clearDrag, true);
    window.addEventListener('blur', clearDrag);

    document.addEventListener('wheel', event => {
        const grid = event.target.closest?.('.desktop-shortcuts');
        if (!grid || event.ctrlKey || event.deltaX || !event.deltaY || grid.scrollWidth <= grid.clientWidth) return;
        const scale = event.deltaMode === 1 ? 20 : event.deltaMode === 2 ? grid.clientWidth : 1;
        grid.scrollLeft += event.deltaY * scale;
        event.preventDefault();
    }, {passive: false});

    document.addEventListener('contextmenu', event => {
        const canvas = canvasFor(event.target);
        if (!canvas || event.target.closest('.desktop-window,.desktop-context-menu')) return;
        event.preventDefault();
        const rect = canvas.getBoundingClientRect();
        const target = event.target.closest('.desktop-menu-backdrop')
            ? document.elementsFromPoint(event.clientX,event.clientY).find(element => element !== event.target && canvas.contains(element))
            : event.target;
        if (!target || target.closest('.desktop-window')) { send(canvas, {kind: 'dismiss'}); return; }
        send(canvas, {kind: 'context', zone: zone(target), item: shortcut(target.closest('.desktop-icon-slot')),
            x: Math.round(event.clientX - rect.left), y: Math.round(event.clientY - rect.top)});
    }, true);

    document.addEventListener('keydown', event => {
        const menu = document.querySelector('.server-desktop:not(.hidden) .desktop-context-menu');
        if (!menu) return;
        if (event.key === 'Escape') {
            send(menu.closest('.desktop-canvas'), {kind: 'dismiss'});
        } else if (['ArrowUp', 'ArrowDown', 'Home', 'End'].includes(event.key) && menu.contains(event.target)) {
            const buttons = [...menu.querySelectorAll('button:not(:disabled)')];
            if (!buttons.length) return;
            const index = buttons.indexOf(document.activeElement);
            const next = event.key === 'Home' ? 0 : event.key === 'End' ? buttons.length - 1
                : (index + (event.key === 'ArrowDown' ? 1 : -1) + buttons.length) % buttons.length;
            buttons[next].focus();
        } else return;
        event.preventDefault();
        event.stopImmediatePropagation();
    }, true);

    function update() {
        for (const slot of document.querySelectorAll('.desktop-icon-slot:not([draggable])')) slot.draggable = true;
        const menu = document.querySelector('.desktop-context-menu');
        const canvas = menu && canvasFor(menu);
        if (!canvas) { focusedMenu = null; return; }
        const [x, y] = menu.querySelector('.desktop-menu-position').textContent.split(',').map(Number);
        menu.style.left = `${Math.max(4, Math.min(x, canvas.clientWidth - menu.offsetWidth - 4))}px`;
        menu.style.top = `${Math.max(4, Math.min(y, canvas.clientHeight - menu.offsetHeight - 4))}px`;
        if (focusedMenu !== menu) {
            focusedMenu = menu;
            menu.querySelector('button')?.focus({preventScroll: true});
        }
    }

    new MutationObserver(update).observe(document, {subtree: true, childList: true, characterData: true, attributes: true, attributeFilter: ['class']});
    window.addEventListener('resize', update);
})();
