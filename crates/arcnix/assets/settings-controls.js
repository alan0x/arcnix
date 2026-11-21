// Glory exposes a text-input purpose. Upgrade the ColorInput swatch to the
// platform color chooser while keeping its value and events owned by Glory.
(() => {
    const selector = 'input.theme-accent-picker';
    function upgrade(root) {
        if (root.matches?.(selector) && root.type !== 'color') root.type = 'color';
    }
    new MutationObserver(records => {
        for (const record of records) {
            if (record.type === 'attributes') upgrade(record.target);
            for (const node of record.addedNodes) {
                upgrade(node);
                node.querySelectorAll?.(selector).forEach(upgrade);
            }
        }
    }).observe(document, {subtree: true, childList: true, attributes: true, attributeFilter: ['type']});
    document.querySelectorAll(selector).forEach(upgrade);
})();
