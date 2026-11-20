(() => {
    const windowSelector = '.desktop-window.app-files:not(.hidden)';
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
        if (!active?.offsetWidth || active.closest('[inert]') || event.repeat || event.isComposing) return;
        let gesture;
        if (arcnixKeys.matches(event, 'files_hidden', 'Ctrl+H')
            && !event.target.closest?.('input, textarea, select, [contenteditable]')) {
            gesture = {kind: 'toggle_hidden', window: windowId(active)};
        } else if ((event.ctrlKey || event.metaKey) && !event.shiftKey && !event.altKey) {
            if (event.key.toLowerCase() === 't') gesture = {kind: 'new', window: windowId(active)};
            if (event.key.toLowerCase() === 'w') {
                const tab = active.querySelector('.files-tab.selected');
                if (tab) gesture = {kind: 'close', window: windowId(active), tab: tabId(tab)};
            }
        }
        if (!gesture) return;
        event.preventDefault();
        event.stopImmediatePropagation();
        send(gesture);
    }, true);
})();
