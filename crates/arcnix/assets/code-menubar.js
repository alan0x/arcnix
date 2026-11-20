// Leave the blank part of the embedded titlebar to the desktop window manager.
(() => {
    let menu, reportedWidth, frame;
    const resize = new ResizeObserver(schedule);
    const mutations = new MutationObserver(schedule);

    function measure() {
        frame = 0;
        const next = document.querySelector('.part.titlebar .menubar');
        if (!next) return;
        if (menu !== next) {
            resize.disconnect();
            mutations.disconnect();
            menu = next;
            resize.observe(menu);
            mutations.observe(menu.closest('.part.titlebar'), {
                childList: true, subtree: true, characterData: true, attributes: true,
                attributeFilter: ['class', 'style', 'hidden'],
            });
            mutations.observe(document.documentElement, { attributes: true, attributeFilter: ['class'] });
        }
        if (!document.documentElement.classList.contains('has-menu-slot')) return;
        const bar = menu.getBoundingClientRect();
        let right = bar.left;
        for (const button of menu.querySelectorAll(':scope > .menubar-menu-button')) {
            const bounds = button.getBoundingClientRect();
            if (bounds.width && bounds.height) right = Math.max(right, bounds.right);
        }
        const width = Math.round((Math.min(right, bar.right) - bar.left) * devicePixelRatio);
        if (width !== reportedWidth) {
            reportedWidth = width;
            window.ipc.postMessage(JSON.stringify({ op: 'menuWidth', width }));
        }
    }

    function schedule() {
        if (!frame) frame = requestAnimationFrame(measure);
    }

    window.addEventListener('resize', schedule);
    mutations.observe(document, { childList: true, subtree: true });
    schedule();
})();
