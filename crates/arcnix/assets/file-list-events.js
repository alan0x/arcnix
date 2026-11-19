// Carry native click counts/modifiers into finite Rust-owned browser gestures.
(() => {
    let sequence = 0;
    let lastPane = null;
    document.addEventListener("pointerdown", () => document.documentElement.classList.remove("keyboard-navigation"), true);
    document.addEventListener("keydown", () => document.documentElement.classList.add("keyboard-navigation"), true);
    const additive = event => /Mac|iPhone|iPad/.test(navigator.platform) ? event.metaKey : event.ctrlKey;
    function send(pane, kind, row, event = {}, extra = {}) {
        const bridge = pane?.querySelector(".file-gesture-input");
        if (!bridge || pane.closest("[inert],[hidden]")) return;
        bridge.value = JSON.stringify({kind, path: row?.getAttribute("aria-label") || "", toggle: !!additive(event), range: !!event.shiftKey, ...extra, sequence: ++sequence});
        bridge.dispatchEvent(new Event("input", {bubbles: true}));
    }
    function finishName(field, cancel) {
        send(validPane(field), 'finish_name', field.closest('.file-row'), {}, {name: cancel ? null : field.value});
    }
    document.addEventListener('keydown', event => {
        const field = event.target;
        if (!field.matches?.('.file-inline-name') || event.isComposing || event.keyCode === 229) return;
        if (event.key !== 'Escape' && event.key !== 'Enter') return;
        event.preventDefault();
        event.stopImmediatePropagation();
        finishName(field, event.key === 'Escape');
    }, true);
    document.addEventListener('focusout', event => {
        if (event.target.matches?.('.file-inline-name') && event.target.isConnected) finishName(event.target, false);
    }, true);
    let fileMenu = null;
    let menuScheduled = false;
    function validPane(target) {
        const pane=target instanceof Element ? target.closest('.file-pane') : null;
        return pane && !pane.closest('[inert],[hidden],.hidden') ? pane : null;
    }
    function pathPane(target) {
        if (!(target instanceof Element)) return null;
        const explorer = target.closest('.explorer-titlebar')?.closest('.desktop-window');
        return validPane(explorer?.querySelector('.explorer-pane') || target);
    }
    const fileDragType = 'application/x-arcnix-file-transfer';
    const favoriteDragType = 'application/x-arcnix-explorer-favorite';
    let fileDrag = null;
    let favoriteDrag = null;
    let favoriteIndicator = null;
    function clearFavoriteIndicator() {
        favoriteIndicator?.removeAttribute('data-favorite-drop');
        favoriteIndicator = null;
    }
    function favoriteZone(target) {
        const zone = target instanceof Element && target.closest('.explorer-favorites.editable');
        return zone && !zone.closest('[inert],[hidden],.hidden') ? zone : null;
    }
    function favoriteSource(row) {
        const panel = row.closest('.remote-explorer');
        const fromFavorites = row.classList.contains('explorer-favorite-row');
        const zone = favoriteZone(panel?.querySelector('.explorer-favorites'));
        if (!zone || row.closest('.hidden')) return null;
        if (!fromFavorites && !(validPane(row) && row.classList.contains('file-draggable'))) return null;
        return {panel, fromFavorites};
    }
    function favoriteTarget(event) {
        const zone = favoriteZone(event.target);
        if (!zone || !favoriteDrag.panel.contains(zone)) return null;
        const row = event.target.closest('.explorer-favorite-row');
        const box = row?.getBoundingClientRect();
        const anchor = row?.getAttribute('aria-label') || null;
        if (anchor === favoriteDrag.path) return null;
        return {zone, row, anchor, after: !!box && event.clientY >= box.top + box.height / 2};
    }
    function clearDropTarget() {
        document.querySelectorAll('[data-file-drop-target]').forEach(pane => pane.removeAttribute('data-file-drop-target'));
    }
    function clearFileDrag() {
        clearFavoriteIndicator();
        if (favoriteDrag?.downloadPane) send(favoriteDrag.downloadPane, 'drag_end', null, {}, {drag_id: favoriteDrag.id});
        favoriteDrag = null;
        delete document.documentElement.dataset.explorerDownloading;
        if (fileDrag) send(fileDrag.pane, 'drag_end', null, {}, {drag_id:fileDrag.id});
        document.querySelectorAll('[data-file-drag-source]').forEach(pane => pane.removeAttribute('data-file-drag-source'));
        clearDropTarget();
        fileDrag = null;
    }
    function dragPaneReady(pane) {
        return pane?.isConnected && validPane(pane) === pane && pane.classList.contains('file-transfer-ready');
    }
    function dropPane(target) {
        const pane = validPane(target);
        return fileDrag && pane !== fileDrag.pane && dragPaneReady(pane) && dragPaneReady(fileDrag.pane) ? pane : null;
    }
    function explorerDownloadTarget(target) {
        const drop = target instanceof Element && target.closest('.explorer-download-drop');
        return drop && !drop.closest('[inert],.hidden,[hidden]') && drop.getClientRects().length;
    }
    function explorerUploadTarget(event) {
        const pane = validPane(event.target);
        return !favoriteDrag && !fileDrag && pane?.classList.contains('explorer-upload-ready')
            && event.dataTransfer?.types.includes('Files') ? pane : null;
    }
    let staging = false;
    async function stageUpload(pane, files) {
        const explorer = pane.closest('.remote-explorer');
        const bridge = explorer.querySelector('.explorer-upload-input');
        const ack = explorer.querySelector('.explorer-upload-ack');
        const snapshot = JSON.parse(explorer.querySelector('.explorer-favorites-state').textContent);
        const id = ++sequence;
        let serial = 0;
        const transmit = event => new Promise((resolve, reject) => {
            if (!bridge.isConnected) { reject(); return; }
            const prefix = `${id}:${++serial}:`;
            const observer = new MutationObserver(() => {
                if (!ack.textContent.startsWith(prefix)) return;
                observer.disconnect();
                clearTimeout(timeout);
                if (ack.textContent === prefix + '1') resolve();
                else reject();
            });
            const timeout = setTimeout(() => { observer.disconnect(); reject(); }, 10000);
            observer.observe(ack, {childList: true, subtree: true, characterData: true});
            bridge.value = JSON.stringify({id, serial, ...event});
            bridge.dispatchEvent(new Event('input', {bubbles: true}));
        });
        staging = true;
        try {
            await transmit({kind: 'begin', ...snapshot});
            for (const file of files) {
                await transmit({kind: 'file', name: file.name, size: file.size});
                const chunkSize = 256 * 1024;
                for (let offset = 0; offset < file.size; offset += chunkSize) {
                    const bytes = new Uint8Array(await file.slice(offset, offset + chunkSize).arrayBuffer());
                    let data;
                    if (bytes.toBase64) {
                        data = bytes.toBase64();
                    } else {
                        let binary = '';
                        for (let start = 0; start < bytes.length; start += 32768) {
                            binary += String.fromCharCode(...bytes.subarray(start, start + 32768));
                        }
                        data = btoa(binary);
                    }
                    await transmit({kind: 'chunk', data});
                }
                await transmit({kind: 'finish_file'});
            }
            await transmit({kind: 'commit'});
        } catch {
            if (bridge.isConnected) {
                bridge.value = JSON.stringify({id, serial: ++serial, kind: 'abort'});
                bridge.dispatchEvent(new Event('input', {bubbles: true}));
            }
        } finally {
            staging = false;
        }
    }
    document.addEventListener('dragstart', event => {
        const row = event.target instanceof Element && event.target.closest('.file-row,.explorer-favorite-row');
        if (!row) return;
        if (row.closest('.remote-explorer')) {
            const source = favoriteSource(row);
            const pane = validPane(row);
            const downloadPane = row.classList.contains('file-draggable') && dragPaneReady(pane) ? pane : null;
            const path = row.getAttribute('aria-label');
            if ((!source && !downloadPane) || !path || !event.dataTransfer) { event.preventDefault(); return; }
            clearFileDrag();
            const panel = row.closest('.remote-explorer');
            const snapshot = JSON.parse(panel.querySelector('.explorer-favorites-state').textContent);
            favoriteDrag = {panel, ...snapshot, path, id: ++sequence, fromFavorites: !!source?.fromFavorites, downloadPane};
            if (downloadPane) {
                document.documentElement.dataset.explorerDownloading = 'true';
                send(downloadPane, 'drag_start', row, {}, {drag_id: favoriteDrag.id});
            }
            event.dataTransfer.effectAllowed = favoriteDrag.fromFavorites ? 'move' : 'copy';
            event.dataTransfer.setData(favoriteDragType, String(favoriteDrag.id));
            event.dataTransfer.setDragImage(row, 16, row.offsetHeight / 2);
            return;
        }
        const pane = validPane(row);
        if (!dragPaneReady(pane) || !row.classList.contains('file-draggable') || !event.dataTransfer) {
            event.preventDefault(); return;
        }
        clearFileDrag();
        fileDrag = {pane, id:++sequence};
        event.dataTransfer.effectAllowed = 'copy';
        event.dataTransfer.setData(fileDragType, String(fileDrag.id));
        event.dataTransfer.setDragImage(row, 20, row.offsetHeight / 2);
        pane.setAttribute('data-file-drag-source', 'true');
        send(pane, 'drag_start', row, {}, {drag_id:fileDrag.id});
    }, true);
    document.addEventListener('dragover', event => {
        if (favoriteDrag) {
            clearFavoriteIndicator();
            const target = favoriteTarget(event);
            const download = favoriteDrag.downloadPane && explorerDownloadTarget(event.target);
            event.preventDefault();
            event.dataTransfer.dropEffect = target ? (favoriteDrag.fromFavorites ? 'move' : 'copy') : download ? 'copy' : 'none';
            if (target) {
                favoriteIndicator = target.row || target.zone;
                favoriteIndicator.setAttribute('data-favorite-drop', target.row ? (target.after ? 'after' : 'before') : 'append');
            }
            return;
        }
        const upload = explorerUploadTarget(event);
        if (upload) {
            event.preventDefault();
            event.stopImmediatePropagation();
            event.dataTransfer.dropEffect = staging ? 'none' : 'copy';
            if (!staging) upload.setAttribute('data-file-drop-target', 'true');
            return;
        }
        if (!fileDrag) return;
        clearDropTarget();
        const pane = dropPane(event.target);
        event.preventDefault();
        event.dataTransfer.dropEffect = pane ? 'copy' : 'none';
        if (pane) pane.setAttribute('data-file-drop-target', 'true');
    }, true);
    document.addEventListener('dragleave', event => {
        if (favoriteIndicator && (!(event.relatedTarget instanceof Node) || !favoriteIndicator.contains(event.relatedTarget))) clearFavoriteIndicator();
        const pane = validPane(event.target);
        if (pane && (!(event.relatedTarget instanceof Node) || !pane.contains(event.relatedTarget))) {
            pane.removeAttribute('data-file-drop-target');
        }
    }, true);
    document.addEventListener('drop', event => {
        if (favoriteDrag) {
            event.preventDefault(); event.stopImmediatePropagation();
            const target = favoriteTarget(event);
            const matches = event.dataTransfer?.getData(favoriteDragType) === String(favoriteDrag.id);
            if (target && matches) {
                const {panel, path, fromFavorites, epoch, request} = favoriteDrag;
                const bridge = panel.querySelector('.explorer-favorites-input');
                bridge.value = JSON.stringify({path, from_favorites: fromFavorites, epoch, request, anchor: target.anchor, after: target.after});
                bridge.dispatchEvent(new Event('input', {bubbles: true}));
            } else if (matches && favoriteDrag.downloadPane && explorerDownloadTarget(event.target)) {
                send(favoriteDrag.downloadPane, 'explorer_download', null, {}, {drag_id: favoriteDrag.id});
            }
            clearFileDrag();
            return;
        }
        const upload = explorerUploadTarget(event);
        if (upload) {
            event.preventDefault();
            event.stopImmediatePropagation();
            clearDropTarget();
            const files = [...event.dataTransfer.items]
                .filter(item => item.kind === 'file' && !item.webkitGetAsEntry?.()?.isDirectory)
                .map(item => item.getAsFile()).filter(Boolean);
            if (!staging && files.length) void stageUpload(upload, files);
            return;
        }
        if (!fileDrag) return;
        event.preventDefault(); event.stopImmediatePropagation();
        const pane = dropPane(event.target);
        if (pane && event.dataTransfer?.getData(fileDragType) === String(fileDrag.id)) {
            send(pane, 'transfer_drop', null, {}, {drag_id:fileDrag.id});
        }
        clearFileDrag();
    }, true);
    document.addEventListener('dragend', clearFileDrag, true);
    window.addEventListener('blur', clearFileDrag);
    function updateDraggableRows() {
        if (fileDrag && !dragPaneReady(fileDrag.pane)) clearFileDrag();
        for (const row of document.querySelectorAll('.file-row')) {
            const draggable = !row.querySelector('.file-inline-name') && ((row.closest('.remote-explorer') && !!favoriteSource(row))
                || (row.classList.contains('file-draggable') && dragPaneReady(validPane(row))));
            if (row.draggable !== !!draggable) row.draggable = !!draggable;
        }
        for (const row of document.querySelectorAll('.explorer-favorite-row')) {
            const draggable = !!favoriteSource(row);
            if (row.draggable !== draggable) row.draggable = draggable;
        }
    }
    new MutationObserver(updateDraggableRows).observe(document, {subtree:true, childList:true, attributes:true, attributeFilter:['class','inert','hidden']});
    function context(target,event,keyboard=false) {
        const pane=validPane(target);
        const listing=target instanceof Element && target.closest('.file-list-container,.file-empty');
        if(!pane||!listing||target.closest('.file-inline-name'))return false;
        const row=target.closest('.file-row');
        const origin=row?.querySelector('.file-name:not(:disabled)')||pane.querySelector('.file-list-container');
        origin?.focus({preventScroll:true});
        const box=(row||listing).getBoundingClientRect();
        fileMenu={pane,origin,x:keyboard?box.left+24:event.clientX,y:keyboard?box.bottom:event.clientY,opened:false,focus:true,toolbar:false};
        lastPane=pane;
        send(pane,'context',row);
        scheduleMenu();
        return true;
    }
    document.addEventListener('contextmenu',event=>{
        const target=event.target instanceof Element&&event.target.closest('.menu-dismiss')
            ? document.elementsFromPoint(event.clientX,event.clientY).find(item=>item.closest('.file-list-container,.file-empty')) : event.target;
        if(context(target,event)){event.preventDefault();event.stopImmediatePropagation();}
    },true);
    document.addEventListener('click',event=>{
        const button=event.target instanceof Element&&event.target.closest('.pane-menu-anchor>button');
        const pane=pathPane(button);
        if(!pane)return;
        const box=button.getBoundingClientRect();
        fileMenu={pane,origin:button,x:box.right,y:box.bottom,opened:false,focus:true,toolbar:true};
        scheduleMenu();
    },true);
    function dismissMenu(restore) {
        if(fileMenu)fileMenu.restore=restore;
        document.querySelector('.menu-dismiss:not(.hidden)')?.click();
        scheduleMenu();
    }
    document.addEventListener('keydown',event=>{
        if(!(event.target instanceof Element)||event.isComposing)return;
        const menu=event.target.closest('.file-context-menu,.file-actions-menu');
        if(menu){
            if(['ArrowDown','ArrowUp','Home','End'].includes(event.key)){
                const buttons=[...menu.querySelectorAll('button:not(:disabled)')].filter(b=>b.getClientRects().length);
                const index=buttons.indexOf(event.target.closest('button'));
                const next=event.key==='Home'?0:event.key==='End'?buttons.length-1:(index+(event.key==='ArrowUp'?-1:1)+buttons.length)%buttons.length;
                buttons[next]?.focus();event.preventDefault();event.stopImmediatePropagation();
            } else if(event.key==='Escape'||event.key==='Tab'){
                dismissMenu(true);event.preventDefault();event.stopImmediatePropagation();
            }
            return;
        }
        if(event.target.matches('input,textarea'))return;
        if(event.key==='ContextMenu'||event.shiftKey&&event.key==='F10'){
            if(context(event.target,event,true)){event.preventDefault();event.stopImmediatePropagation();}
            return;
        }
        const pane=validPane(event.target);
        if(!pane||!event.target.closest('.file-list-container'))return;
        let kind=null;
        if(event.key==='F2')kind='rename';
        else if(event.key==='Delete'&&!event.ctrlKey&&!event.metaKey&&!event.altKey&&!event.shiftKey)kind='delete';
        else if(event.key==='F5')kind='refresh';
        else if(event.altKey&&event.key==='Enter')kind='properties';
        else if(additive(event)&&event.shiftKey&&event.key.toLowerCase()==='c')kind='copy_paths';
        if(kind){send(pane,kind,event.target.closest('.file-row'));event.preventDefault();event.stopImmediatePropagation();}
    },true);
    function placeFileMenu(){
        menuScheduled=false;
        if(!fileMenu?.pane.isConnected){fileMenu=null;return;}
        const menu=fileMenu.pane.querySelector('.file-context-menu,.file-actions-menu');
        if(!menu||!menu.getClientRects().length){
            if(fileMenu.opened){
                if(fileMenu.restore&&fileMenu.origin?.isConnected)fileMenu.origin.focus({preventScroll:true});
                fileMenu=null;
            }
            return;
        }
        fileMenu.opened=true;
        const frame=desktopFrame.boundsFor(menu);
        const box=menu.getBoundingClientRect();
        let x=fileMenu.x-(fileMenu.toolbar?box.width:0), y=fileMenu.y;
        if(fileMenu.toolbar&&y+box.height>frame.bottom-8)y=fileMenu.origin.getBoundingClientRect().top-box.height;
        menu.style.left=`${Math.max(frame.left+8,Math.min(x,frame.right-box.width-8))}px`;
        menu.style.top=`${Math.max(frame.top+8,Math.min(y,frame.bottom-box.height-8))}px`;
        if(fileMenu.focus){const first=menu.querySelector('button:not(:disabled)');if(first)first.focus();else menu.focus();fileMenu.focus=false;}
    }
    function scheduleMenu(){if(!menuScheduled){menuScheduled=true;requestAnimationFrame(placeFileMenu);}}
    new MutationObserver(scheduleMenu).observe(document,{subtree:true,childList:true,attributes:true,attributeFilter:['class']});
    document.addEventListener('scroll',event=>{
        if(fileMenu?.opened&&event.target instanceof Element&&!event.target.closest('.pane-menu')&&fileMenu.pane.contains(event.target))dismissMenu(false);
    },true);
    window.addEventListener('resize',()=>{if(fileMenu?.opened)dismissMenu(false);});
    document.addEventListener('click', event => {
        if (!(event.target instanceof Element)) return;
        const options = document.querySelector('.explorer-search-options');
        if (options?.getClientRects().length
            && !event.target.closest('.explorer-search-options,.explorer-search-options-toggle')) {
            options.closest('.pathbar')?.querySelector('.explorer-search-options-toggle')?.click();
        }
        if (event.target.closest('.explorer-search-clear')) {
            event.target.closest('.pathbar')?.querySelector('.explorer-search-input')?.focus();
        }
    }, true);
    document.addEventListener('keydown', event => {
        if (event.isComposing || !(event.target instanceof Element)) return;
        const explorer = event.target.closest('.desktop-window.app-explorer.active')
            || (event.key === 'Escape' && (event.target === document.body || event.target.matches('.desktop-canvas'))
                ? document.querySelector('.server-desktop:not(.hidden) .desktop-window.app-explorer.active')
                : null);
        const toggle = explorer?.querySelector('.explorer-search-toggle:not(:disabled)');
        if (!toggle?.getClientRects().length || toggle.closest('[inert]')) return;
        const input = explorer.querySelector('.explorer-search-input');
        if (event.key === 'Escape' && input) {
            const options = explorer.querySelector('.explorer-search-options');
            const control = options ? explorer.querySelector('.explorer-search-options-toggle') : toggle;
            control?.click();
            control?.focus();
        } else if (additive(event) && !event.shiftKey && !event.altKey && event.key.toLowerCase() === 'f') {
            if (input) input.focus();
            else toggle.click();
        } else return;
        event.preventDefault();
        event.stopImmediatePropagation();
    }, true);
    let breadcrumbClick;
    document.addEventListener("click", event => {
        if (!(event.target instanceof Element) || event.detail === 0) return;
        clearTimeout(breadcrumbClick);
        const crumb = event.target.closest('.breadcrumb');
        if (crumb) {
            event.preventDefault(); event.stopImmediatePropagation();
            // Give the browser time to recognize a double-click before replacing the path.
            if (event.detail === 1 && !crumb.classList.contains('current')) {
                breadcrumbClick = setTimeout(() => {
                    if (crumb.isConnected && crumb.getClientRects().length && pathPane(crumb)) crumb.click();
                }, 500);
            }
            return;
        }
        lastPane = pathPane(event.target) || lastPane;
        const row = event.target.closest(".file-list .file-row");
        if (!row || event.target.closest(".file-expander,.file-inline-name")) return;
        const name = row.querySelector(".file-name");
        if (!name || name.disabled) return;
        event.preventDefault(); event.stopImmediatePropagation();
        name.focus({preventScroll: true});
        if (event.detail === 1) send(row.closest(".file-pane"), "select", row, event);
        else if (event.detail === 2 && !additive(event) && !event.shiftKey) send(row.closest(".file-pane"), "activate", row);
    }, true);
    document.addEventListener('dblclick', event => {
        if (event.button !== 0 || !(event.target instanceof Element) || !event.target.closest('.breadcrumbs')) return;
        const pane = pathPane(event.target);
        if (!pane) return;
        clearTimeout(breadcrumbClick);
        lastPane = pane;
        send(pane, 'edit_path', null);
        event.preventDefault(); event.stopImmediatePropagation();
    }, true);
    document.addEventListener("keydown", event => {
        if (!(event.target instanceof Element) || event.isComposing) return;
        if (event.target.closest(".terminal-module")) return;
        const lastWindow = lastPane?.closest('.desktop-window');
        const pane = pathPane(event.target) || (additive(event) && event.key.toLowerCase() === "l"
            && lastPane?.isConnected && (!lastWindow || lastWindow.classList.contains('active')) ? lastPane : null);
        if (!validPane(pane)) return;
        if (additive(event) && event.key.toLowerCase() === "l") {
            event.preventDefault(); event.stopImmediatePropagation();
            const pathScope = pane.closest('.desktop-window.app-explorer') || pane;
            const field = pathScope.querySelector(".path-input:not(.hidden)");
            if (field) { field.focus(); field.select(); }
            else send(pane, 'edit_path', null);
            return;
        }
        if (event.target.matches("input,textarea") || event.altKey) return;
        const row = event.target.closest(".file-row");
        if (!row) return;
        let handled = true;
        if (additive(event) && event.key.toLowerCase() === "a") send(pane, "all", row);
        else if (event.key === " ") send(pane, "select", row, event);
        else if (pane.classList.contains("icons-view") && ["ArrowLeft", "ArrowRight"].includes(event.key)) {
            const rows = [...pane.querySelectorAll(".file-row")];
            const target = rows[Math.max(0, Math.min(rows.length - 1, rows.indexOf(row) + (event.key === "ArrowLeft" ? -1 : 1)))];
            if (target) { target.querySelector(".file-name").focus(); if (!additive(event) || event.shiftKey) send(pane, "select", target, event); }
        }
        else if (event.key === "ArrowRight") send(pane, "expand", row);
        else if (event.key === "ArrowLeft") {
            if (row.querySelector(".file-expander")?.getAttribute("aria-expanded") === "true") send(pane, "collapse", row);
            else {
                const rows = [...pane.querySelectorAll(".file-row")];
                const depth = parseFloat(getComputedStyle(row).paddingLeft);
                const parent = rows.slice(0, rows.indexOf(row)).reverse().find(item => parseFloat(getComputedStyle(item).paddingLeft) < depth);
                if (parent) { parent.querySelector(".file-name")?.focus(); send(pane, "select", parent); }
            }
        } else if (["ArrowUp", "ArrowDown", "Home", "End"].includes(event.key)) {
            const rows = [...pane.querySelectorAll(".file-row")].filter(item => !item.querySelector(".file-name").disabled);
            const index = rows.indexOf(row);
            const columns = pane.classList.contains("icons-view") ? Math.max(1, getComputedStyle(pane.querySelector(".file-list")).gridTemplateColumns.split(/\s+/).length) : 1;
            const next = event.key === "Home" ? 0 : event.key === "End" ? rows.length - 1 : index + (event.key === "ArrowUp" ? -columns : columns);
            const target = rows[Math.max(0, Math.min(rows.length - 1, next))];
            if (target) { target.querySelector(".file-name").focus(); if (!additive(event) || event.shiftKey) send(pane, "select", target, event); }
        } else handled = false;
        if (handled) { event.preventDefault(); event.stopImmediatePropagation(); }
    }, true);
    const focused = new WeakSet();
    const currentPaths = new WeakMap();
    new MutationObserver(() => {
        for (const field of document.querySelectorAll(".path-input,.explorer-search-input,.file-inline-name")) {
            if (field.classList.contains("hidden")) focused.delete(field);
            else if (!focused.has(field) && field.getClientRects().length) { focused.add(field); field.focus(); field.select(); }
        }
        for (const bar of document.querySelectorAll(".breadcrumbs")) {
            const current = bar.querySelector(".current")?.getAttribute("aria-label");
            if (current && currentPaths.get(bar) !== current) { currentPaths.set(bar, current); bar.scrollLeft = bar.scrollWidth; }
        }
    }).observe(document, {subtree: true, childList: true, attributes: true, attributeFilter: ["class"]});
    document.addEventListener("wheel", event => {
        const bar = event.target instanceof Element && event.target.closest(".breadcrumbs");
        if (bar && !event.ctrlKey && bar.scrollWidth > bar.clientWidth) { event.preventDefault(); bar.scrollLeft += event.deltaX || event.deltaY; }
    }, {passive:false});
})();
