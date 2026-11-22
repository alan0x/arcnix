(() => {
    const activeTabs = new WeakMap();
    let observed = new Set();
    let frame;
    const resize = new ResizeObserver(schedule);

    // All detachable tab strips use the same pointer session and drop hit test.
    let drag;
    let suppressClick = false;

    function destination(event) {
        const hit = document.elementFromPoint(event.clientX, event.clientY);
        if (!hit || hit.closest('.desktop-canvas') !== drag.canvas || hit.closest('[inert], [popover]:popover-open, .desktop-dock-zone')) return;
        const win = hit.closest('.desktop-window');
        if (!win) return {target: null};
        const item = drag.layout.windows.find(item => win.classList.contains(item.class));
        if (!item || item.id !== drag.app || !win.matches('.window-tabs-movable')) return;
        const hovered = hit.closest('.window-tab-strip');
        if (win === drag.source && !hovered) return;
        const strip = hovered || win.querySelector('.window-tab-strip');
        if (!strip) return;
        // Use layout positions so animating neighbours cannot change the insertion point.
        const rect = strip.getBoundingClientRect();
        const scale = strip.offsetWidth ? rect.width / strip.offsetWidth : 1;
        const x = (event.clientX - rect.left) / scale + strip.scrollLeft;
        const before = hovered && Array.from(strip.children).find(tab =>
            tab !== drag.tab && tab.matches('.window-tab-item') && x < tab.offsetLeft + tab.offsetWidth / 2);
        return {strip, hovered, target: item.window, before: before || null};
    }

    function showSlot(drop) {
        const slot = drag.slot;
        if (!drop?.strip) {
            slot.remove();
            return;
        }
        const {strip, before} = drop;
        if (slot.parentElement === strip && slot.nextElementSibling === before) return;
        const neighbours = Array.from(strip.children).filter(tab => tab !== drag.tab && tab.matches('.window-tab-item'));
        const positions = neighbours.map(tab => tab.offsetLeft);
        strip.insertBefore(slot, before);
        if (!matchMedia('(prefers-reduced-motion: reduce)').matches) {
            neighbours.forEach((tab, i) => {
                const delta = positions[i] - tab.offsetLeft;
                if (delta) tab.animate([{translate: delta + 'px'}, {translate: '0px'}],
                    {duration: 130, easing: 'ease-out'});
            });
        }
        if (!drop.hovered) strip.scrollLeft = strip.scrollWidth;
    }

    function tabId(item) {
        const id = item?.querySelector('.window-tab-id');
        return id ? Number(id.textContent) : null;
    }

    function finish() {
        if (!drag) return;
        const session = drag;
        drag = null;
        session.slot?.remove();
        session.tab.removeAttribute('data-tab-dragging');
        session.preview?.remove();
        if (session.canvas.hasPointerCapture(session.pointer)) session.canvas.releasePointerCapture(session.pointer);
    }

    document.addEventListener('pointerdown', event => {
        if (!event.isPrimary || event.button !== 0) return;
        suppressClick = false;
        finish();
        const button = event.target.closest?.('.window-tab-select');
        const tab = button?.closest('.window-tab-item');
        const source = tab?.closest('.desktop-window');
        if (!source?.matches('.window-tabs-movable') || source.closest('.hidden,[inert]')) return;
        const canvas = source.closest('.desktop-canvas');
        const layout = JSON.parse(canvas.closest('.server-desktop').querySelector('.desktop-motion-state').textContent);
        const item = layout.windows.find(item => source.classList.contains(item.class));
        if (!item || tabId(tab) === null) return;
        const box = tab.getBoundingClientRect();
        // Measure at press time, before a background window can move to the front.
        drag = {pointer: event.pointerId, tab, button, canvas, source, layout, app: item.id, window: item.window,
            box, x: event.clientX, y: event.clientY};
    }, true);

    document.addEventListener('pointermove', event => {
        if (!drag || drag.pointer !== event.pointerId) return;
        if (!drag.tab.isConnected || drag.source.closest('.hidden,[inert]') || !event.buttons) { finish(); return; }
        const dx = event.clientX - drag.x, dy = event.clientY - drag.y;
        if (!drag.preview) {
            if (Math.hypot(dx, dy) < 6) return;
            drag.canvas.setPointerCapture(event.pointerId);
            const preview = document.createElement('div');
            preview.className = 'window-tab-preview';
            preview.setAttribute('aria-hidden', 'true');
            const style = getComputedStyle(drag.tab);
            const label = drag.button.getBoundingClientRect();
            const buttonStyle = getComputedStyle(drag.button);
            preview.textContent = drag.button.textContent;
            Object.assign(preview.style, {
                width: drag.box.width + 'px', height: drag.box.height + 'px',
                paddingLeft: (label.left - drag.tab.getBoundingClientRect().left + parseFloat(buttonStyle.paddingLeft)) + 'px',
                font: buttonStyle.font, color: style.getPropertyValue('--text'),
                background: style.getPropertyValue('--accent-bg'), borderColor: style.getPropertyValue('--accent'),
            });
            document.body.appendChild(preview);
            drag.preview = preview;
            const slot = document.createElement('span');
            slot.className = 'window-tab-slot';
            slot.setAttribute('aria-hidden', 'true');
            slot.style.width = drag.tab.offsetWidth + 'px';
            slot.style.height = drag.tab.offsetHeight + 'px';
            drag.slot = slot;
            drag.tab.setAttribute('data-tab-dragging', '');
            suppressClick = true;
        }
        event.preventDefault();
        drag.preview.style.transform = 'translate(' + (drag.box.left + dx) + 'px,' + (drag.box.top + dy) + 'px)';
        let drop = destination(event);
        const strip = drop?.hovered;
        if (strip && strip.scrollWidth > strip.clientWidth) {
            const box = strip.getBoundingClientRect();
            const previous = strip.scrollLeft;
            if (event.clientX < box.left + 24) strip.scrollLeft -= 10;
            else if (event.clientX > box.right - 24) strip.scrollLeft += 10;
            if (previous !== strip.scrollLeft) drop = destination(event);
        }
        showSlot(drop);
        drag.lastX = event.clientX;
        drag.lastY = event.clientY;
    }, true);

    document.addEventListener('pointerup', event => {
        if (!drag || drag.pointer !== event.pointerId) return;
        const drop = drag.preview && destination(event);
        if (drop) {
            // Flex layout may settle after the last move. Release into the slot the user sees.
            const before = drop.strip && drag.slot.parentElement === drop.strip &&
                drag.lastX === event.clientX && drag.lastY === event.clientY
                ? drag.slot.nextElementSibling : drop.before;
            const input = drag.canvas.closest('.server-desktop').querySelector('.window-tab-move');
            input.value = JSON.stringify({epoch: drag.layout.epoch, app: drag.app, source: drag.window,
                tab: tabId(drag.tab), target: drop.target, before: tabId(before)});
            input.dispatchEvent(new Event('input', {bubbles: true}));
        }
        finish();
    }, true);
    for (const name of ['pointercancel', 'lostpointercapture']) {
        document.addEventListener(name, event => {
            if (drag?.pointer === event.pointerId && (name === 'pointercancel' || event.target === drag.canvas)) finish();
        }, true);
    }
    window.addEventListener('blur', finish);
    window.addEventListener('keydown', event => { if (event.key === 'Escape') finish(); }, true);
    document.addEventListener('click', event => {
        if (!suppressClick || !event.detail) return;
        suppressClick = false;
        event.preventDefault();
        event.stopImmediatePropagation();
    }, true);

    function positionMenu(menu, trigger) {
        const anchor = trigger.getBoundingClientRect();
        const layer = trigger.closest('.desktop-window-layer').getBoundingClientRect();
        const left = Math.max(0, layer.left) + 6;
        const right = Math.min(innerWidth, layer.right) - 6;
        const top = Math.max(0, layer.top) + 6;
        const bottom = Math.min(innerHeight, layer.bottom) - 6;
        const below = Math.max(0, bottom - anchor.bottom - 4);
        const above = Math.max(0, anchor.top - top - 4);
        const openBelow = below >= Math.min(280, menu.scrollHeight) || below >= above;
        const width = Math.max(0, Math.min(288, right - left));
        Object.assign(menu.style, {
            left: `${Math.max(left, Math.min(anchor.right - width, right - width))}px`,
            top: openBelow ? `${Math.max(top, anchor.bottom + 4)}px` : 'auto',
            bottom: openBelow ? 'auto' : `${innerHeight - Math.min(bottom, anchor.top - 4)}px`,
            width: `${width}px`,
            maxHeight: `${Math.min(280, openBelow ? below : above)}px`,
        });
    }

    function sync() {
        frame = 0;
        if (drag && (!drag.tab.isConnected || !drag.source.matches('.window-tabs-movable') || drag.source.closest('.hidden,[inert]'))) finish();
        const next = new Set();
        for (const control of document.querySelectorAll('.window-tabs-overflow')) {
            const strip = control.previousElementSibling;
            if (!strip?.matches('.window-tab-strip')) continue;
            next.add(strip);
            if (!observed.has(strip)) resize.observe(strip);
            const layer = strip.closest('.desktop-window-layer');
            if (layer) {
                next.add(layer);
                if (!observed.has(layer)) resize.observe(layer);
            }
            const gap = parseFloat(getComputedStyle(control.parentElement).columnGap) || 0;
            const reserved = control.offsetWidth ? control.offsetWidth + gap : 0;
            const overflow = strip.getClientRects().length > 0 && strip.scrollWidth > strip.clientWidth + reserved + 1;
            control.toggleAttribute('data-tabs-overflow', overflow);
            const menu = control.querySelector('.window-tabs-menu');
            if (menu.matches(':popover-open')) {
                if (overflow && !control.closest('[inert],.hidden')) positionMenu(menu, control.firstElementChild);
                else menu.hidePopover();
            }

            const active = strip.querySelector('.window-tab-select[aria-pressed=true]');
            if (active && active !== activeTabs.get(strip) && strip.clientWidth) {
                activeTabs.set(strip, active);
                const tab = active.getBoundingClientRect();
                const viewport = strip.getBoundingClientRect();
                if (tab.left < viewport.left) strip.scrollLeft += tab.left - viewport.left;
                else if (tab.right > viewport.right) strip.scrollLeft += tab.right - viewport.right;
            }
        }
        for (const node of observed) if (!next.has(node)) resize.unobserve(node);
        observed = next;
    }

    function schedule() {
        if (!frame) frame = requestAnimationFrame(sync);
    }

    document.addEventListener('click', event => {
        const trigger = event.target.closest?.('.window-tabs-more');
        if (!trigger || trigger.closest('[inert]')) return;
        const control = trigger.parentElement;
        const menu = control.querySelector('.window-tabs-menu');
        if (menu.matches(':popover-open')) {
            menu.hidePopover();
            return;
        }
        if (!menu.hasAttribute('popover')) {
            menu.setAttribute('popover', 'auto');
            menu.setAttribute('role', 'group');
            menu.addEventListener('toggle', () => trigger.setAttribute('aria-expanded', String(menu.matches(':popover-open'))));
        }
        menu.setAttribute('aria-label', trigger.getAttribute('aria-label'));
        const items = [...control.previousElementSibling.querySelectorAll('.window-tab-select')].map(tab => {
            const item = document.createElement('button');
            item.type = 'button';
            item.textContent = tab.textContent.trim() || tab.getAttribute('aria-label');
            item.title = tab.getAttribute('aria-label') || item.textContent;
            if (tab.getAttribute('aria-pressed') === 'true') item.setAttribute('aria-current', 'true');
            item.addEventListener('click', () => {
                menu.hidePopover();
                if (tab.isConnected) tab.click();
                trigger.focus({preventScroll: true});
            });
            return item;
        });
        menu.replaceChildren(...items);
        menu.showPopover();
        positionMenu(menu, trigger);
        (menu.querySelector('[aria-current=true]') || items[0])?.focus();
    });

    document.addEventListener('wheel', event => {
        const strip = event.target.closest?.('.window-tab-strip');
        if (!strip || event.defaultPrevented || event.ctrlKey || event.deltaX || !event.deltaY
            || strip.scrollWidth <= strip.clientWidth) return;
        const scale = event.deltaMode === 1 ? 20 : event.deltaMode === 2 ? strip.clientWidth : 1;
        strip.scrollLeft += event.deltaY * scale;
        event.preventDefault();
    }, {passive: false});

    new MutationObserver(schedule).observe(document, {
        subtree: true, childList: true, characterData: true, attributes: true,
        attributeFilter: ['class', 'aria-pressed', 'inert'],
    });
    schedule();
})();
