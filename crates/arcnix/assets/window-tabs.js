(() => {
    const activeTabs = new WeakMap();
    let observed = new Set();
    let frame;
    const resize = new ResizeObserver(schedule);

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
