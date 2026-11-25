// Carry native click counts/modifiers into finite Rust-owned browser gestures.
(() => {
    let sequence = 0;
    let lastPane = null;
    const additive = event => /Mac|iPhone|iPad/.test(navigator.platform) ? event.metaKey : event.ctrlKey;
    function send(pane, kind, row, event = {}, extra = {}) {
        const bridge = pane?.querySelector(".file-gesture-input");
        if (!bridge || pane.closest("[inert],[hidden]")) return;
        bridge.value = JSON.stringify({kind, path: row?.getAttribute("aria-label") || "", toggle: !!additive(event), range: !!event.shiftKey, ...extra, sequence: ++sequence});
        bridge.dispatchEvent(new Event("input", {bubbles: true}));
    }
    const cancelledNames = new WeakSet();
    function finishName(field, cancel) {
        send(validPane(field), 'finish_name', field.closest('.file-row'), {}, {name: cancel ? null : field.value});
    }
    document.addEventListener('keydown', event => {
        const field = event.target;
        if (!field.matches?.('.file-inline-name') || event.isComposing || event.keyCode === 229) return;
        if (event.key !== 'Escape' && event.key !== 'Enter') return;
        event.preventDefault();
        event.stopImmediatePropagation();
        const cancel = event.key === 'Escape';
        if (cancel) cancelledNames.add(field);
        finishName(field, cancel);
        if (cancel) field.closest('.file-row')?.querySelector('.file-name')?.focus({preventScroll: true});
    }, true);
    document.addEventListener('focusout', event => {
        const field = event.target;
        if (field.matches?.('.file-inline-name') && field.isConnected && !cancelledNames.delete(field)) {
            finishName(field, false);
        }
    }, true);
    let fileMenu = null;
    let menuScheduled = false;
    let submenuClose;
    function validPane(target) {
        const pane=target instanceof Element ? target.closest('.file-pane') : null;
        return pane && !pane.closest('[inert],[hidden],.hidden') ? pane : null;
    }
    function pathPane(target) {
        if (!(target instanceof Element)) return null;
        const files = target.closest('.desktop-window.app-files');
        return validPane(files?.querySelector('.files-pane') || target);
    }
    function keyboardFiles(target) {
        const window = target.closest('.desktop-window');
        if (window) return window.matches('.app-files.active') ? window : null;
        if (target !== document.body && !target.closest('.desktop-canvas')) return null;
        return document.querySelector('.server-desktop:not(.hidden) .desktop-window.app-files.active');
    }
    const fileDragType = 'application/x-arcnix-file-transfer';
    const favoriteDragType = 'application/x-arcnix-files-favorite';
    let fileDrag = null;
    let favoriteDrag = null;
    let favoriteIndicator = null;
    let fileTarget = null;
    const copiesFiles = event => /Mac|iPhone|iPad/.test(navigator.platform) ? event.altKey : event.ctrlKey;
    function clearFavoriteIndicator() {
        favoriteIndicator?.removeAttribute('data-favorite-drop');
        favoriteIndicator = null;
    }
    function favoriteZone(target) {
        const zone = target instanceof Element && target.closest('.files-favorites.editable');
        return zone && !zone.closest('[inert],[hidden],.hidden') ? zone : null;
    }
    function favoriteSource(row) {
        const panel = row.closest('.remote-files');
        const fromFavorites = row.classList.contains('files-favorite-row');
        const zone = favoriteZone(panel?.querySelector('.files-favorites'));
        if (!zone || row.closest('.hidden')) return null;
        if (!fromFavorites && !(validPane(row) && row.classList.contains('file-draggable'))) return null;
        return {panel, fromFavorites};
    }
    function favoriteTarget(event) {
        const zone = favoriteZone(event.target);
        if (!zone) return null;
        const row = event.target.closest('.files-favorite-row');
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
        clearFileTarget();
        if (favoriteDrag?.downloadPane) send(favoriteDrag.downloadPane, 'drag_end', null, {}, {drag_id: favoriteDrag.id});
        favoriteDrag = null;
        delete document.documentElement.dataset.filesDownloading;
        if (fileDrag) send(fileDrag.pane, 'drag_end', null, {}, {drag_id:fileDrag.id});
        document.querySelectorAll('[data-file-drag-source]').forEach(pane => pane.removeAttribute('data-file-drag-source'));
        clearDropTarget();
        fileDrag = null;
    }
    function clearFileTarget() {
        fileTarget?.removeAttribute('data-files-item-target');
        fileTarget = null;
    }
    function itemTarget(event) {
        if (!favoriteDrag?.sourcePane) return null;
        const canvas = event.target.closest?.('.desktop-canvas');
        const editor = event.target.closest?.('.editor-file-drop');
        if (editor && canvas === favoriteDrag.window.closest('.desktop-canvas')
            && !editor.closest('[inert],.hidden,[hidden]')) {
            return {bridge: favoriteDrag.window.querySelector('.files-item-drop-input'),
                mark: editor, copy: true, effect: 'copy', data: {kind: 'text_editor', window: Number(editor.querySelector('.editor-window-id').textContent)}};
        }
        if (canvas && !event.target.closest('.desktop-window,.desktop-dock-zone,.desktop-context-menu,[inert],.hidden,[hidden]')
            && canvas === favoriteDrag.window.closest('.desktop-canvas') && !favoriteDrag.fromFavorites) {
            const shortcut = event.altKey || ((event.ctrlKey || event.metaKey) && event.shiftKey);
            const action = shortcut ? 'shortcut' : (event.ctrlKey || event.metaKey) ? 'copy' : 'move';
            return {bridge:favoriteDrag.window.querySelector('.files-item-drop-input'), mark:canvas,
                copy:action === 'copy', effect:shortcut ? 'link' : action, data:{kind:'desktop', action}};
        }
        const target = event.target.closest?.('.files-window-content');
        if (!target || target.closest('[inert],.hidden,[hidden]')) return null;
        const bridge = target.querySelector('.files-item-drop-input');
        const copy = copiesFiles(event);
        const location = event.target.closest('.files-drop-location');
        const trash = event.target.closest('.files-trash-place');
        const tag = event.target.closest('.files-tag-link');
        if (location) {
            const path = location.querySelector('.files-drop-path').textContent;
            return {bridge, mark:location, copy, data:{kind:'location', path}};
        }
        if (trash && !copy) return {bridge, mark:trash, copy:false, data:{kind:'trash'}};
        const color = tag?.querySelector('.files-drop-color')?.textContent;
        if (color) return {bridge, mark:tag, copy, data:{kind:'tag', color}};
        const pane = event.target.closest('.files-pane,.files-inactive-preview,.files-window-tools');
        const folder = event.target.closest('.files-directory-row');
        const directory = target.querySelector('.files-drop-directory')?.textContent;
        if (!pane || !directory || (!folder && target === favoriteDrag.window && !copy)) return null;
        const path = folder?.getAttribute('aria-label') || null;
        if (path === favoriteDrag.path || path?.startsWith(favoriteDrag.path + '/')) return null;
        return {bridge, mark:folder || pane, copy, data:{kind:'directory', path}};
    }
    function fileGhost(event, row) {
        const ghost = document.createElement('div');
        ghost.className = 'files-drag-ghost';
        const icon = row.querySelector('.file-entry-icon,.action-glyph');
        if (icon) ghost.append(icon.cloneNode(true));
        const label = row.querySelector('.file-label,.files-place span:not(.action-glyph)')?.textContent;
        ghost.append(label || row.getAttribute('aria-label')?.split('/').at(-1) || '');
        document.body.append(ghost);
        event.dataTransfer.setDragImage(ghost, 14, ghost.offsetHeight / 2);
        requestAnimationFrame(() => ghost.remove());
    }
    function sendItemDrop(bridge, copy, target) {
        if (!bridge) return;
        const {epoch,request,windowId,path,sourceDirectory} = favoriteDrag;
        bridge.value = JSON.stringify({epoch,request,source:windowId,path,source_directory:sourceDirectory,copy,target});
        bridge.dispatchEvent(new Event('input',{bubbles:true}));
    }
    function dragPaneReady(pane) {
        return pane?.isConnected && validPane(pane) === pane && pane.classList.contains('file-transfer-ready');
    }
    function dropPane(target) {
        const pane = validPane(target);
        return fileDrag && pane !== fileDrag.pane && dragPaneReady(pane) && dragPaneReady(fileDrag.pane) ? pane : null;
    }
    function filesDownloadTarget(target) {
        const drop = target instanceof Element && target.closest('.files-download-drop');
        return drop && !drop.closest('[inert],.hidden,[hidden]') && drop.getClientRects().length;
    }
    function filesUploadTarget(event) {
        const pane = validPane(event.target);
        return !favoriteDrag && !fileDrag && pane?.classList.contains('files-upload-ready')
            && event.dataTransfer?.types.includes('Files') ? pane : null;
    }
    let staging = false;
    async function stageUpload(pane, files) {
        const panel = pane.closest('.remote-files');
        const bridge = panel.querySelector('.files-upload-input');
        const ack = panel.querySelector('.files-upload-ack');
        const snapshot = JSON.parse(panel.querySelector('.files-favorites-state').textContent);
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
        const row = event.target instanceof Element && event.target.closest('.file-row,.files-favorite-row,.files-inactive-draggable');
        if (!row) return;
        if (row.closest('.remote-files')) {
            const source = favoriteSource(row);
            const pane = validPane(row);
            const downloadPane = row.classList.contains('file-draggable') && dragPaneReady(pane) ? pane : null;
            const inactive = row.closest('.files-inactive-preview');
            const sourcePane = inactive || (row.classList.contains('file-draggable') ? pane : null);
            const path = row.getAttribute('aria-label');
            if ((!source && !sourcePane) || !path || !event.dataTransfer) { event.preventDefault(); return; }
            clearFileDrag();
            const window = row.closest('.files-window-content');
            const panel = row.closest('.remote-files');
            const origin = panel.querySelector('.files-favorites-state') ? panel : document.querySelector('.remote-files:has(.files-favorites-state)');
            if (!origin) { event.preventDefault(); return; }
            const snapshot = JSON.parse(origin.querySelector('.files-favorites-state').textContent);
            const windowId = Number(window.querySelector('.files-window-id').textContent);
            const sourceDirectory = window.querySelector('.files-drop-directory').textContent;
            favoriteDrag = {panel:origin, window, windowId, ...snapshot, sourceDirectory, path, id: ++sequence, fromFavorites: !!source?.fromFavorites, downloadPane, sourcePane};
            if (downloadPane) {
                document.documentElement.dataset.filesDownloading = 'true';
                send(downloadPane, 'drag_start', row, {}, {drag_id: favoriteDrag.id});
            }
            event.dataTransfer.effectAllowed = favoriteDrag.fromFavorites ? 'move' : 'all';
            event.dataTransfer.setData(favoriteDragType, String(favoriteDrag.id));
            fileGhost(event, row);
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
            clearFileTarget();
            const target = favoriteTarget(event);
            const transfer = !target && itemTarget(event);
            const download = favoriteDrag.downloadPane && filesDownloadTarget(event.target);
            event.preventDefault();
            event.dataTransfer.dropEffect = target ? (favoriteDrag.fromFavorites ? 'move' : 'copy')
                : transfer ? (transfer.effect || (transfer.copy || transfer.data.kind === 'tag' ? 'copy' : 'move')) : download ? 'copy' : 'none';
            if (target) {
                favoriteIndicator = target.row || target.zone;
                favoriteIndicator.setAttribute('data-favorite-drop', target.row ? (target.after ? 'after' : 'before') : 'append');
            }
            if (transfer) {
                fileTarget = transfer.mark;
                fileTarget.setAttribute('data-files-item-target', '');
            }
            return;
        }
        const upload = filesUploadTarget(event);
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
        if (fileTarget && (!(event.relatedTarget instanceof Node) || !fileTarget.contains(event.relatedTarget))) clearFileTarget();
        const pane = validPane(event.target);
        if (pane && (!(event.relatedTarget instanceof Node) || !pane.contains(event.relatedTarget))) {
            pane.removeAttribute('data-file-drop-target');
        }
    }, true);
    document.addEventListener('drop', event => {
        if (favoriteDrag) {
            event.preventDefault(); event.stopImmediatePropagation();
            const target = favoriteTarget(event);
            const transfer = !target && itemTarget(event);
            const matches = event.dataTransfer?.getData(favoriteDragType) === String(favoriteDrag.id);
            if (target && matches && favoriteDrag.sourcePane) {
                const bridge = target.zone.closest('.files-window-content')?.querySelector('.files-item-drop-input');
                sendItemDrop(bridge, true, {kind:'favorite', anchor:target.anchor, after:target.after});
            } else if (target && matches) {
                const {panel, path, fromFavorites, epoch, request} = favoriteDrag;
                const bridge = panel.querySelector('.files-favorites-input');
                bridge.value = JSON.stringify({path, from_favorites: fromFavorites, epoch, request, anchor: target.anchor, after: target.after});
                bridge.dispatchEvent(new Event('input', {bubbles: true}));
            } else if (transfer?.bridge && matches) {
                sendItemDrop(transfer.bridge, transfer.copy, transfer.data);
            } else if (matches && favoriteDrag.downloadPane && filesDownloadTarget(event.target)) {
                send(favoriteDrag.downloadPane, 'files_download', null, {}, {drag_id: favoriteDrag.id});
            }
            clearFileDrag();
            return;
        }
        const upload = filesUploadTarget(event);
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
            const draggable = !row.querySelector('.file-inline-name') && ((row.closest('.remote-files') && !!favoriteSource(row))
                || (row.closest('.remote-files') && row.classList.contains('file-draggable') && !!validPane(row))
                || (row.classList.contains('file-draggable') && dragPaneReady(validPane(row))));
            if (row.draggable !== !!draggable) row.draggable = !!draggable;
        }
        for (const row of document.querySelectorAll('.files-favorite-row')) {
            const draggable = !!favoriteSource(row);
            if (row.draggable !== draggable) row.draggable = draggable;
        }
        for (const row of document.querySelectorAll('.files-inactive-draggable')) {
            row.draggable = !!row.closest('.files-window-content')?.querySelector('.files-drop-directory')?.textContent;
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
    function placeFileSubmenu(branch) {
        const menu = branch.closest('.file-context-menu,.file-actions-menu');
        const trigger = branch.querySelector('.file-open-trigger');
        const submenu = branch.querySelector('.file-open-submenu');
        const frame = desktopFrame.boundsFor(menu);
        submenu.style.setProperty('--file-submenu-max-width', `${frame.right - frame.left - 16}px`);
        submenu.style.setProperty('--file-submenu-max-height', `${frame.bottom - frame.top - 16}px`);
        const parent = menu.getBoundingClientRect();
        const anchor = trigger.getBoundingClientRect();
        const {width, height} = submenu.getBoundingClientRect();
        const right = parent.right - 1;
        const left = parent.left - width + 1;
        const x = right + width <= frame.right - 8 ? right : left;
        submenu.style.left = `${Math.max(frame.left + 8, Math.min(x, frame.right - width - 8))}px`;
        submenu.style.top = `${Math.max(frame.top + 8, Math.min(anchor.top - 5, frame.bottom - height - 8))}px`;
    }
    function setFileSubmenu(branch, open, focus = false) {
        const trigger = branch.querySelector('.file-open-trigger');
        if (open && trigger.disabled) return;
        branch.toggleAttribute('data-submenu-open', open);
        trigger.setAttribute('aria-expanded', String(open));
        if (open) {
            placeFileSubmenu(branch);
            if (focus) branch.querySelector('.file-open-submenu button:not(:disabled)')?.focus();
        } else if (focus) trigger.focus();
    }
    document.addEventListener('pointerover', event => {
        const branch = event.target.closest?.('.file-open-branch');
        if (!branch) return;
        clearTimeout(submenuClose);
        const trigger = event.target.closest?.('.file-open-trigger');
        if (trigger && event.pointerType !== 'touch') setFileSubmenu(branch, true);
    }, true);
    document.addEventListener('pointerout', event => {
        const branch = event.target.closest?.('.file-open-branch[data-submenu-open]');
        const next = event.relatedTarget;
        if (branch && (!(next instanceof Node) || !branch.contains(next))
            && !branch.querySelector('.file-open-submenu').contains(document.activeElement)) {
            clearTimeout(submenuClose);
            submenuClose = setTimeout(() => setFileSubmenu(branch, false), 180);
        }
    }, true);
    document.addEventListener('click', event => {
        const trigger = event.target.closest?.('.file-open-trigger');
        if (trigger) setFileSubmenu(trigger.closest('.file-open-branch'), true, true);
    }, true);
    const fileCommands = [
        ['files_copy_paths', 'copy_paths'], ['files_properties', 'properties'],
        ['files_open', 'activate'], ['files_rename', 'rename'], ['files_new_folder', 'new_folder'],
        ['files_trash', 'delete'], ['files_delete_permanently', 'delete_permanently'],
        ['files_refresh', 'refresh'], ['files_copy', 'copy_files'], ['files_paste', 'paste_files'],
        ['files_select_all', 'all'],
    ];
    document.addEventListener('keydown',event=>{
        if(!(event.target instanceof Element)||event.isComposing)return;
        const menu=event.target.closest('.file-context-menu,.file-actions-menu');
        if(menu){
            if (event.target.matches('input[type=range]') && !['Escape','Tab'].includes(event.key)) return;
            const branch = event.target.closest('.file-open-branch');
            const submenu = event.target.closest('.file-open-submenu');
            if (branch && event.key === 'ArrowRight' && !submenu) {
                setFileSubmenu(branch, true, true);
                event.preventDefault(); event.stopImmediatePropagation();
                return;
            }
            if (branch?.hasAttribute('data-submenu-open')
                && (event.key === 'Escape' || (submenu && event.key === 'ArrowLeft'))) {
                setFileSubmenu(branch, false, true);
                event.preventDefault(); event.stopImmediatePropagation();
                return;
            }
            const palette=event.target.closest('.file-color-menu');
            const horizontal=palette&&['ArrowLeft','ArrowRight'].includes(event.key);
            if(horizontal||['ArrowDown','ArrowUp','Home','End'].includes(event.key)){
                const buttons=[...(horizontal ? palette : submenu || menu).querySelectorAll('button:not(:disabled), input[type=range]:not(:disabled)')]
                    .filter(button => (!button.closest('.file-open-submenu') || submenu) && button.getClientRects().length);
                const index=buttons.indexOf(event.target.closest('button,input[type=range]'));
                const back=event.key==='ArrowUp'||event.key==='ArrowLeft';
                const next=event.key==='Home'?0:event.key==='End'?buttons.length-1:(index+(back?-1:1)+buttons.length)%buttons.length;
                if (branch && !submenu && buttons[next] !== event.target) setFileSubmenu(branch, false);
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
        const desktopFiles = !!pane.closest('.desktop-window.app-files');
        const bindings = desktopFiles ? arcnixKeys.state()?.bindings
            : JSON.parse(pane.querySelector('.native-file-shortcuts')?.textContent || '{}');
        const chord = window.arcnixShortcutChord(event);
        if (!bindings || event.getModifierState('AltGraph')) return;
        const row = event.target.closest('.file-row');
        for (const [action, kind] of fileCommands) {
            if (!bindings[action]?.includes(chord)) continue;
            if (kind === 'activate' && !event.target.closest('.file-name')) return;
            if ((kind === 'copy_files' || kind === 'paste_files') && !desktopFiles) return;
            event.preventDefault();
            event.stopImmediatePropagation();
            if (!event.repeat) send(pane, kind, row);
            return;
        }
        // Browsers activate a focused button on Return. Respect a remapped or disabled file Open binding.
        if (chord === 'Enter' && event.target.closest('.file-name')) {
            event.preventDefault();
            event.stopImmediatePropagation();
        }
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
        const trigger = menu.querySelector('.file-open-trigger');
        if (trigger && !trigger.hasAttribute('aria-expanded')) {
            trigger.setAttribute('aria-expanded', 'false');
        }
        const submenu = trigger?.closest('.file-open-branch[data-submenu-open]');
        if (submenu) placeFileSubmenu(submenu);
        if(fileMenu.focus){const first=menu.querySelector('button:not(:disabled)');if(first)first.focus();else menu.focus();fileMenu.focus=false;}
    }
    function scheduleMenu(){if(!menuScheduled){menuScheduled=true;requestAnimationFrame(placeFileMenu);}}
    new MutationObserver(scheduleMenu).observe(document,{subtree:true,childList:true,attributes:true,attributeFilter:['class']});
    document.addEventListener('scroll',event=>{
        if(fileMenu?.opened&&!fileMenu.toolbar&&event.target instanceof Element&&!event.target.closest('.pane-menu')&&fileMenu.pane.contains(event.target))dismissMenu(false);
    },true);
    window.addEventListener('resize',()=>{if(fileMenu?.opened)dismissMenu(false);});
    document.addEventListener('click', event => {
        if (!(event.target instanceof Element)) return;
        const options = document.querySelector('.files-search-options');
        if (options?.getClientRects().length
            && !event.target.closest('.files-search-options,.files-search-options-toggle')) {
            options.closest('.pathbar')?.querySelector('.files-search-options-toggle')?.click();
        }
        if (event.target.closest('.files-search-clear')) {
            event.target.closest('.pathbar')?.querySelector('.files-search-input')?.focus();
        }
    }, true);
    document.addEventListener('keydown', event => {
        if (event.isComposing || !(event.target instanceof Element)) return;
        const files = keyboardFiles(event.target);
        const toggle = files?.querySelector('.files-search-toggle:not(:disabled)');
        if (!toggle?.getClientRects().length || toggle.closest('[inert]')) return;
        const input = files.querySelector('.files-search-input');
        if (files.querySelector('.file-actions-menu,.file-context-menu')) return;
        if (arcnixKeys.matches(event, 'files_refresh', 'F5') && !event.target.closest('input,textarea,select,[contenteditable]')) {
            files.querySelector('.path-refresh:not(:disabled)')?.click();
        } else if (event.key === 'Escape' && input) {
            const options = files.querySelector('.files-search-options');
            const control = options ? files.querySelector('.files-search-options-toggle') : toggle;
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
        if (crumb && pathPane(crumb)) {
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
        const pane = validPane(row);
        if (!pane || event.target.closest(".file-expander,.file-inline-name")) return;
        const name = row.querySelector(".file-name");
        if (!name || name.disabled) return;
        event.preventDefault(); event.stopImmediatePropagation();
        name.focus({preventScroll: true});
        if (event.detail === 1) send(pane, "select", row, event);
        else if (event.detail === 2 && !additive(event) && !event.shiftKey) send(pane, "activate", row);
    }, true);
    document.addEventListener('dblclick', event => {
        if (event.button !== 0 || !(event.target instanceof Element)) return;
        const picker = event.target.closest('.file-picker-row');
        if (picker && !picker.closest('[inert]')) {
            picker.querySelector('.file-picker-open-entry')?.click();
            return;
        }
        if (!event.target.closest('.breadcrumbs')) return;
        const pane = pathPane(event.target);
        if (!pane) return;
        clearTimeout(breadcrumbClick);
        lastPane = pane;
        send(pane, 'edit_path', null);
        event.preventDefault(); event.stopImmediatePropagation();
    }, true);
    const fileNavigationKeys = [
        ['files_select', 'Space'], ['files_left', 'ArrowLeft'], ['files_right', 'ArrowRight'],
        ['files_up', 'ArrowUp'], ['files_down', 'ArrowDown'], ['files_first', 'Home'], ['files_last', 'End'],
    ];
    function editPathShortcut(event, pane) {
        return pane?.closest('.desktop-window.app-files')
            ? arcnixKeys.matches(event, 'files_edit_path', 'Ctrl+L')
            : additive(event) && event.key.toLowerCase() === 'l';
    }
    document.addEventListener("keydown", event => {
        if (!(event.target instanceof Element) || event.isComposing) return;
        if (event.target.closest(".terminal-module")) return;
        const lastWindow = lastPane?.closest('.desktop-window');
        const pane = pathPane(event.target) || validPane(keyboardFiles(event.target)?.querySelector('.files-pane')) || (editPathShortcut(event, lastPane)
            && lastPane?.isConnected && (!lastWindow || lastWindow.classList.contains('active')) ? lastPane : null);
        if (!validPane(pane)) return;
        if (editPathShortcut(event, pane)) {
            event.preventDefault(); event.stopImmediatePropagation();
            const pathScope = pane.closest('.desktop-window.app-files') || pane;
            const field = pathScope.querySelector(".path-input:not(.hidden)");
            if (field) { field.focus(); field.select(); }
            else send(pane, 'edit_path', null);
            return;
        }
        if (event.target.closest("input,textarea,select,[contenteditable]")) return;
        const row = event.target.closest(".file-row");
        if (!row) return;
        const files = !!pane.closest('.desktop-window.app-files');
        const navigationBinding = fileNavigationKeys.find(([action, key]) => {
            const binding = files ? arcnixKeys.binding(action, key) : [key];
            // Preserve Shift/Ctrl selection modifiers on the standard navigation keys.
            return window.arcnixChordMatches(event, binding)
                || (binding.includes(key) && !event.altKey && (event.code || event.key) === key);
        });
        const navigation = navigationBinding?.[1];
        const customNavigation = files && navigationBinding
            && !(arcnixKeys.binding(navigationBinding[0], navigation).includes(navigation)
                && (event.code || event.key) === navigation && !event.altKey);
        const selectionEvent = customNavigation ? {} : event;
        let handled = true;
        if (navigation === "Space") send(pane, "select", row, selectionEvent);
        else if (pane.classList.contains("icons-view") && ["ArrowLeft", "ArrowRight"].includes(navigation)) {
            const rows = [...pane.querySelectorAll(".file-row")];
            const target = rows[Math.max(0, Math.min(rows.length - 1, rows.indexOf(row) + (navigation === "ArrowLeft" ? -1 : 1)))];
            if (target) { target.querySelector(".file-name").focus(); if (!additive(selectionEvent) || selectionEvent.shiftKey) send(pane, "select", target, selectionEvent); }
        }
        else if (navigation === "ArrowRight") send(pane, "expand", row);
        else if (navigation === "ArrowLeft") {
            if (row.querySelector(".file-expander")?.getAttribute("aria-expanded") === "true") send(pane, "collapse", row);
            else {
                const rows = [...pane.querySelectorAll(".file-row")];
                const depth = parseFloat(getComputedStyle(row).paddingLeft);
                const parent = rows.slice(0, rows.indexOf(row)).reverse().find(item => parseFloat(getComputedStyle(item).paddingLeft) < depth);
                if (parent) { parent.querySelector(".file-name")?.focus(); send(pane, "select", parent); }
            }
        } else if (["ArrowUp", "ArrowDown", "Home", "End"].includes(navigation)) {
            const rows = [...pane.querySelectorAll(".file-row")].filter(item => !item.querySelector(".file-name").disabled);
            const index = rows.indexOf(row);
            const columns = pane.classList.contains("icons-view") ? Math.max(1, getComputedStyle(pane.querySelector(".file-list")).gridTemplateColumns.split(/\s+/).length) : 1;
            const next = navigation === "Home" ? 0 : navigation === "End" ? rows.length - 1 : index + (navigation === "ArrowUp" ? -columns : columns);
            const target = rows[Math.max(0, Math.min(rows.length - 1, next))];
            if (target) { target.querySelector(".file-name").focus(); if (!additive(selectionEvent) || selectionEvent.shiftKey) send(pane, "select", target, selectionEvent); }
        } else handled = false;
        if (handled) { event.preventDefault(); event.stopImmediatePropagation(); }
    }, true);
    const focused = new WeakSet();
    const currentPaths = new WeakMap();
    new MutationObserver(() => {
        for (const field of document.querySelectorAll(".path-input,.files-search-input,.file-inline-name")) {
            if (field.classList.contains("hidden")) focused.delete(field);
            else if (!focused.has(field) && field.getClientRects().length) {
                focused.add(field);
                field.focus();
                const dot = field.classList.contains("file-inline-stem") ? field.value.lastIndexOf(".") : -1;
                const end = dot > 0 && dot < field.value.length - 1 ? dot : field.value.length;
                field.setSelectionRange(0, end);
            }
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
