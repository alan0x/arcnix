// Desktop presentation only. Rust still owns app state and session lifetimes.
// View transitions use browser snapshots; no app DOM, terminal or file is copied.
(() => {
    const reduced = matchMedia('(prefers-reduced-motion: reduce)');
    const forwarded = new WeakSet(), animations = new Set();
    let transition = null, stateNode = null, previous = null, pendingDeliver = null;
    const stateObserver = new MutationObserver(changed);
    function state() {
        try { return JSON.parse(stateNode?.textContent || 'null'); }
        catch (_) { return null; }
    }
    function signature(value) {
        return JSON.stringify(value && [value.shown,value.epoch,value.app,value.fullscreen,
            value.windows.map(window=>[window.class,window.maximized])]);
    }
    function windowFor(app) {
        return app && document.querySelector(`.desktop-window.${app}:not(.hidden)`);
    }
    function appClass(element) {
        return element && ([ ...element.classList ].find(name=>name.startsWith('files-window-')) || [...element.classList].find(name=>name.startsWith('app-')));
    }
    function animate(element,frames,duration) {
        if(!element || reduced.matches || !element.animate)return;
        const animation=element.animate(frames,{duration,easing:'cubic-bezier(.2,.75,.25,1)'});
        animations.add(animation);
        animation.finished.catch(()=>{}).then(()=>animations.delete(animation));
    }
    function changed() {
        const current=state(),old=previous;
        previous=current;
        desktopGeometry.sync(current);
        if(!current || !old || signature(current)===signature(old))return;
        if(!current.shown || current.epoch!==old.epoch){
            transition?.skipTransition();
            for(const animation of animations)animation.cancel();
            return;
        }
        // Links inside apps and hosts without view transitions use the live entrance.
        if(transition || reduced.matches || !current.app
            || old.windows.some(window=>window.class===current.app))return;
        const window=windowFor(current.app);
        const switching=Boolean(old.app);
        animate(switching ? window?.querySelector('.desktop-window-content') : window,
            [{opacity:switching ? .35 : 0,transform:switching ? 'translateX(18px)' : 'translateY(30px) scale(.91)'},
             {opacity:1,transform:'none'}],switching ? 200 : 240);
    }
    function initialize() {
        const node=document.querySelector('.desktop-motion-state');
        if(node===stateNode)return;
        transition?.skipTransition();
        stateObserver.disconnect();
        stateNode=node;
        previous=state();
        desktopGeometry.sync(previous);
        if(node)stateObserver.observe(node,{subtree:true,childList:true,characterData:true});
    }
    // Rendering/localization can replace the bridge. This observer only finds
    // that node; its dedicated observer handles the tiny presentation state.
    new MutationObserver(initialize).observe(document,{subtree:true,childList:true});
    initialize();
    function forward(button) {
        if(!button.isConnected || button.disabled)return;
        forwarded.add(button);
        try { button.click(); } finally { forwarded.delete(button); }
    }
    function kind(button,before) {
        if(button.matches('.desktop-shortcut,.desktop-dock-app')){
            const app=appClass(button);
            return app && !before.windows.some(window=>window.class===app)
                ? { motion:before.app?'switch':'open', app } : null;
        }
        const app=appClass(button.closest('.desktop-window')) || before.app;
        if(!app)return null;
        if(button.matches('.window-close'))return {motion:'close',app};
        if(button.matches('.window-minimize,.desktop-dock-home'))return {motion:'minimize',app};
        if(button.matches('.window-maximize'))return {motion:'resize',app};
        return null;
    }
    function windowOffset(app,anchor) {
        const window=windowFor(app)?.getBoundingClientRect();
        if(!anchor || !window?.width)return;
        const shift=(from,to,limit)=>`${Math.max(-limit,Math.min(limit,(from-to)*.22))}px`;
        const root=document.documentElement;
        root.style.setProperty('--desktop-motion-x',shift(anchor.x+anchor.width/2,window.x+window.width/2,84));
        root.style.setProperty('--desktop-motion-y',shift(anchor.y+anchor.height/2,window.y+window.height/2,76));
    }
    function settle(before,after) {
        return new Promise(resolve=>{
            const node=stateNode;
            let finished=false,timer;
            const observer=new MutationObserver(check);
            function finish(skip) {
                if(finished)return;
                finished=true;observer.disconnect();clearTimeout(timer);
                // Rendering frames are suspended during a view update. Resolve
                // after the committed DOM mutation, never await an animation frame.
                if(skip)transition?.skipTransition();
                else after?.();
                resolve();
            }
            function check() {
                const current=state();
                if(!current || current.epoch!==before.epoch || !current.shown)finish(true);
                else if(signature(current)!==signature(before))finish(false);
            }
            if(node)observer.observe(node,{subtree:true,childList:true,characterData:true});
            // Never wait for SSH, an app's data or a worker response. The native
            // click has already been delivered; a slow render only skips motion.
            timer=setTimeout(()=>finish(true),220);
            check();
        });
    }
    function start(button,{motion,app},before) {
        let delivered=false,active;
        const root=document.documentElement;
        const named=new Set();
        const mark=()=>{
            const element=windowFor(app);
            if(element){
                element.style.viewTransitionName='arcnix-desktop-window';
                named.add(element);
            }
        };
        const deliver=()=>{if(!delivered){delivered=true;if(pendingDeliver===deliver)pendingDeliver=null;forward(button);}};
        const launchRect=button.querySelector('.desktop-app-icon')?.getBoundingClientRect();
        if(motion==='minimize') {
            const dock=document.querySelector(`.desktop-dock-app.${app} .desktop-app-icon`);
            windowOffset(app,dock?.getBoundingClientRect());
        }
        const clear=()=>{
            for(const element of named)element.style.removeProperty('view-transition-name');
            root.classList.remove('desktop-motion',`desktop-motion-${motion}`);
            root.style.removeProperty('--desktop-motion-x');
            root.style.removeProperty('--desktop-motion-y');
        };
        pendingDeliver=deliver;
        root.classList.add('desktop-motion',`desktop-motion-${motion}`);
        mark();
        try {
            active=document.startViewTransition(async()=>{
                deliver();
                await settle(before,()=>{
                    mark();
                    if(launchRect)windowOffset(app,launchRect);
                });
            });
            transition=active;
            active.ready.catch(()=>{});
            active.updateCallbackDone.catch(deliver);
            active.finished.catch(()=>{}).then(()=>{
                if(transition!==active)return;
                transition=null;
                clear();
            });
        } catch (_) {
            clear();
            transition=null;deliver();
        }
    }
    document.addEventListener('click',event=>{
        if(!(event.target instanceof Element))return;
        if(forwarded.has(event.target.closest('button')))return;
        // A second input can arrive before the browser takes its first snapshot.
        // Commit the earlier intent first, even when the new click leaves Desktop.
        if(pendingDeliver){transition?.skipTransition();pendingDeliver();}
        if(event.button!==0
            || event.altKey || event.ctrlKey || event.metaKey || event.shiftKey)return;
        const button=event.target.closest('.desktop-shortcut,.desktop-dock-app,.desktop-dock-home,.window-minimize,.window-close,.window-maximize');
        if(!button || button.disabled || forwarded.has(button))return;
        const before=state();
        if(!before?.shown || document.visibilityState!=='visible')return;
        const motion=kind(button,before);
        if(!motion || reduced.matches)return;
        if(button.matches('.desktop-shortcut,.desktop-dock-app')){
            animate(button.querySelector('.desktop-app-icon'),
                [{transform:'scale(.88)'},{transform:'translateY(-6px) scale(1.08)',offset:.55},{transform:'none'}],245);
        }
        if(typeof document.startViewTransition!=='function'
            || !CSS.supports('view-transition-name','arcnix-desktop-window'))return;
        event.preventDefault();event.stopImmediatePropagation();
        if(transition){
            // Fast clicks stay in native order. Skip the previous visual, then
            // forward this intent exactly once after its update has committed.
            const previous=transition;
            previous.skipTransition();
            previous.updateCallbackDone.catch(()=>{}).then(()=>forward(button));
        } else start(button,motion,before);
    },true);
    document.addEventListener('dblclick', event => {
        const target = event.target;
        if (event.button !== 0 || !(target instanceof Element) || desktopGeometry.movedRecently()) return;
        if (target.closest('button, a, input, textarea, select, [contenteditable], [role="button"], [role="tab"], .path-location, .files-search-options')) return;
        const titlebar = target.closest('.desktop-window-titlebar');
        if (!titlebar?.closest('.desktop-window:not(.hidden)') || !state()?.shown) return;
        const maximize = titlebar.querySelector('.window-maximize:not(:disabled)');
        if (!maximize) return;
        event.preventDefault();
        maximize.click();
    });
    reduced.addEventListener('change',()=>{
        if(!reduced.matches)return;
        transition?.skipTransition();
        for(const animation of animations)animation.cancel();
    });
})();
