(() => {
    let drag = null;
    let suppressClick = false;
    const windowSelector = '.desktop-window.app-explorer:not(.hidden)';
    const number = element => Number(element.textContent);
    const windowId = window => number(window.querySelector('.files-window-id'));
    const tabId = tab => number(tab.querySelector('.files-tab-id'));

    function send(gesture) {
        const root = document.querySelector('.server-desktop:not(.hidden)');
        const input = root?.querySelector('.files-tab-input');
        if (!input || root.closest('[inert]')) return;
        const {epoch} = JSON.parse(root.querySelector('.desktop-motion-state').textContent);
        input.value = JSON.stringify([epoch, gesture]);
        input.dispatchEvent(new Event('input', {bubbles: true}));
    }

    function finish() {
        const moved = drag;
        drag = null;
        moved?.tab.classList.remove('files-tab-dragging');
        return moved;
    }

    document.addEventListener('pointerdown', event => {
        if (event.button !== 0 || !event.isPrimary || event.target.closest('.files-tab-close')) return;
        const tab = event.target.closest('.files-tab');
        const window = tab?.closest(windowSelector);
        if (!window || window.closest('[inert]')) return;
        drag = {tab, window, x: event.clientX, y: event.clientY, pointer: event.pointerId, moved: false};
    }, true);

    document.addEventListener('pointermove', event => {
        if (drag?.pointer !== event.pointerId || drag.moved) return;
        if (Math.hypot(event.clientX - drag.x, event.clientY - drag.y) < 7) return;
        drag.moved = true;
        drag.tab.setPointerCapture(event.pointerId);
        drag.tab.classList.add('files-tab-dragging');
    }, true);

    document.addEventListener('pointercancel', event => {
        if (drag?.pointer === event.pointerId) finish();
    }, true);

    document.addEventListener('pointerup', event => {
        if (drag?.pointer !== event.pointerId) return;
        const moved = finish();
        if (!moved.moved) return;
        suppressClick = true;
        setTimeout(() => { suppressClick = false; }, 0);
        const under = document.elementFromPoint(event.clientX, event.clientY);
        const destination = under?.closest(windowSelector);
        if (destination === moved.window && !under.closest('.files-tabs')) return;
        let before = under?.closest('.files-tab');
        if (before) {
            const bounds = before.getBoundingClientRect();
            if (event.clientX > bounds.left + bounds.width / 2) before = before.nextElementSibling;
        }
        send({
            kind: 'move', window: windowId(moved.window), tab: tabId(moved.tab),
            target: destination ? windowId(destination) : null,
            before: before?.matches('.files-tab') ? tabId(before) : null,
        });
    }, true);

    document.addEventListener('click', event => {
        if (!suppressClick) return;
        event.preventDefault();
        event.stopImmediatePropagation();
    }, true);

    function trashMenu(panel, data) {
        const input = panel.querySelector('.files-trash-input');
        if (!input) return;
        input.value = data ? JSON.stringify(data) : 'null';
        input.dispatchEvent(new Event('input', {bubbles: true}));
    }

    function openTrashMenu(panel, row, x, y) {
        trashMenu(panel, row ? {key: row.querySelector('.trash-item-key').textContent, x, y} : null);
    }

    document.addEventListener('contextmenu', event => {
        const panel = event.target.closest('.app-explorer .trash-panel');
        if (!panel) return;
        event.preventDefault();
        openTrashMenu(panel, event.target.closest('.trash-item'), event.clientX, event.clientY);
    }, true);

    document.addEventListener('pointerdown', event => {
        const panel = document.querySelector('.app-explorer.active .trash-panel:has(.trash-item-menu)');
        if (panel && !event.target.closest('.trash-item-menu')) trashMenu(panel, null);
    }, true);

    function positionTrashMenu() {
        const menu = document.querySelector('.trash-item-menu');
        if (!menu) return;
        const [x, y] = menu.querySelector('.trash-menu-position').textContent.split(',').map(Number);
        const {left, top, right, bottom} = desktopFrame.boundsFor(menu);
        menu.style.left = Math.max(left + 8, Math.min(x, right - menu.offsetWidth - 8)) + 'px';
        menu.style.top = Math.max(top + 8, Math.min(y, bottom - menu.offsetHeight - 8)) + 'px';
    }
    new MutationObserver(positionTrashMenu).observe(document, {subtree: true, childList: true, characterData: true});

    document.addEventListener('keydown', event => {
        const active = document.querySelector(windowSelector + '.active');
        if (!active?.offsetWidth || active.closest('[inert]') || event.altKey || event.repeat) return;
        const menu = active.querySelector('.trash-item-menu');
        if (menu && event.key === 'Escape') {
            event.preventDefault();
            event.stopImmediatePropagation();
            trashMenu(menu.closest('.trash-panel'), null);
            return;
        }
        const row = event.target.closest('.trash-item');
        if (row && (event.key === 'ContextMenu' || (event.key === 'F10' && event.shiftKey))) {
            const bounds = row.getBoundingClientRect();
            event.preventDefault();
            event.stopImmediatePropagation();
            openTrashMenu(row.closest('.trash-panel'), row, Math.round(bounds.left + 12), Math.round(bounds.bottom));
            return;
        }
        if ((!event.ctrlKey && !event.metaKey) || event.shiftKey) return;
        let gesture;
        if (event.key.toLowerCase() === 't') gesture = {kind: 'new', window: windowId(active)};
        if (event.key.toLowerCase() === 'w') {
            const tab = active.querySelector('.files-tab.selected');
            if (tab) gesture = {kind: 'close', window: windowId(active), tab: tabId(tab)};
        }
        if (!gesture) return;
        event.preventDefault();
        event.stopImmediatePropagation();
        send(gesture);
    }, true);
})();
