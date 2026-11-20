(() => {
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

    document.addEventListener('keydown', event => {
        const active = document.querySelector(windowSelector + '.active');
        if (!active?.offsetWidth || active.closest('[inert]') || event.altKey || event.repeat) return;
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
