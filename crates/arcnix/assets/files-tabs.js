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

    // A wheel notch is typically 100 pixels or three lines. Trackpads send
    // many small deltas. Accumulate within one gesture on one tab.
    let wheelTarget, lastWheel = 0, distance = 0;
    document.addEventListener('wheel', event => {
        if (!event.ctrlKey || event.altKey || event.shiftKey || event.metaKey || !event.deltaY) return;
        const files = event.target.closest?.(windowSelector);
        const tab = files?.querySelector('.files-tab.selected');
        if (!tab || files.closest('[inert],.hidden') || !files.querySelector('.files-drop-directory')?.textContent
            || event.target.closest('.pane-menu,.files-search-options')) return;
        event.preventDefault();
        const delta = event.deltaY * (event.deltaMode === 1 ? 60 : event.deltaMode === 2 ? 100 : 1);
        if (tab !== wheelTarget || event.timeStamp - lastWheel > 220 || Math.sign(delta) !== Math.sign(distance)) distance = 0;
        wheelTarget = tab;
        lastWheel = event.timeStamp;
        distance += delta;
        if (Math.abs(distance) < 60) return;
        distance = 0;
        send({kind: 'zoom', window: windowId(files), tab: tabId(tab), larger: delta < 0});
    }, {capture: true, passive: false});

    document.addEventListener('keydown', event => {
        const active = document.querySelector(windowSelector + '.active');
        if (!active?.offsetWidth || active.closest('[inert]') || event.repeat || event.isComposing) return;
        let gesture;
        if (arcnixKeys.matches(event, 'files_hidden', 'Ctrl+H')
            && !event.target.closest?.('input, textarea, select, [contenteditable]')) {
            gesture = {kind: 'toggle_hidden', window: windowId(active)};
        } else if (arcnixKeys.matches(event, 'files_new_tab')) {
            gesture = {kind: 'new', window: windowId(active)};
        } else if ((event.ctrlKey || event.metaKey) && !event.shiftKey && !event.altKey
            && event.key.toLowerCase() === 'w') {
            const tab = active.querySelector('.files-tab.selected');
            if (tab) gesture = {kind: 'close', window: windowId(active), tab: tabId(tab)};
        }
        if (!gesture) return;
        event.preventDefault();
        event.stopImmediatePropagation();
        send(gesture);
    }, true);
})();
