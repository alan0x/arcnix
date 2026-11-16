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
    let fileMenu = null;
    let menuScheduled = false;
    function validPane(target) {
        const pane=target instanceof Element ? target.closest('.file-pane') : null;
        return pane && !pane.closest('[inert],[hidden]') ? pane : null;
    }
    const fileDragType = 'application/x-arcnix-file-transfer';
    let fileDrag = null;
    function clearDropTarget() {
        document.querySelectorAll('[data-file-drop-target]').forEach(pane => pane.removeAttribute('data-file-drop-target'));
    }
    function clearFileDrag() {
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
    document.addEventListener('dragstart', event => {
        const row = event.target instanceof Element && event.target.closest('.file-row');
        if (!row) return;
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
        if (!fileDrag) return;
        clearDropTarget();
        const pane = dropPane(event.target);
        event.preventDefault();
        event.dataTransfer.dropEffect = pane ? 'copy' : 'none';
        if (pane) pane.setAttribute('data-file-drop-target', 'true');
    }, true);
    document.addEventListener('dragleave', event => {
        const pane = validPane(event.target);
        if (pane && (!(event.relatedTarget instanceof Node) || !pane.contains(event.relatedTarget))) {
            pane.removeAttribute('data-file-drop-target');
        }
    }, true);
    document.addEventListener('drop', event => {
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
            const draggable = row.classList.contains('file-draggable') && dragPaneReady(validPane(row));
            if (row.draggable !== !!draggable) row.draggable = !!draggable;
        }
    }
    new MutationObserver(updateDraggableRows).observe(document, {subtree:true, childList:true, attributes:true, attributeFilter:['class','inert','hidden']});
    function context(target,event,keyboard=false) {
        const pane=validPane(target);
        const listing=target instanceof Element && target.closest('.file-list-container,.file-empty');
        if(!pane||!listing)return false;
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
        const pane=validPane(button);
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
        const box=menu.getBoundingClientRect();
        let x=fileMenu.x-(fileMenu.toolbar?box.width:0), y=fileMenu.y;
        if(fileMenu.toolbar&&y+box.height>innerHeight-8)y=fileMenu.origin.getBoundingClientRect().top-box.height;
        menu.style.left=`${Math.max(8,Math.min(x,innerWidth-box.width-8))}px`;
        menu.style.top=`${Math.max(8,Math.min(y,innerHeight-box.height-8))}px`;
        if(fileMenu.focus){const first=menu.querySelector('button:not(:disabled)');if(first)first.focus();else menu.focus();fileMenu.focus=false;}
    }
    function scheduleMenu(){if(!menuScheduled){menuScheduled=true;requestAnimationFrame(placeFileMenu);}}
    new MutationObserver(scheduleMenu).observe(document,{subtree:true,childList:true,attributes:true,attributeFilter:['class']});
    document.addEventListener('scroll',event=>{
        if(fileMenu?.opened&&event.target instanceof Element&&!event.target.closest('.pane-menu')&&fileMenu.pane.contains(event.target))dismissMenu(false);
    },true);
    window.addEventListener('resize',()=>{if(fileMenu?.opened)dismissMenu(false);});
    document.addEventListener("click", event => {
        if (!(event.target instanceof Element) || event.detail === 0) return;
        lastPane = event.target.closest(".file-pane") || lastPane;
        const row = event.target.closest(".file-list .file-row");
        if (!row || event.target.closest(".file-expander")) return;
        const name = row.querySelector(".file-name");
        if (!name || name.disabled) return;
        event.preventDefault(); event.stopImmediatePropagation();
        name.focus({preventScroll: true});
        if (event.detail === 1) send(row.closest(".file-pane"), "select", row, event);
        else if (event.detail === 2 && !additive(event) && !event.shiftKey) send(row.closest(".file-pane"), "activate", row);
    }, true);
    document.addEventListener("keydown", event => {
        if (!(event.target instanceof Element) || event.isComposing) return;
        if (event.target.closest(".terminal-module")) return;
        const pane = event.target.closest(".file-pane") || (additive(event) && event.key.toLowerCase() === "l" && lastPane?.isConnected ? lastPane : null);
        if (!pane || pane.closest("[inert],[hidden]")) return;
        if (additive(event) && event.key.toLowerCase() === "l") {
            event.preventDefault(); event.stopImmediatePropagation();
            const field = pane.querySelector(".path-input:not(.hidden)");
            if (field) { field.focus(); field.select(); }
            else pane.querySelector(".path-edit")?.click();
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
        for (const field of document.querySelectorAll(".path-input")) {
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
