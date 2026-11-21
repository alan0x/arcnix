// Keep the native browser surface inside its DOM window. Subtract the
// rectangles of apps and overlays above it before exposing the child HWND.
function observeNativeSurface(app) {
    let previousInput;
    let previous;
    let frame;
    const watched = new Set();
    const resize = new ResizeObserver(schedule);

    function rect(element) {
        if (!element || element.closest('.hidden,[hidden]') || !element.getClientRects().length) return null;
        const { left, top, right, bottom } = element.getBoundingClientRect();
        const scale = window.devicePixelRatio || 1;
        const box = {
            left: Math.round(left * scale),
            top: Math.round(top * scale),
            right: Math.round(right * scale),
            bottom: Math.round(bottom * scale),
        };
        return box.left < box.right && box.top < box.bottom ? box : null;
    }

    function sync() {
        frame = 0;
        const browser = document.querySelector(`.desktop-window.app-${app}`);
        const bridge = browser?.querySelector('.browser-viewport-input');
        if (!bridge) {
            previousInput = previous = undefined;
            for (const node of watched) resize.unobserve(node);
            watched.clear();
            return;
        }
        const surface = browser.querySelector(app === 'player' ? '.remote-player-screen' : '.browser-surface');
        const commandButton = app === 'code' ? browser.querySelector('.code-quick-open') : null;
        const menuSlot = app === 'code' ? browser.querySelector('.code-menu-slot') : null;
        const canvas = browser.closest('.desktop-canvas');
        const layer = browser.closest('.desktop-window-layer');
        const windows = Array.from(layer.querySelectorAll('.desktop-window:not(.hidden)'));
        const next = new Set([surface, commandButton, menuSlot, layer, ...windows].filter(Boolean));
        for (const node of watched) {
            if (!next.has(node)) {
                resize.unobserve(node);
                watched.delete(node);
            }
        }
        for (const node of next) {
            if (!watched.has(node)) {
                resize.observe(node);
                watched.add(node);
            }
        }
        const workArea = rect(layer);
        const surfaceBox = rect(surface);
        const area = workArea && surfaceBox && !browser.closest('[inert]')
            && surfaceBox.right > workArea.left && surfaceBox.left < workArea.right
            && surfaceBox.bottom > workArea.top && surfaceBox.top < workArea.bottom ? surfaceBox : null;
        const commandCenter = area && rect(commandButton);
        const top = commandCenter?.top ?? area?.top;
        const rank = Number(getComputedStyle(browser).zIndex) || 0;
        const blockers = windows.filter(other => other !== browser && (Number(getComputedStyle(other).zIndex) || 0) > rank);
        const overlays = canvas.querySelectorAll(
            '.file-context-menu,.file-actions-menu,.terminal-context-menu,.desktop-context-menu,.desktop-icon-menu,.sapid-tooltip,.window-tabs-menu:popover-open'
        );
        const clip = area ? [...blockers, ...overlays].map(rect).filter(box => box
            && box.right > area.left && box.left < area.right && box.bottom > top && box.top < area.bottom
        ).slice(0, 32) : [];
        const epoch = Number(browser.querySelector('.browser-epoch').textContent);
        const serialized = JSON.stringify({epoch, area, frame: workArea, clip, command_center: commandCenter, menu_slot: rect(menuSlot)});
        if (bridge !== previousInput || serialized !== previous) {
            previousInput = bridge;
            previous = serialized;
            bridge.value = serialized;
            bridge.dispatchEvent(new Event('input', {bubbles: true}));
        }
        if (area && windows.some(element => element.getAnimations().some(animation => animation.playState === "running"))) schedule();
    }

    function schedule() {
        if (!frame) frame = requestAnimationFrame(sync);
    }

    new MutationObserver(schedule).observe(document, {
        subtree: true, childList: true, attributes: true,
        attributeFilter: ['class', 'hidden', 'inert', 'style', 'data-positioned'],
    });
    addEventListener('resize', schedule);
    addEventListener('scroll', schedule, true);
    schedule();
}
observeNativeSurface('browser');
observeNativeSurface('code');
observeNativeSurface('player');
(() => {
    document.addEventListener('keydown', event => {
        const code = event.target.closest?.('.desktop-window.app-code.active');
        if (code && !code.closest('[inert]') && !event.isComposing &&
            (event.ctrlKey || event.metaKey) && !event.altKey && !event.shiftKey &&
            event.key.toLowerCase() === 'p') {
            event.preventDefault();
            event.stopImmediatePropagation();
            code.querySelector('.code-quick-open')?.click();
            return;
        }
        const browser = event.target.closest?.('.desktop-window.app-browser.active');
        if (!browser || browser.closest('[inert]') || event.isComposing) return;
        if (event.key === 'Enter' && event.target.matches('.browser-address')) {
            browser.querySelector('.browser-open-address')?.click();
        } else if (event.ctrlKey && !event.altKey && !event.shiftKey && event.key.toLowerCase() === 'l') {
            const address = browser.querySelector('.browser-address');
            address?.focus();
            address?.select();
        } else {
            return;
        }
        event.preventDefault();
        event.stopImmediatePropagation();
    }, true);
})();
